"""Resolve only exact registered font names into a private derived PDF.
The immutable registry hash and every used program hash are checked. Source bytes
and embedded font programs are never changed. No generic font substitution.
"""
import hashlib,json,os
from pathlib import Path
from pypdf import PdfReader,PdfWriter
from pypdf.generic import DictionaryObject,NameObject,NumberObject,ArrayObject,DecodedStreamObject,FloatObject,IndirectObject
PROFILE='org.jrip.registered-font-resolution-v1'
ROOT=Path('/jrip-fonts')
LAST_RESOLUTION=None

def registry():
    expected=os.environ.get('JRIP_FONT_MANIFEST_SHA256')
    if not expected:return None
    raw=(ROOT/'manifest.json').read_bytes()
    if len(raw)>256*1024 or hashlib.sha256(raw).hexdigest()!=expected:raise ValueError('font_registry_integrity')
    manifest=json.loads(raw)
    if manifest.get('version')!=1 or not isinstance(manifest.get('fonts'),list) or len(manifest['fonts'])>128:raise ValueError('font_registry_invalid')
    result={}
    for f in manifest['fonts']:
        name=f['name']
        if not name or len(name)>128 or any(not(c.isascii() and (c.isalnum() or c in '-_.+')) for c in name) or name in result:raise ValueError('font_registry_invalid')
        if f['format'] not in ('ttf','otf') or f['file']!=f['sha256']+'.'+f['format'] or len(f['sha256'])!=64:raise ValueError('font_registry_invalid')
        result[name]=f
    return result

def program(f):
    path=ROOT/f['file']
    if path.resolve()!=path or not 0<f['bytes']<=16*1024*1024:raise ValueError('font_registry_invalid')
    with path.open('rb') as source:data=source.read(16*1024*1024+1)
    if len(data)!=f['bytes'] or hashlib.sha256(data).hexdigest()!=f['sha256']:raise ValueError('font_registry_integrity')
    return data

def verify_postscript_registry():
    fonts=registry()
    if fonts is None:return False
    expected='(Fontmap.GS) .runlibfile\n'
    for f in fonts.values():
        program(f)
        expected+='/'+f['name']+' (/jrip-fonts/'+f['file']+') ;\n'
    if (ROOT/'Fontmap').read_text()!=expected:raise ValueError('font_registry_integrity')
    return True

def prepare_pdf(path):
    global LAST_RESOLUTION
    LAST_RESOLUTION=None
    fonts=registry()
    if not fonts:return path
    if Path(path).stat().st_size>32*1024*1024:raise ValueError('font_source_limit')
    reader=PdfReader(path,strict=True)
    if reader.is_encrypted or not 1<=len(reader.pages)<=100:raise ValueError('font_source_limit')
    writer=PdfWriter(clone_from=reader)
    seen=set();resolved=[];streams={};total=0
    def visit(obj,depth=0):
        nonlocal total
        if depth>48:raise ValueError('font_object_limit')
        if isinstance(obj,IndirectObject):obj=obj.get_object()
        if not isinstance(obj,(DictionaryObject,list,ArrayObject)):return
        if id(obj) in seen:return
        seen.add(id(obj))
        if len(seen)>8192:raise ValueError('font_object_limit')
        if isinstance(obj,DictionaryObject):
            kind=str(obj.get('/Subtype',''));name=str(obj.get('/BaseFont',''))[1:]
            if kind in ('/Type1','/TrueType','/Type0') and name in fonts:
                target=obj
                if kind=='/Type0':
                    descendants=obj.get('/DescendantFonts')
                    if descendants is None or len(descendants)!=1:raise ValueError('font_descendant_invalid')
                    target=descendants[0].get_object()
                desc=target.get('/FontDescriptor')
                desc=desc.get_object() if desc is not None else None
                if desc is None or not any(key in desc for key in ('/FontFile','/FontFile2','/FontFile3')):
                    f=fonts[name]
                    compatible=(kind=='/TrueType' and f['format']=='ttf') or (kind=='/Type1' and f['format']=='otf' and not f['cid']) or (kind=='/Type0' and str(obj.get('/Encoding','')) in ('/Identity-H','/Identity-V') and ((target.get('/Subtype')=='/CIDFontType2' and f['format']=='ttf') or (target.get('/Subtype')=='/CIDFontType0' and f['cid'])))
                    if not compatible:raise ValueError('registered_font_kind_mismatch')
                    if desc is not None and str(desc.get('/FontName','/'+name))[1:]!=name:raise ValueError('registered_font_name_mismatch')
                    if name not in streams:
                        data=program(f);total+=len(data)
                        if total>64*1024*1024:raise ValueError('font_program_limit')
                        stream=DecodedStreamObject();stream.set_data(data)
                        if f['format']=='otf':stream[NameObject('/Subtype')]=NameObject('/OpenType')
                        streams[name]=writer._add_object(stream)
                    if desc is None:
                        desc=DictionaryObject({NameObject('/Type'):NameObject('/FontDescriptor'),NameObject('/FontName'):NameObject('/'+name),NameObject('/Flags'):NumberObject(f['flags']),NameObject('/FontBBox'):ArrayObject([NumberObject(n) for n in f['bbox']]),NameObject('/Ascent'):NumberObject(f['ascent']),NameObject('/Descent'):NumberObject(f['descent']),NameObject('/CapHeight'):NumberObject(f['capHeight']),NameObject('/ItalicAngle'):FloatObject(f['italicAngle']),NameObject('/StemV'):NumberObject(80)})
                        target[NameObject('/FontDescriptor')]=writer._add_object(desc)
                    desc[NameObject('/FontFile2' if f['format']=='ttf' else '/FontFile3')]=streams[name]
                    resolved.append({'name':name,'sha256':f['sha256'],'bytes':f['bytes'],'format':f['format']})
            for key,value in list(obj.items()):
                if key not in ('/Parent','/P'):visit(value,depth+1)
        else:
            for value in list(obj):visit(value,depth+1)
    for obj in list(writer._objects):visit(obj)
    if not resolved:return path
    output=Path('/tmp/jrip-font-resolved.pdf')
    with output.open('wb') as f:writer.write(f)
    if output.stat().st_size>32*1024*1024:raise ValueError('resolved_pdf_limit')
    LAST_RESOLUTION={'profile':PROFILE,'snapshotSha256':os.environ['JRIP_FONT_MANIFEST_SHA256'],'sourceSha256':hashlib.sha256(Path(path).read_bytes()).hexdigest(),'preparedSha256':hashlib.sha256(output.read_bytes()).hexdigest(),'fonts':resolved}
    return str(output)
