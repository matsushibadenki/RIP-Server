"""Generate owned outline fonts, actually subset them, and embed in a PDF.

No system/external font is read. Requires only the pinned fontTools package.
"""
from io import BytesIO
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools import subset
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable


def outlines(pen, name):
    def polygon(points):
        pen.moveTo(points[0])
        for point in points[1:]:
            pen.lineTo(point)
        pen.closePath()

    def rect(x, y, w, h, hole=False):
        points = [(x, y), (x, y+h), (x+w, y+h), (x+w, y)]
        polygon(points[::-1] if hole else points)

    if name == 'uni65E5':  # 日
        rect(100, 100, 800, 800)
        rect(240, 240, 520, 230, hole=True)
        rect(240, 570, 520, 190, hole=True)
        rect(400, 600, 100, 100)  # Owned sentinel inside counter: detects fallback fonts.
    elif name == 'uni672C':  # 本
        rect(430, 80, 140, 840)
        rect(100, 650, 800, 120)
        rect(250, 240, 500, 100)
        # Match winding so overlapping stem/diagonals remain filled.
        polygon([(430, 650), (500, 570), (200, 150), (80, 150)])
        polygon([(570, 650), (920, 150), (800, 150), (500, 570)])
    else:
        rect(100, 100, 800, 800)


def font(is_ttf):
    name = 'JRIPAA+OwnedTT' if is_ttf else 'JRIPCC+OwnedCFF'
    builder = FontBuilder(1000, isTTF=is_ttf)
    order = ['.notdef', 'uni672C', 'uni8A9E', 'uni65E5']
    builder.setupGlyphOrder(order)
    builder.setupCharacterMap({0x65E5: 'uni65E5', 0x672C: 'uni672C', 0x8A9E: 'uni8A9E'})
    if is_ttf:
        glyphs = {}
        for glyph in order:
            pen = TTGlyphPen(None)
            outlines(pen, glyph)
            glyphs[glyph] = pen.glyph()
        builder.setupGlyf(glyphs)
    else:
        chars = {}
        for glyph in order:
            pen = T2CharStringPen(1000, None)
            outlines(pen, glyph)
            chars[glyph] = pen.getCharString()
        builder.setupCFF(name, {'FullName': name, 'FamilyName': 'JRIP Owned', 'Weight': 'Regular'}, chars, {})
    builder.setupHorizontalMetrics({glyph: (1000, 80 if glyph == 'uni672C' else 100) for glyph in order})
    builder.setupHorizontalHeader(ascent=1000, descent=0)
    builder.setupNameTable({'familyName': 'JRIP Owned', 'styleName': 'Regular', 'uniqueFontIdentifier': name, 'fullName': name, 'psName': name})
    builder.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
    builder.setupPost()
    if is_ttf:
        symbol = CmapSubtable.newSubtable(4)
        symbol.platformID, symbol.platEncID, symbol.language = 3, 0, 0
        symbol.cmap = {0xF041: 'uni65E5', 0xF042: 'uni672C'}
        builder.font['cmap'].tables.append(symbol)
    builder.font.recalcTimestamp = False
    builder.font['head'].created = builder.font['head'].modified = 2082844800
    options = subset.Options()
    options.glyph_names = True
    options.symbol_cmap = True
    sub = subset.Subsetter(options=options)
    sub.populate(unicodes=[0x65E5, 0x672C], glyphs=['uni65E5', 'uni672C'])
    sub.subset(builder.font)
    assert 'uni8A9E' not in builder.font.getGlyphOrder()
    assert len(builder.font.getGlyphOrder()) == 3
    if is_ttf:
        result = BytesIO()
        builder.font.save(result)
        data = result.getvalue()
    else:
        data = builder.font['CFF '].compile(builder.font)
    return name, data, builder.font


def generate():
    objects = []

    def obj(value):
        objects.append(value.encode('ascii') if isinstance(value, str) else value)
        return len(objects)

    def stream(data, extra=''):
        if isinstance(data, str):
            data = data.encode('ascii')
        return obj(f'<< /Length {len(data)} {extra} >>\nstream\n'.encode() + data + b'\nendstream')

    catalog, pages = obj(''), obj('')
    tt_name, ttf, tt = font(True)
    cff_name, cff, _ = font(False)
    tt_stream = stream(ttf, f'/Length1 {len(ttf)}')
    cff_stream = stream(cff, '/Subtype /Type1C')
    descriptor = lambda name, key, ref: obj(f'<< /Type /FontDescriptor /FontName /{name} /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 1000 /Descent 0 /CapHeight 900 /StemV 100 /{key} {ref} 0 R >>')
    tt_desc, cff_desc = descriptor(tt_name, 'FontFile2', tt_stream), descriptor(cff_name, 'FontFile3', cff_stream)

    def unicode_map(cid):
        first, last = ('0000', 'FFFF') if cid else ('00', 'FF')
        mappings = '<0007> <65E5>\n<0009> <672C>' if cid else '<41> <65E5>\n<42> <672C>'
        return stream(f'/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n/CIDSystemInfo << /Registry (JRIP) /Ordering (Test) /Supplement 0 >> def\n/CMapName /JRIPOwned def /CMapType 2 def\n1 begincodespacerange <{first}> <{last}> endcodespacerange\n2 beginbfchar {mappings} endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end')

    encoding = '/Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [65 /uni65E5 /uni672C] >> /FirstChar 65 /LastChar 66 /Widths [1000 1000]'
    simple_tt = obj(f'<< /Type /Font /Subtype /TrueType /BaseFont /{tt_name} /FirstChar 65 /LastChar 66 /Widths [1000 1000] /FontDescriptor {tt_desc} 0 R /ToUnicode {unicode_map(False)} 0 R >>')
    # CID values intentionally differ from Unicode and glyph IDs after subsetting.
    mapping = bytearray(20)
    for cid, glyph in [(7, 'uni65E5'), (9, 'uni672C')]:
        gid = tt.getGlyphID(glyph)
        assert gid != cid
        mapping[cid*2:cid*2+2] = gid.to_bytes(2, 'big')
    cid_map = stream(bytes(mapping))
    descendant = obj(f'<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{tt_name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {tt_desc} 0 R /DW 1000 /CIDToGIDMap {cid_map} 0 R >>')
    cid_tt = obj(f'<< /Type /Font /Subtype /Type0 /BaseFont /{tt_name} /Encoding /Identity-H /DescendantFonts [{descendant} 0 R] /ToUnicode {unicode_map(True)} 0 R >>')
    simple_cff = obj(f'<< /Type /Font /Subtype /Type1 /BaseFont /{cff_name} {encoding} /FontDescriptor {cff_desc} 0 R /ToUnicode {unicode_map(False)} 0 R >>')
    page_ids = []
    for font_ref, chars in [(simple_tt, '4142'), (cid_tt, '00070009'), (simple_cff, '4142')]:
        commands = stream(f'0 g BT /Japanese 24 Tf 12 24 Td <{chars}> Tj ET')
        page_ids.append(obj(f'<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 72 72] /Resources << /Font << /Japanese {font_ref} 0 R >> >> /Contents {commands} 0 R >>'))
    objects[0] = f'<< /Type /Catalog /Pages {pages} 0 R >>'.encode()
    objects[1] = f'<< /Type /Pages /Count 3 /Kids [{" ".join(f"{n} 0 R" for n in page_ids)}] >>'.encode()
    output = bytearray(b'%PDF-1.4\n%JRIP owned subset font regression\n')
    offsets = []
    for number, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f'{number} 0 obj\n'.encode() + value + b'\nendobj\n')
    xref = len(output)
    output.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        output.extend(f'{offset:010d} 00000 n \n'.encode())
    output.extend(f'trailer\n<< /Size {len(objects)+1} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(output)


if __name__ == '__main__':
    import sys
    destination = Path(__file__).with_name('font-quality.pdf')
    data = generate()
    if sys.argv[1:] == ['--check']:
        if destination.read_bytes() != data:
            raise SystemExit('font-quality.pdf differs; regenerate with pinned requirements')
    elif sys.argv[1:]:
        raise SystemExit('usage: generate.py [--check]')
    else:
        destination.write_bytes(data)
