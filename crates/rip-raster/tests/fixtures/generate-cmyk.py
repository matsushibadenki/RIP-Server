import struct,itertools
be=lambda n:struct.pack('>I',n)
header=bytearray(128)
header[8:12]=bytes([2,0x10,0,0]);header[12:24]=b'prtrCMYKLab ';header[36:40]=b'acsp';header[64:68]=be(1)
header[68:80]=b''.join(be(round(v*65536)) for v in (.9642,1,.8249))
lut=b'mft1'+bytes(4)+bytes([4,3,2,0])+b''.join(be(65536 if i%4==0 else 0) for i in range(9))+bytes(range(256))*4
for c,m,y,k in itertools.product(range(2),repeat=4):
    lut+=bytes([0 if k else round(255*(1-(c+m+y)/4)),128,128])
lut+=bytes(range(256))*3
white=b'XYZ '+bytes(4)+header[68:80]
tags=[(b'A2B0',lut),(b'wtpt',white)]
offset=132+12*len(tags);table=b'';data=b''
for name,payload in tags:
    table+=name+be(offset+len(data))+be(len(payload));data+=payload;data+=bytes((-len(data))%4)
profile=header+be(len(tags))+table+data;profile[:4]=be(len(profile))
open('crates/rip-raster/tests/fixtures/synthetic-cmyk.icc','wb').write(profile)
