import base64
import hashlib
import io
import unittest
import tempfile
from pathlib import Path
import zlib
from unittest.mock import patch
from pypdf import PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, NameObject, NumberObject, DecodedStreamObject, EncodedStreamObject
from analyze import analyze, MAX_ICC


def icc(color=b"RGB ", size=132):
    value=bytearray(size)
    value[:4]=size.to_bytes(4,"big");value[12:16]=b"mntr";value[16:20]=color;value[20:24]=b"XYZ ";value[36:40]=b"acsp"
    return bytes(value)


def pdf(profile=None, output=None, compressed=False, n=3, external=False, bad_filter=False, cycle=False):
    writer=PdfWriter();page=writer.add_blank_page(width=72,height=72)
    if profile is not None:
        stream=EncodedStreamObject() if compressed or bad_filter else DecodedStreamObject()
        stream._data=zlib.compress(profile) if compressed else profile
        stream[NameObject("/N")]=NumberObject(n)
        if compressed or bad_filter:
            stream[NameObject("/Filter")]=NameObject("/LZWDecode" if bad_filter else "/FlateDecode")
        if external: stream[NameObject("/F")]=NameObject("/must-not-open")
        ref=writer._add_object(stream)
        resource=DictionaryObject({NameObject("/CS1"):ArrayObject([NameObject("/ICCBased"),ref])})
        page[NameObject("/Resources")]=DictionaryObject({NameObject("/ColorSpace"):resource})
        if cycle: resource[NameObject("/Cycle")]=writer._add_object(resource)
    if output is not None:
        stream=DecodedStreamObject();stream.set_data(output)
        intent=DictionaryObject({NameObject("/Type"):NameObject("/OutputIntent"),NameObject("/DestOutputProfile"):writer._add_object(stream)})
        writer._root_object[NameObject("/OutputIntents")]=ArrayObject([writer._add_object(intent)])
    out=io.BytesIO();writer.write(out);return out.getvalue()


def ps(profile, encoding="Hex", units="Lines", count=None):
    encoded=profile.hex().encode() if encoding=="Hex" else base64.a85encode(profile,adobe=True)
    rows=[b"% "+encoded[i:i+60]+b"\n" for i in range(0,len(encoded),60)]
    number=len(rows) if units=="Lines" else sum(map(len,rows))
    if count is not None: number=count
    return b"%!PS-Adobe-3.0 EPSF-3.0\n%%BeginICCProfile: (test) "+str(number).encode()+b" "+encoding.encode()+b" "+units.encode()+b"\n"+b"".join(rows)+b"%%EndICCProfile\nshowpage\n"

class Profiles(unittest.TestCase):
    def test_pdf_iccbased_output_intent_are_distinct_and_cycles_terminate(self):
        source=pdf(icc(),icc(b"CMYK"),compressed=True,cycle=True)
        result=analyze(source)
        self.assertEqual(result["status"],"DECLARATIONS_INSPECTED")
        self.assertEqual(result["sourceSha256"],hashlib.sha256(source).hexdigest())
        entries={p["kind"]:p for p in result["profiles"]}
        self.assertEqual(entries["PDF_ICC_BASED"]["sha256"],hashlib.sha256(icc()).hexdigest())
        self.assertEqual(entries["PDF_OUTPUT_INTENT"]["colorSpace"],"cmyk")
        self.assertFalse(result["automaticSelection"]);self.assertFalse(result["outputIntentIsInputProfile"])

    def test_pdf_absence_is_scope_limited(self):
        report=analyze(pdf())
        self.assertEqual(report["profiles"],[])
        self.assertIn("not-executed-content",report["scope"])

    def test_pdf_rejects_external_unsupported_bad_profile_and_component_mismatch(self):
        for source,state in [(pdf(icc(),external=True),"EXTERNAL_PROFILE_REFERENCE"),(pdf(icc(),bad_filter=True),"UNSUPPORTED_PROFILE_FILTER"),(pdf(b"invalid"),"INVALID_ICC"),(pdf(icc(),n=4),"COMPONENT_COUNT_MISMATCH")]:
            with self.subTest(state=state):
                report=analyze(source);self.assertEqual(report["status"],"INCOMPLETE")
                self.assertEqual(report["profiles"][0]["state"],state)

    def test_pdf_compressed_bomb_is_bounded(self):
        report=analyze(pdf(b"A"*(MAX_ICC+1),compressed=True))
        self.assertEqual(report["profiles"][0]["state"],"PROFILE_BYTE_LIMIT")

    def test_malformed_encrypted_pdf_never_claims_absence(self):
        report=analyze(b"%PDF-1.4\nbroken")
        self.assertEqual(report["status"],"INCOMPLETE")
        writer=PdfWriter();writer.add_blank_page(72,72);writer.encrypt("secret")
        out=io.BytesIO();writer.write(out)
        self.assertIn("ENCRYPTED_PDF_UNSUPPORTED",analyze(out.getvalue())["warnings"])

    def test_ps_hex_ascii_lines_bytes_unknown_count(self):
        for encoding,units,count in [("Hex","Lines",None),("Hex","Bytes",None),("ASCII","Lines",None),("Hex","Lines",-1)]:
            with self.subTest(encoding=encoding,units=units):
                report=analyze(ps(icc(),encoding,units,count))
                self.assertEqual(report["status"],"DECLARATIONS_INSPECTED")
                self.assertEqual(report["profiles"][0]["sha256"],hashlib.sha256(icc()).hexdigest())
                self.assertIn("not-execution",report["scope"])

    def test_ps_bad_count_end_encoding_and_profile(self):
        for data,state in [(ps(icc(),count=100),"DSC_COUNT_OR_END_MISMATCH"),(ps(icc()).replace(b"%%EndICCProfile",b"%%gone"),"DSC_COUNT_OR_END_MISMATCH"),(ps(b"bad"),"INVALID_ICC"),(ps(icc()).replace(b"% 00000084",b"% Z0000084"),"INVALID_DSC_ENCODING")]:
            with self.subTest(state=state):
                self.assertEqual(analyze(data)["profiles"][0]["state"],state)

    def test_ps_counted_binary_opaque_data_not_scanned_or_executed(self):
        embedded=ps(icc()).split(b"\n",1)[1]
        data=b"%!PS-Adobe-3.0\n%%BeginBinary: "+str(len(embedded)).encode()+b"\n"+embedded+b"\nshowpage\n"
        report=analyze(data)
        self.assertEqual(report["profiles"],[]);self.assertIn("OPAQUE_POSTSCRIPT_DATA",report["warnings"])

    def test_postscript_program_is_not_executed(self):
        with tempfile.TemporaryDirectory() as directory:
            target=Path(directory)/"must-not-exist"
            source=b"%!PS-Adobe-3.0\n("+str(target).encode()+b") (w) file (payload) writestring\n"
            report=analyze(source)
            self.assertFalse(target.exists())
            self.assertEqual(report["profiles"],[])
            self.assertEqual(report["scope"],"static-dsc-declarations-not-execution")

    def test_deep_pdf_graph_is_not_reported_as_complete(self):
        writer=PdfWriter();writer.add_blank_page(72,72)
        nested=ArrayObject()
        for _ in range(40): nested=ArrayObject([nested])
        writer._root_object[NameObject("/JRIPTest")]=nested
        output=io.BytesIO();writer.write(output)
        report=analyze(output.getvalue())
        self.assertEqual(report["status"],"INCOMPLETE")
        self.assertIn("OBJECT_GRAPH_LIMIT",report["warnings"])

    def test_graph_profile_and_source_limits_are_explicit(self):
        with patch("analyze.MAX_PROFILES",1):
            self.assertIn("PROFILE_BUDGET_LIMIT", analyze(pdf(icc(), icc()))["warnings"])
        with patch("analyze.MAX_SOURCE",10):
            with self.assertRaises(ValueError): analyze(b"%!PS-Adobe-3.0\n")

if __name__=="__main__": unittest.main()
