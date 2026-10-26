"""Passive, bounded ICC declaration analysis. Run only in the worker sandbox.
PDF object graph != executed color use; DSC comments != PostScript execution.
"""
import base64
import hashlib
import io
import json
import logging
import re
import sys
import zlib

MAX_SOURCE = 32 * 1024 * 1024
MAX_ICC = 4 * 1024 * 1024
MAX_TOTAL = 16 * 1024 * 1024
MAX_PROFILES = 64

class Analysis:
    def __init__(self):
        self.profiles = []
        self.warnings = set()
        self.total = 0

    def warn(self, code):
        self.warnings.add(code)

    def add(self, kind, data=None, expected=None, error=None):
        if len(self.profiles) >= MAX_PROFILES:
            self.warn("PROFILE_COUNT_LIMIT")
            return
        record = {"kind": kind, "state": error or "INVALID_ICC"}
        if data is not None:
            self.total += len(data)
            if len(data) > MAX_ICC or self.total > MAX_TOTAL:
                record["state"] = "PROFILE_BYTE_LIMIT"
            else:
                record.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data))
                if len(data) >= 132 and int.from_bytes(data[:4], "big") == len(data) and data[36:40] == b"acsp":
                    count = int.from_bytes(data[128:132], "big")
                    valid = count <= 1024 and 132 + 12 * count <= len(data)
                    if valid:
                        for offset in range(132, 132 + 12 * count, 12):
                            start = int.from_bytes(data[offset+4:offset+8], "big")
                            size = int.from_bytes(data[offset+8:offset+12], "big")
                            if start < 132 + 12 * count or start + size > len(data):
                                valid = False
                    if valid:
                        valid = data[20:24] in ((b"RGB ", b"CMYK", b"GRAY", b"XYZ ", b"Lab ") if data[12:16] == b"link" else (b"XYZ ", b"Lab "))
                    if valid:
                        spaces = {b"RGB ": ("rgb", 3), b"CMYK": ("cmyk", 4), b"GRAY": ("gray", 1), b"Lab ": ("lab", 3), b"XYZ ": ("xyz", 3)}
                        color, channels = spaces.get(data[16:20], ("unsupported", None))
                        record["colorSpace"] = color
                        record["state"] = "VALIDATED_ENVELOPE"
                        if int.from_bytes(data[64:68], "big") > 3:
                            record["state"] = "INVALID_ICC"
                        elif color == "unsupported":
                            record["state"] = "UNSUPPORTED_COLOR_SPACE"
                        elif data[12:16] == b"link":
                            record["state"] = "DEVICE_LINK"
                        elif expected is not None and expected != channels:
                            record["state"] = "COMPONENT_COUNT_MISMATCH"
        if record["state"] != "VALIDATED_ENVELOPE":
            self.warn(record["state"])
        self.profiles.append(record)

    def pdf(self, data):
        import pypdf
        from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, StreamObject
        if pypdf.__version__ != "6.21.2":
            raise ValueError("PARSER_VERSION_MISMATCH")
        class Warnings(logging.Handler):
            def emit(_, record):
                self.warn("PDF_PARSER_WARNING")
        logger = logging.getLogger("pypdf")
        handler = Warnings()
        logger.addHandler(handler)
        try:
            reader = pypdf.PdfReader(io.BytesIO(data), strict=True)
            if reader.is_encrypted:
                raise ValueError("ENCRYPTED_PDF_UNSUPPORTED")
            def resolve(obj):
                return obj.get_object() if isinstance(obj, IndirectObject) else obj
            seen_profiles = set()
            def profile(obj, kind, require_n):
                try:
                    stream = resolve(obj)
                    key = (kind, id(stream))
                    if key in seen_profiles:
                        return
                    seen_profiles.add(key)
                    if len(self.profiles) >= MAX_PROFILES or self.total >= MAX_TOTAL:
                        self.warn("PROFILE_BUDGET_LIMIT")
                        return
                    if not isinstance(stream, StreamObject):
                        raise ValueError("INVALID_PROFILE_STREAM")
                    if any(k in stream for k in ("/F", "/FFilter", "/FDecodeParms")):
                        raise ValueError("EXTERNAL_PROFILE_REFERENCE")
                    expected = resolve(stream.get("/N"))
                    if (require_n or expected is not None) and (type(expected) is bool or expected not in (1, 3, 4)):
                        raise ValueError("INVALID_COMPONENT_COUNT")
                    filters = resolve(stream.get("/Filter"))
                    if isinstance(filters, ArrayObject) and len(filters) == 1:
                        filters = resolve(filters[0])
                    params = resolve(stream.get("/DecodeParms"))
                    if params is not None:
                        raise ValueError("UNSUPPORTED_DECODE_PARAMETERS")
                    raw = stream._data  # Deliberately pinned pypdf API; never unbounded get_data().
                    if filters is None:
                        decoded = raw
                    elif filters == "/FlateDecode":
                        decoder = zlib.decompressobj()
                        decoded = decoder.decompress(raw, MAX_ICC + 1)
                        if len(decoded) > MAX_ICC:
                            raise ValueError("PROFILE_BYTE_LIMIT")
                        if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
                            raise ValueError("INVALID_FLATE_STREAM")
                    else:
                        raise ValueError("UNSUPPORTED_PROFILE_FILTER")
                    self.add(kind, decoded, expected)
                except Exception as error:
                    code = str(error) if isinstance(error, ValueError) and re.fullmatch(r"[A-Z_]+", str(error)) else "PROFILE_STREAM_FAILED"
                    self.add(kind, error=code)
            root = resolve(reader.trailer["/Root"])
            intents = resolve(root.get("/OutputIntents", []))
            if not isinstance(intents, (ArrayObject, list)):
                raise ValueError("INVALID_OUTPUT_INTENTS")
            for intent in intents[:MAX_PROFILES+1]:
                intent = resolve(intent)
                if not isinstance(intent, DictionaryObject):
                    self.warn("INVALID_OUTPUT_INTENT")
                elif "/DestOutputProfile" in intent:
                    profile(intent["/DestOutputProfile"], "PDF_OUTPUT_INTENT", False)
                else:
                    self.warn("OUTPUT_INTENT_WITHOUT_EMBEDDED_PROFILE")
            if len(intents) > MAX_PROFILES:
                self.warn("PROFILE_COUNT_LIMIT")
            seen_refs, seen_objects = set(), set()
            visits = 0
            def walk(obj, depth=0):
                nonlocal visits
                visits += 1
                if visits > 4096 or depth > 32:
                    self.warn("OBJECT_GRAPH_LIMIT")
                    return
                if isinstance(obj, IndirectObject):
                    key = (obj.idnum, obj.generation)
                    if key in seen_refs:
                        return
                    seen_refs.add(key)
                    obj = resolve(obj)
                if not isinstance(obj, (DictionaryObject, ArrayObject)):
                    return
                if id(obj) in seen_objects:
                    return
                seen_objects.add(id(obj))
                if isinstance(obj, ArrayObject):
                    if len(obj) >= 2 and obj[0] == "/ICCBased":
                        profile(obj[1], "PDF_ICC_BASED", True)
                    children = obj
                else:
                    children = list(obj.values())
                for child in children:
                    if visits >= 4096:
                        self.warn("OBJECT_GRAPH_LIMIT")
                        break
                    walk(child, depth+1)
            walk(root)
        finally:
            logger.removeHandler(handler)

    def postscript(self, data):
        source = io.BytesIO(data)
        while line := source.readline():
            # Counted opaque blocks are skipped, never interpreted or searched for declarations.
            if line.startswith((b"%%BeginBinary:", b"%%BeginData:")):
                match = re.match(rb"%%Begin(?:Binary|Data):\s*(\d+)(?:\s+(?:Binary|ASCII|Hex))?(?:\s+(Bytes|Lines))?\s*$", line)
                self.warn("OPAQUE_POSTSCRIPT_DATA")
                if not match:
                    break
                count = int(match[1])
                if count > MAX_SOURCE:
                    self.warn("DSC_DATA_LIMIT")
                    break
                if match[2] == b"Lines":
                    for _ in range(count):
                        if not source.readline():
                            self.warn("TRUNCATED_DSC_DATA")
                            break
                elif len(source.read(count)) != count:
                    self.warn("TRUNCATED_DSC_DATA")
                continue
            if not line.startswith(b"%%BeginICCProfile:"):
                continue
            match = re.search(rb"\s(-?\d+)(?:\s+(Hex|ASCII))?(?:\s+(Bytes|Lines))?\s*$", line)
            if not match or int(match[1]) < -1:
                self.add("PS_DSC_ICC", error="INVALID_DSC_HEADER")
                break
            count = int(match[1]); encoding = match[2] or b"ASCII"; units = match[3] or b"Bytes"
            physical = bytearray(); lines = 0; ended = False
            while row := source.readline():
                if row.rstrip(b"\r\n") == b"%%EndICCProfile":
                    ended = True
                    break
                physical.extend(row); lines += 1
                if len(physical) > 8 * 1024 * 1024:
                    self.add("PS_DSC_ICC", error="PROFILE_BYTE_LIMIT")
                    return
            if not ended or (count != -1 and count != (lines if units == b"Lines" else len(physical))):
                self.add("PS_DSC_ICC", error="DSC_COUNT_OR_END_MISMATCH")
                continue
            rows = physical.splitlines()
            if any(not row.startswith(b"% ") for row in rows):
                self.add("PS_DSC_ICC", error="INVALID_DSC_DATA")
                continue
            encoded = b"".join(row[2:].strip() for row in rows)
            try:
                if encoding == b"Hex":
                    decoded = bytes.fromhex(encoded.decode("ascii"))
                else:
                    decoded = base64.a85decode(encoded if encoded.startswith(b"<~") else encoded.removesuffix(b"~>"), adobe=encoded.startswith(b"<~"))
                self.add("PS_DSC_ICC", decoded)
            except (ValueError, UnicodeError):
                self.add("PS_DSC_ICC", error="INVALID_DSC_ENCODING")
            if len(self.profiles) >= MAX_PROFILES or self.total >= MAX_TOTAL:
                self.warn("PROFILE_BUDGET_LIMIT")
                break

def analyze(data):
    if not 0 < len(data) <= MAX_SOURCE:
        raise ValueError("SOURCE_BYTE_LIMIT")
    report = Analysis()
    if data.startswith(b"%PDF-"):
        kind, scope = "pdf", "reachable-catalog-object-graph-not-executed-content"
        try:
            report.pdf(data)
        except Exception as error:
            report.warn(str(error) if str(error) in ("ENCRYPTED_PDF_UNSUPPORTED", "PARSER_VERSION_MISMATCH") else "PDF_PARSE_FAILED")
    elif data.startswith(b"%!PS-Adobe-"):
        kind, scope = "postscript", "static-dsc-declarations-not-execution"
        report.postscript(data)
    else:
        raise ValueError("UNSUPPORTED_SOURCE_FORMAT")
    return {"schemaVersion": 1, "profile": "org.jrip.source-color-profiles-v1", "inspectorRevision": "jrip-source-icc-v1-pypdf-6.21.2",
            "sourceSha256": hashlib.sha256(data).hexdigest(), "sourceBytes": len(data), "format": kind, "scope": scope,
            "status": "INCOMPLETE" if report.warnings else "DECLARATIONS_INSPECTED", "profiles": report.profiles,
            "warnings": sorted(report.warnings), "automaticSelection": False, "outputIntentIsInputProfile": False,
            "validationMeaning": "bounded-icc-envelope-not-colorimetric-validation",
            "messages": {"en": "Review declared ICC roles and warnings; no input profile was selected",
                         "ja": "宣言されたICCの役割と警告を確認してください。入力profileは選択していません",
                         "zh-CN": "请检查声明的ICC用途与警告；尚未选择输入profile"}}

if __name__ == "__main__":
    with open(sys.argv[1] if len(sys.argv) == 2 else "/input/document", "rb") as source:
        result = analyze(source.read(MAX_SOURCE + 1))
    encoded = json.dumps(result, separators=(",", ":"))
    if len(encoded.encode()) > 65536:
        raise ValueError("ANALYSIS_OUTPUT_LIMIT")
    print(encoded)
