"""Deterministic, explicitly positioned multilingual production-document corpus.

Prepare renamed OFL subsets once from the hash-pinned Noto collection; normal CI
rebuilds the PDF and independent outline probes from the checked-in small subsets.
"""
import hashlib
import io
import json
import math
import re
import sys
from pathlib import Path
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.pens.recordingPen import RecordingPen
from fontTools.pens.pointInsidePen import PointInsidePen

ROOT = Path(__file__).resolve().parent
SOURCE_SHA256 = 'b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a'
PAGES = [
    [('jp',12,'J-RIP / PRINT PROOF'), ('jp',26,'製造指示書'),
     ('jp',16,'Order: JRIP-2026-0042'), ('jp',16,'部品名：制御盤ラベル / CONTROL PANEL'),
     ('jp',16,'数量：120枚　用紙：白色ラベル　検査：合格'),
     ('jp',16,'文字と数字：日本語 ABC xyz 0123456789'),
     ('jp',16,'記号：、。「印刷品質」（確認済み）￥1,280'),
     ('jp',16,'色管理と工程確認を実施してください。'),
     ('jp',16,'Revision 03 / Approved for production'),
     ('jp',12,'Fixture only / 検証用原稿 / No physical print authorization')],
    [('sc',12,'J-RIP / PRINT PROOF'), ('sc',26,'生产检验单'),
     ('sc',16,'Order: JRIP-2026-0042'), ('sc',16,'产品名称：控制面板标签 / CONTROL PANEL'),
     ('sc',16,'数量：120张　纸张：白色标签　检验：合格'),
     ('sc',16,'文字与数字：简体中文 ABC xyz 0123456789'),
     ('sc',16,'标点：，。“打印质量”（确认完成）￥1,280'),
     ('sc',16,'请检查颜色管理、字形和页面布局。'),
     ('sc',16,'Revision 03 / Approved for production'),
     ('sc',12,'Fixture only / 测试原稿 / No physical print authorization')],
    [('jp',12,'J-RIP / MULTILINGUAL PACKING LIST'), ('jp',26,'Shipment 0042'),
     ('jp',16,'Item: Control panel label'),
     ('jp',16,'Quantity: 120 / Status: PASS'),
     ('jp',16,'日本語：制御盤ラベル、検査合格。'),
     ('sc',16,'简体中文：控制面板标签，检验合格。'),
     ('jp',16,'Invoice total: JPY 1,280 / USD 8.50'),
     ('jp',16,'Check text, punctuation and decimal alignment.'),
     ('jp',16,'Address: 1-2-3 Tokyo / Delivery: 2026-11-06'),
     ('sc',16,'说明：本原稿仅用于软件渲染回归测试。'),
     ('jp',12,'Fixture only / 検証用 / 测试用 / No physical print authorization')]
]


def prepare(source):
    assert hashlib.sha256(Path(source).read_bytes()).hexdigest() == SOURCE_SHA256
    for key, index in [('jp',0), ('sc',2)]:
        font = TTFont(source, fontNumber=index, recalcTimestamp=False)
        chars = {ord(c) for page in PAGES for language, _, text in page if language == key for c in text}
        assert chars <= font.getBestCmap().keys()
        operation = subset.Subsetter(options=subset.Options())
        operation.populate(unicodes=chars)
        operation.subset(font)
        name = 'JRIPCorpusCJK'+key.upper()
        for ident in (1,3,4,6,16):
            for platform, encoding, language in [(3,1,0x409),(1,0,0)]:
                font['name'].setName(name,ident,platform,encoding,language)
        cff = font['CFF '].cff
        cff.fontNames = [name]
        top = cff.topDictIndex[0]
        top.FullName = top.FamilyName = name
        for i, fd in enumerate(top.FDArray):
            if hasattr(fd,'FontName'): fd.FontName = name+'FD'+str(i)
        font['head'].created = font['head'].modified = 2082844800
        font.save(ROOT/(key+'.otf'))
    metadata={'source':'Debian bookworm fonts-noto-cjk / NotoSansCJK-Regular.ttc', 'sourceSha256':SOURCE_SHA256, 'license':'SIL Open Font License 1.1', 'subsets':{}}
    for key,index in [('jp',0),('sc',2)]:
        font=TTFont(ROOT/(key+'.otf'))
        metadata['subsets'][key]={'collectionIndex':index,'sha256':hashlib.sha256((ROOT/(key+'.otf')).read_bytes()).hexdigest(),'postscriptName':font['name'].getDebugName(6),'version':font['name'].getDebugName(5),'glyphs':len(font.getGlyphOrder()),'fontDicts':len(font['CFF '].cff.topDictIndex[0].FDArray)}
    (ROOT/'PROVENANCE.json').write_text(json.dumps(metadata,ensure_ascii=False,indent=2)+'\n')


def generate():
    metadata=json.loads((ROOT/'PROVENANCE.json').read_text())
    assert metadata['sourceSha256']==SOURCE_SHA256
    for key in ('jp','sc'):
        assert hashlib.sha256((ROOT/(key+'.otf')).read_bytes()).hexdigest()==metadata['subsets'][key]['sha256']
    fonts = {key: TTFont(ROOT/(key+'.otf'),recalcTimestamp=False) for key in ('jp','sc')}
    objects = []
    def obj(data):
        objects.append(data.encode() if isinstance(data,str) else data)
        return len(objects)
    def stream(data, extra=''):
        return obj(f'<< /Length {len(data)} {extra} >>\nstream\n'.encode()+data+b'\nendstream')
    catalog = obj('')
    pages_id = obj('')
    resources = {}
    maps = {}
    for key, font in fonts.items():
        top = font['CFF '].cff.topDictIndex[0]
        assert top.ROS == ('Adobe','Identity',0)
        mapping = {code: int(name[3:]) for code,name in font.getBestCmap().items()}
        maps[key] = mapping
        buffer = io.BytesIO(); font['CFF '].cff.compile(buffer,font)
        program = stream(buffer.getvalue(),'/Subtype /CIDFontType0C')
        name = 'JRIPCorpusCJK'+key.upper()
        descriptor = obj(f'<< /Type /FontDescriptor /FontName /{name} /Flags 4 /FontBBox [-1000 -1100 3000 3000] /ItalicAngle 0 /Ascent 1160 /Descent -288 /CapHeight 733 /StemV 80 /FontFile3 {program} 0 R >>')
        widths = ' '.join(f'{cid} [{font["hmtx"].metrics[font.getBestCmap()[code]][0]}]' for code,cid in sorted(mapping.items()))
        descendant = obj(f'<< /Type /Font /Subtype /CIDFontType0 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R /DW 1000 /W [{widths}] >>')
        codes = sorted(mapping)
        cmap = ['/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def /CMapName /JRIP-'+key+' def /CMapType 1 def /WMode 0 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange']
        unicode = ['/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /JRIP-UCS-'+key+' def /CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange']
        for offset in range(0,len(codes),100):
            group=codes[offset:offset+100]
            cmap += [f'{len(group)} begincidchar']+[f'<{c:04X}> {mapping[c]}' for c in group]+['endcidchar']
            unicode += [f'{len(group)} beginbfchar']+[f'<{c:04X}> <{c:04X}>' for c in group]+['endbfchar']
        ending='endcmap CMapName currentdict /CMap defineresource pop end end'
        encoding = stream(('\n'.join(cmap+[ending])).encode(),'/Type /CMap /WMode 0')
        to_unicode = stream(('\n'.join(unicode+[ending])).encode())
        resources[key] = obj(f'<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding {encoding} 0 R /DescendantFonts [{descendant} 0 R] /ToUnicode {to_unicode} 0 R >>')
    outline_cache = {}
    def outline_probes(key, code):
        cachekey=(key,code)
        if cachekey in outline_cache: return outline_cache[cachekey]
        glyphset=fonts[key].getGlyphSet()
        recording=RecordingPen();glyphset[fonts[key].getBestCmap()[code]].draw(recording)
        def inside(x,y):
            pen=PointInsidePen(glyphset,(x,y),evenOdd=False)
            recording.replay(pen)
            return pen.getResult()
        found={True:[],False:[]}
        advance=fonts[key]['hmtx'].metrics[fonts[key].getBestCmap()[code]][0]
        for y in range(50,951,50):
            for x in range(50,min(951,advance-80),50):
                value=inside(x,y)
                if len(found[value])>=3: continue
                if all(inside(x+dx,y+dy)==value for dx in (-40,0,40) for dy in (-40,0,40)):
                    found[value].append((x,y))
            if all(len(v)>=3 for v in found.values()): break
        result=[(x,y,0 if state else 255) for state, points in found.items() for x,y in points]
        outline_cache[cachekey]=result
        return result
    pages=[];probe_pages=[]
    for lines in PAGES:
        commands=['0 g']; probes=[];y=780;glyphs=0
        for key,size,text in lines:
            assert all(ord(c) in maps[key] for c in text)
            width=sum(fonts[key]['hmtx'].metrics[fonts[key].getBestCmap()[ord(c)]][0] for c in text)*size/1000
            assert width <= 499,(text,width)
            commands.append(f'BT /{key} {size} Tf 1 0 0 1 48 {y} Tm <'+''.join(f'{ord(c):04X}' for c in text)+'> Tj ET')
            x=48
            for c in text:
                if not c.isspace():
                    for px,py,value in outline_probes(key,ord(c)):
                        # Pixel coordinates for bottom-left PDF points at 288 dpi.
                        probes.append([math.floor((x+px*size/1000)*4),math.floor((842-y-py*size/1000)*4),value])
                x+=fonts[key]['hmtx'].metrics[fonts[key].getBestCmap()[ord(c)]][0]*size/1000
                glyphs+=1
            y-=44 if size==26 else 38
        content=stream(('\n'.join(commands)).encode())
        pages.append(obj(f'<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 595 842] /Resources << /Font << /jp {resources["jp"]} 0 R /sc {resources["sc"]} 0 R >> >> /Contents {content} 0 R >>'))
        probe_pages.append({'characterCount':glyphs,'probes':probes})
    objects[catalog-1]=f'<< /Type /Catalog /Pages {pages_id} 0 R >>'.encode()
    objects[pages_id-1]=f'<< /Type /Pages /Count {len(pages)} /Kids [{" ".join(f"{p} 0 R" for p in pages)}] >>'.encode()
    output=bytearray(b'%PDF-1.4\n% J-RIP multilingual corpus\n');offsets=[]
    for number,data in enumerate(objects,1):
        offsets.append(len(output));output.extend(f'{number} 0 obj\n'.encode()+data+b'\nendobj\n')
    xref=len(output);output.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets: output.extend(f'{offset:010d} 00000 n \n'.encode())
    output.extend(f'trailer\n<< /Size {len(objects)+1} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(output), (json.dumps({'dpi':288,'width':2380,'height':3368,'pages':probe_pages},separators=(',',':'))+'\n').encode()


if __name__=='__main__':
    if sys.argv[1:2]==['--prepare']:
        prepare(sys.argv[2])
    elif sys.argv[1:] not in ([],['--check']):
        raise SystemExit('usage: generate.py [--check | --prepare source.ttc]')
    pdf,probes=generate()
    for name,data in [('multilingual-quality.pdf',pdf),('probes.json',probes)]:
        if sys.argv[1:]==['--check']:
            assert (ROOT/name).read_bytes()==data,name+' differs'
        else: (ROOT/name).write_bytes(data)
