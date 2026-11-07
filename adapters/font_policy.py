"""Bounded PDF resource font policy; no system fonts or external files are read.

This inventories declared resources, not executed glyph use or shaping quality.
"""
from pathlib import Path
from font_identity import identify, PROFILE as IDENTITY_PROFILE, MAX_TOTAL
from pypdf import PdfReader
from pypdf.generic import ArrayObject, DictionaryObject, IndirectObject, StreamObject

PROFILE = "org.jrip.pdf-font-policy-v1"
STANDARD14 = frozenset(("Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic",
    "Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique",
    "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique", "Symbol", "ZapfDingbats"))
MAX_NODES = 8192
MAX_FONTS = 128


def inspect_fonts(path):
    if Path(path).stat().st_size > 32 * 1024 * 1024:
        raise ValueError("font_policy_source_limit")
    reader = PdfReader(path, strict=True)
    if reader.is_encrypted or not 1 <= len(reader.pages) <= 100:
        raise ValueError("font_policy_document_limit")
    seen, fonts, records = set(), set(), []
    visited = 0
    programs = {}
    total_program_bytes = 0

    def resolve(obj):
        nonlocal visited
        visited += 1
        if visited > MAX_NODES:
            raise ValueError("font_policy_object_limit")
        return obj.get_object() if isinstance(obj, IndirectObject) else obj

    def key(obj):
        return (obj.idnum, obj.generation) if isinstance(obj, IndirectObject) else (id(obj),)

    def font(obj):
        nonlocal total_program_bytes
        program_identity = None
        identity = key(obj)
        if identity in fonts:
            return
        fonts.add(identity)
        if len(fonts) > MAX_FONTS:
            raise ValueError("font_policy_font_limit")
        value = resolve(obj)
        if not isinstance(value, DictionaryObject):
            raise ValueError("font_policy_invalid_font")
        kind, name = str(value.get('/Subtype', '')), str(value.get('/BaseFont', ''))[1:]
        if len(name) > 128:
            raise ValueError("font_policy_name_limit")
        if kind == '/Type3':
            procs = resolve(value.get('/CharProcs'))
            if not isinstance(procs, DictionaryObject) or not procs:
                raise ValueError("font_policy_invalid_type3")
            state = 'SELF_CONTAINED_TYPE3'
        else:
            target = value
            if kind == '/Type0':
                children = resolve(value.get('/DescendantFonts'))
                if not isinstance(children, ArrayObject) or len(children) != 1:
                    raise ValueError("font_policy_invalid_descendant")
                target = resolve(children[0])
                if not isinstance(target, DictionaryObject) or target.get('/Subtype') not in ('/CIDFontType0', '/CIDFontType2'):
                    raise ValueError("font_policy_invalid_descendant")
            elif kind not in ('/Type1', '/MMType1', '/TrueType'):
                raise ValueError("font_policy_unsupported_font")
            descriptor = resolve(target.get('/FontDescriptor'))
            embedded = []
            if isinstance(descriptor, DictionaryObject):
                for field in ('/FontFile', '/FontFile2', '/FontFile3'):
                    if field in descriptor:
                        stream = resolve(descriptor.raw_get(field))
                        if not isinstance(stream, StreamObject) or not stream._data or any(k in stream for k in ('/F', '/FFilter', '/FDecodeParms')):
                            raise ValueError("font_policy_invalid_program")
                        subtype = str(target.get('/Subtype', ''))
                        allowed = {
                            '/TrueType': ('/FontFile2', '/FontFile3'),
                            '/CIDFontType2': ('/FontFile2', '/FontFile3'),
                            '/Type1': ('/FontFile', '/FontFile3'),
                            '/MMType1': ('/FontFile', '/FontFile3'),
                            '/CIDFontType0': ('/FontFile3',),
                        }[subtype]
                        if field not in allowed:
                            raise ValueError("font_policy_program_type_mismatch")
                        if field == '/FontFile3':
                            allowed_subtypes = ('/OpenType', '/CIDFontType0C') if subtype == '/CIDFontType0' else (('/OpenType',) if subtype in ('/TrueType', '/CIDFontType2') else ('/Type1C', '/OpenType'))
                            if stream.get('/Subtype') not in allowed_subtypes:
                                raise ValueError("font_policy_program_type_mismatch")
                        program_key = key(descriptor.raw_get(field))
                        if program_key not in programs:
                            programs[program_key] = identify(stream, MAX_TOTAL-total_program_bytes)
                            total_program_bytes += programs[program_key].charged_bytes
                        program_identity = dict(programs[program_key], field=field[1:])
                        embedded.append(field)
            if len(embedded) > 1:
                raise ValueError("font_policy_ambiguous_program")
            if embedded:
                state = 'EMBEDDED'
            elif kind == '/Type1' and name in STANDARD14:
                state = 'STANDARD14_SUBSTITUTION'
            else:
                raise ValueError("font_policy_unembedded_font")
        record = {'baseFont': name, 'subtype': kind[1:], 'state': state}
        if program_identity is not None: record['programIdentity'] = program_identity
        records.append(record)

    def walk(obj, depth=0):
        if depth > 48:
            raise ValueError("font_policy_depth_limit")
        identity = key(obj)
        if identity in seen:
            return
        seen.add(identity)
        value = resolve(obj)
        if isinstance(value, DictionaryObject):
            # /Parent would escape resources into the entire page tree.
            for field, child in value.items():
                if field == '/Parent':
                    continue
                if field == '/Font':
                    table = resolve(child)
                    if not isinstance(table, DictionaryObject) or len(table) > MAX_FONTS:
                        raise ValueError("font_policy_invalid_resources")
                    for entry in table.values():
                        font(entry)
                        # Font metrics/encodings are validated by MuPDF, not this
                        # resource inventory. Large CJK width arrays must not
                        # consume the resource-graph budget.
                        declared = resolve(entry)
                        walk(declared.get('/Resources', DictionaryObject()), depth + 1)
                    continue
                walk(child, depth + 1)
        elif isinstance(value, ArrayObject):
            for child in value:
                walk(child, depth + 1)

    for page in reader.pages:
        walk(page.get('/Resources', DictionaryObject()))
        walk(page.get('/Annots', ArrayObject()))
    acroform = reader.trailer['/Root'].get('/AcroForm')
    if acroform is not None:
        walk(acroform)
    return {'profile': PROFILE, 'programIdentityProfile': IDENTITY_PROFILE, 'accepted': True, 'scope': 'declared-resources',
            'fonts': records, 'standard14Substitution': any(r['state'] == 'STANDARD14_SUBSTITUTION' for r in records)}
