import sys, pathlib, subprocess, hashlib, os
sys.path.append('/usr/local/lib/jrip')
import font_prepare as fp
from font_policy import inspect_fonts
from pypdf import PdfWriter
from pypdf.generic import DictionaryObject as D, NameObject as N, NumberObject as I, ArrayObject as A, DecodedStreamObject
fonts=fp.registry(); name=next(iter(fonts)); font=fonts[name]
w=PdfWriter();page=w.add_blank_page(200,100)
f=D({N('/Type'):N('/Font'),N('/Subtype'):N('/TrueType' if font['format']=='ttf' else '/Type1'),N('/BaseFont'):N('/'+name),N('/Encoding'):N('/WinAnsiEncoding'),N('/FirstChar'):I(0),N('/LastChar'):I(255),N('/Widths'):A([I(600) for _ in range(256)])})
page[N('/Resources')]=D({N('/Font'):D({N('/F1'):w._add_object(f)})})
s=DecodedStreamObject();s.set_data(b'BT /F1 24 Tf 20 40 Td (AB) Tj ET');page[N('/Contents')]=w._add_object(s)
path='/tmp/source.pdf';w.write(path);original=pathlib.Path(path).read_bytes()
try:inspect_fonts(path)
except ValueError:pass
else:raise AssertionError('unregistered missing font accepted')
prepared=fp.prepare_pdf(path);assert prepared!=path;inspect_fonts(prepared)
assert pathlib.Path(path).read_bytes()==original
assert fp.LAST_RESOLUTION['sourceSha256']==hashlib.sha256(original).hexdigest()
engine=sys.argv[1]
if engine=='mupdf':
    cmd=['mutool','draw','-q','-r','72','-o','/tmp/page.png',prepared]
    # Verify script argument handling in the actual pinned MuPDF build.
    result=subprocess.run(['mutool','run','/usr/local/lib/jrip/inspect.js',prepared],capture_output=True)
    assert result.returncode==0,result.stderr
else:
    assert fp.verify_postscript_registry()
    cmd=['gs','-q','-dBATCH','-dNOPAUSE','-dSAFER','-sDEVICE=png16m','-sOutputFile=/tmp/page.png',prepared]
result=subprocess.run(cmd,capture_output=True);assert result.returncode==0,result.stderr
assert pathlib.Path('/tmp/page.png').stat().st_size>100
if engine=='ghostscript':
    ps=pathlib.Path('/tmp/source.ps');ps.write_text('%!PS\n/'+name+' findfont 24 scalefont setfont 20 40 moveto (AB) show\n/Helvetica findfont 10 scalefont setfont 20 20 moveto (builtin) show showpage\n')
    result=subprocess.run(['gs','-q','-dBATCH','-dNOPAUSE','-dSAFER','-dNOFONTPATH','-dNOPLATFONTS','-dNONATIVEFONTMAP','-sFONTMAP=/jrip-fonts/Fontmap','-sDEVICE=png16m','-sOutputFile=/tmp/ps.png',str(ps)],capture_output=True)
    assert result.returncode==0,result.stderr
    assert pathlib.Path('/tmp/ps.png').stat().st_size>100
print(engine,'registered PDF / exact name / original preserved / PS builtin OK')

pathlib.Path('/input/document').write_bytes(original)
python='/opt/jrip-python/bin/python'
if engine=='mupdf':
    args=[python,'-I','/usr/local/lib/jrip/render.py','72','rgb','2']
    check=subprocess.run([python,'-I','/usr/local/lib/jrip/inspect.py'],capture_output=True)
    assert check.returncode==0,check.stderr
    assert b'registeredResolution' in check.stdout
else:
    args=[python,'-I','/usr/local/lib/jrip/render.py','-dSAFER','-dBATCH','-dNOPAUSE','-dQUIET','-dNOFONTPATH','-dNOPLATFONTS','-dNONATIVEFONTMAP','-dPDFNOCIDFALLBACK','-dPDFSTOPONERROR','-I/usr/local/lib/jrip/strict-resources','-sSUBSTFONT=JRIP-Missing-Font-Rejected','-sstdout=%stderr','-sDEVICE=tiff24nc','-r72','-dLastPage=2','-sOutputFile=/output/page-%06d.tiff','--jrip-format=pdf','-f','/input/document']
check=subprocess.run(args,capture_output=True)
assert check.returncode==0,check.stderr
assert check.stdout.startswith(b'JRIP-PAGES-1\n') and len(check.stdout)>500
print('full bounded adapter passed')
