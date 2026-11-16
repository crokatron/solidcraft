# SPDX-License-Identifier: Apache-2.0
# Original analytic 10 mm cube; binary field layout from cadmpeg-codec-nx
# 0.6.7 Apache-2.0 synthetic test helpers, https://github.com/cadmpeg/cadmpeg
import struct, zlib, pathlib
s=bytearray(b'PS\0\0\0/: TRANSMIT FILE (partition) created by modeller\0\0\0\x0fSCH_TEST_1_9999\0')
def rec(tag,n,refs=(),vecs=(),real=(),sense=None):
 r=bytearray(n);r[1]=tag
 for at,v in refs:struct.pack_into('>H',r,at,v)
 for at,v in vecs:struct.pack_into('>ddd',r,at,*v)
 for at,v in real:struct.pack_into('>d',r,at,v)
 if sense:r[sense[0]]=ord(sense[1])
 s.extend(r)
p=[(0,0,0),(.01,0,0),(.01,.01,0),(0,.01,0),(0,0,.01),(.01,0,.01),(.01,.01,.01),(0,.01,.01)]
faces=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
normals=[(0,0,-1),(0,0,1),(0,-1,0),(1,0,0),(0,1,0),(-1,0,0)]
edges={}
for f,vs in enumerate(faces):
 for j,a in enumerate(vs):
  b=vs[(j+1)%4];k=tuple(sorted((a,b)))
  edges.setdefault(k,(len(edges),[]))[1].append(100+4*f+j)
rec(12,24,[(2,2)])
rec(13,24,[(2,3),(8,1),(10,2),(12,1),(14,40),(16,1),(18,1),(20,4),(22,1)])
rec(19,16,[(2,4)])
for f,vs in enumerate(faces):
 rec(14,39,[(2,40+f),(18,1 if f==5 else 41+f),(20,1 if f==0 else 39+f),(22,50+f),(24,3),(26,60+f)],real=[(10,1e-8)],sense=(28,'+'))
 rec(15,16,[(2,50+f),(10,100+4*f),(12,40+f),(14,1)])
 direction=tuple((b-a)*100 for a,b in zip(p[vs[0]],p[vs[1]]))
 rec(50,91,[(2,60+f)],[(19,p[vs[0]]),(43,normals[f]),(67,direction)],sense=(18,'+'))
 for j,a in enumerate(vs):
  b=vs[(j+1)%4];ei,uses=edges[tuple(sorted((a,b)))];fid=100+4*f+j
  rec(17,23,[(2,fid),(6,50+f),(8,100+4*f+(j+1)%4),(10,100+4*f+(j+3)%4),(12,300+b),(14,next(v for v in uses if v!=fid)),(16,200+ei),(18,1)],sense=(22,'+' if a<b else '-'))
for (a,b),(ei,uses) in sorted(edges.items()):
 rec(16,32,[(2,200+ei),(18,uses[0]),(24,220+ei)],real=[(10,1e-8)])
 direction=tuple((y-x)*100 for x,y in zip(p[a],p[b]))
 rec(30,67,[(2,220+ei)],[(19,p[a]),(43,direction)],sense=(18,'+'))
for i,pos in enumerate(p):
 rec(18,28,[(2,300+i),(16,310+i)],real=[(18,1e-8)])
 rec(29,40,[(2,310+i)],[(16,pos)])
name=b'/Root/UG_PART/UG_PART';payload=zlib.compress(s,1)
f=bytearray(b'SPLMSSTR\x06'+bytes(16)+b'HEADER'+struct.pack('<II',1,len(name))+name)
f+=struct.pack('<QQ',len(f)+16,len(payload));f+=payload
f[17:23]=struct.pack('<Q',len(f))[:6];f+=b'FOOTER'+bytes(8)
pathlib.Path(__file__).with_name('cube.prt').write_bytes(f)
