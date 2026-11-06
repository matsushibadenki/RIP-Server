"""Owned horizontal/vertical punctuation with explicit PDF CMap glyph selection.

This corpus tests PDF character-code -> CID -> glyph interpretation, not GSUB shaping.
No external typeface or source document is read.
"""
from io import BytesIO
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools import subset

GLYPHS = ['.notdef', 'period.h', 'comma.v', 'comma.h', 'bracket.v', 'bracket.h', 'period.v', 'unused']
CIDS = {'comma.h': 21, 'period.h': 22, 'bracket.h': 23,
        'comma.v': 31, 'period.v': 32, 'bracket.v': 33}
CODES = {'comma': 0x3001, 'period': 0x3002, 'bracket': 0x300C}


def outline(pen, name):
    def rect(x, y, width, height, hole=False):
        points = [(x, y), (x, y+height), (x+width, y+height), (x+width, y)]
        if name == 'bracket.v':
            points = [(y, 1000-x) for x, y in points]
        if hole:
            points.reverse()
        pen.moveTo(points[0])
        for point in points[1:]:
            pen.lineTo(point)
        pen.closePath()

    vertical = name.endswith('.v')
    shift = 600 if vertical else 0
    if name.startswith('comma.'):
        rect(100+shift, 100+shift, 200, 200)
        rect(140+shift, 40+shift, 80, 60)
    elif name.startswith('period.'):
        rect(100+shift, 100+shift, 240, 240)
        rect(160+shift, 160+shift, 120, 120, hole=True)
    elif name.startswith('bracket.'):
        rect(100, 100, 120, 800)
        rect(100, 780, 620, 120)
        rect(800, 150, 80, 80)  # Owned asymmetric rotation sentinel.
    else:
        rect(100, 100, 800, 800)


def program(truetype):
    name = 'JRIPPT+OwnedPunctuation' if truetype else 'JRIPPC+OwnedPunctuation'
    builder = FontBuilder(1000, isTTF=truetype)
    builder.setupGlyphOrder(GLYPHS)
    builder.setupCharacterMap({code: kind+'.h' for kind, code in CODES.items()})
    glyphs = {}
    for glyph in GLYPHS:
        pen = TTGlyphPen(None) if truetype else T2CharStringPen(1000, None)
        outline(pen, glyph)
        glyphs[glyph] = pen.glyph() if truetype else pen.getCharString()
    if truetype:
        builder.setupGlyf(glyphs)
    else:
        builder.setupCFF(name, {'FullName': name, 'FamilyName': 'JRIP Owned Punctuation', 'Weight': 'Regular'}, glyphs, {})
    builder.setupHorizontalMetrics({glyph: (1000, 700 if glyph in ('comma.v', 'period.v') else 100) for glyph in GLYPHS})
    builder.setupHorizontalHeader(ascent=1000, descent=0)
    builder.setupNameTable({'familyName': 'JRIP Owned Punctuation', 'styleName': 'Regular',
                            'uniqueFontIdentifier': name, 'fullName': name, 'psName': name})
    builder.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
    builder.setupPost()
    builder.font.recalcTimestamp = False
    builder.font['head'].created = builder.font['head'].modified = 2082844800
    options = subset.Options()
    options.glyph_names = True
    operation = subset.Subsetter(options=options)
    operation.populate(unicodes=list(CODES.values()), glyphs=[name for name in CIDS if name.endswith('.v')])
    operation.subset(builder.font)
    assert 'unused' not in builder.font.getGlyphOrder()
    assert len(builder.font.getGlyphOrder()) == 7
    if truetype:
        data = BytesIO()
        builder.font.save(data)
        return name, data.getvalue(), builder.font
    from fontTools.cffLib import FDArrayIndex, FDSelect, FontDict
    cff = builder.font['CFF '].cff
    top = cff.topDictIndex[0]
    order = builder.font.getGlyphOrder()
    old = top.CharStrings.charStrings
    top.ROS = ('Adobe', 'Identity', 0)
    top.CIDCount = 34
    top.charset = ['.notdef' if glyph == '.notdef' else f'cid{CIDS[glyph]:05}' for glyph in order]
    top.CharStrings.charStrings = {cid: old[glyph] for cid, glyph in zip(top.charset, order)}
    fd = FontDict()
    fd.Private = top.Private
    top.FDArray = FDArrayIndex()
    top.FDArray.append(fd)
    top.FDSelect = FDSelect(format=0)
    top.FDSelect.gidArray = [0] * len(order)
    top.CharStrings.fdArray = top.FDArray
    top.CharStrings.fdSelect = top.FDSelect
    del top.Private
    if hasattr(top, 'Encoding'):
        del top.Encoding
    return name, builder.font['CFF '].compile(builder.font), builder.font


def generate():
    objects = []

    def obj(value):
        objects.append(value.encode('ascii') if isinstance(value, str) else value)
        return len(objects)

    def stream(value, extra=''):
        value = value.encode('ascii') if isinstance(value, str) else value
        return obj(f'<< /Length {len(value)} {extra} >>\nstream\n'.encode() + value + b'\nendstream')

    catalog, pages = obj(''), obj('')
    system = '/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >>'
    encodings = []
    for vertical in [False, True]:
        suffix = 'V' if vertical else 'H'
        mode = int(vertical)
        mappings = '\n'.join(f'<{code:04X}> {CIDS[kind+(".v" if vertical else ".h")]}' for kind, code in CODES.items())
        cmap = f'/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n{system} def\n/CMapName /JRIPPunctuation{suffix} def /CMapType 1 def /WMode {mode} def\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n3 begincidchar\n{mappings}\nendcidchar\nendcmap CMapName currentdict /CMap defineresource pop end end'
        encodings.append(stream(cmap, f'/Type /CMap /CMapName /JRIPPunctuation{suffix} {system} /WMode {mode}'))
    unicode_mappings = '\n'.join(f'<{code:04X}> <{code:04X}>' for code in CODES.values())
    unicode = stream(f'/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n{system} def\n/CMapName /JRIPPunctuationUnicode def /CMapType 2 def\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n3 beginbfchar\n{unicode_mappings}\nendbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end')
    page_ids = []
    for truetype in [True, False]:
        name, data, font = program(truetype)
        program_ref = stream(data, f'/Length1 {len(data)}' if truetype else '/Subtype /CIDFontType0C')
        key = 'FontFile2' if truetype else 'FontFile3'
        descriptor = obj(f'<< /Type /FontDescriptor /FontName /{name} /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 1000 /Descent 0 /CapHeight 940 /StemV 100 /{key} {program_ref} 0 R >>')
        mapping = ''
        if truetype:
            table = bytearray(68)
            for glyph, cid in CIDS.items():
                gid = font.getGlyphID(glyph)
                assert gid != cid
                table[cid*2:cid*2+2] = gid.to_bytes(2, 'big')
            mapping = f'/CIDToGIDMap {stream(bytes(table))} 0 R'
        metrics = ' '.join(f'{cid} [-1200 500 1000]' for cid in sorted(CIDS.values()))
        subtype = 'CIDFontType2' if truetype else 'CIDFontType0'
        descendant = obj(f'<< /Type /Font /Subtype /{subtype} /BaseFont /{name} {system} /FontDescriptor {descriptor} 0 R /DW 1000 /DW2 [880 -1000] /W2 [{metrics}] {mapping} >>')
        fonts = [obj(f'<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding {encoding} 0 R /DescendantFonts [{descendant} 0 R] /ToUnicode {unicode} 0 R >>') for encoding in encodings]
        for code in CODES.values():
            for vertical in [False, True]:
                matrix = '1 0 0 1 36 56' if vertical else '1 0 0 1 16 16'
                content = stream(f'0 g BT /Punctuation 40 Tf {matrix} Tm <{code:04X}> Tj ET')
                page_ids.append(obj(f'<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 72 72] /Resources << /Font << /Punctuation {fonts[int(vertical)]} 0 R >> >> /Contents {content} 0 R >>'))
    objects[0] = f'<< /Type /Catalog /Pages {pages} 0 R >>'.encode()
    objects[1] = f'<< /Type /Pages /Count 12 /Kids [{" ".join(f"{ref} 0 R" for ref in page_ids)}] >>'.encode()
    output = bytearray(b'%PDF-1.4\n%JRIP owned punctuation regression\n')
    offsets = []
    for index, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f'{index} 0 obj\n'.encode() + value + b'\nendobj\n')
    xref = len(output)
    output.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        output.extend(f'{offset:010} 00000 n \n'.encode())
    output.extend(f'trailer\n<< /Size {len(objects)+1} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(output)
