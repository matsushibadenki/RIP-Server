"""Deterministic, self-authored PDF regression corpus; no external fonts/assets."""
from pathlib import Path


def generate():
    objects = []

    def obj(value):
        objects.append(value.encode('ascii') if isinstance(value, str) else value)
        return len(objects)

    def stream(value):
        data = value.encode('ascii')
        return obj(b'<< /Length ' + str(len(data)).encode() + b' >>\nstream\n' + data + b'\nendstream')

    catalog = obj('')
    pages = obj('')
    alpha = obj('<< /Type /ExtGState /ca 0.5 /CA 0.5 >>')
    multiply = obj('<< /Type /ExtGState /BM /Multiply >>')
    overprint = obj('<< /Type /ExtGState /op true /OP true /OPM 1 >>')
    knockout = obj('<< /Type /ExtGState /op false /OP false /OPM 1 >>')
    # Type3 outline glyphs for 日 and 本; deliberately no platform font fallback.
    sun = stream('1000 0 0 0 1000 1000 d1\n100 100 800 800 re 240 240 520 230 re 240 570 520 190 re f*')
    book = stream('1000 0 0 0 1000 1000 d1\n430 80 140 840 re f\n100 650 800 120 re f\n250 240 500 100 re f\n430 650 m 80 150 l 200 150 l 500 570 l f\n570 650 m 920 150 l 800 150 l 500 570 l f')
    unicode_map = stream('/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n/CIDSystemInfo << /Registry (JRIP) /Ordering (Test) /Supplement 0 >> def\n/CMapName /JRIPJapanese def /CMapType 2 def\n1 begincodespacerange <01> <02> endcodespacerange\n2 beginbfchar <01> <65E5> <02> <672C> endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end')
    font = obj(f'<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [.001 0 0 .001 0 0] /CharProcs << /sun {sun} 0 R /book {book} 0 R >> /Encoding << /Type /Encoding /Differences [1 /sun /book] >> /FirstChar 1 /LastChar 2 /Widths [1000 1000] /Resources << >> /ToUnicode {unicode_map} 0 R >>')
    content = [
        '0 0 1 rg 8 8 48 48 re f\nq /Alpha gs 1 0 0 rg 32 32 32 32 re f Q',
        '0 1 1 rg 8 8 48 48 re f\nq /Multiply gs 1 0 0 rg 32 32 32 32 re f Q',
        '1 0 0 0 k 4 4 28 64 re f\nq /Overprint gs 0 0 1 0 k 16 8 12 16 re f Q\nq /Knockout gs 0 0 1 0 k 16 40 12 16 re f Q',
        '0 g BT /Japanese 24 Tf 12 24 Td <0102> Tj ET',
    ]
    page_ids = []
    for operators in content:
        commands = stream(operators)
        page_ids.append(obj(f'<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 72 72] /Resources << /ExtGState << /Alpha {alpha} 0 R /Multiply {multiply} 0 R /Overprint {overprint} 0 R /Knockout {knockout} 0 R >> /Font << /Japanese {font} 0 R >> >> /Contents {commands} 0 R >>'))
    objects[catalog - 1] = f'<< /Type /Catalog /Pages {pages} 0 R >>'.encode()
    objects[pages - 1] = f'<< /Type /Pages /Count 4 /Kids [{" ".join(f"{n} 0 R" for n in page_ids)}] >>'.encode()
    output = bytearray(b'%PDF-1.4\n%JRIP synthetic render regression\n')
    offsets = [0]
    for number, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f'{number} 0 obj\n'.encode() + value + b'\nendobj\n')
    xref = len(output)
    output.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets[1:]:
        output.extend(f'{offset:010d} 00000 n \n'.encode())
    output.extend(f'trailer\n<< /Size {len(objects)+1} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(output)


if __name__ == '__main__':
    Path(__file__).with_name('render-quality.pdf').write_bytes(generate())
