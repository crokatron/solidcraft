# Original analytic 10 mm cube. Based solely on base-schema fields in Apache2 cadmpeg.
import struct, pathlib
root=pathlib.Path(__file__).parent
nodes=[]
def I(v):return ('d',v)
def P(v):return ('p',v)
def R(v):return ('f',v)
def C(v):return ('c',v)
def B(v):return ('u',v)
def V(v):return ('v',v)
def node(kind,idx,*fields):nodes.append((kind,idx,fields))
N=-31415800000000.0
verts=[(0,0,0),(1,0,0),(1,1,0),(0,1,0),(0,0,1),(1,0,1),(1,1,1),(0,1,1)]
verts=[tuple(x*.01 for x in v) for v in verts]
faces=[([0,3,2,1],(0,0,-1),(0,1,0)),([4,5,6,7],(0,0,1),(1,0,0)),([0,1,5,4],(0,-1,0),(1,0,0)),([3,7,6,2],(0,1,0),(0,0,1)),([0,4,7,3],(-1,0,0),(0,0,1)),([1,2,6,5],(1,0,0),(0,1,0))]
edges={}; fins=[]
for f,(vs,_,_) in enumerate(faces):
 for j in range(4):
  a,b=vs[j],vs[(j+1)%4]; key=tuple(sorted((a,b)))
  if key not in edges:edges[key]=[]
  edges[key].append(len(fins)); fins.append((f,j,a,b,key))
node(12,2,I(200),P(1),P(1),P(1),P(1),P(1),P(1),R(1000),R(1e-8),P(1),P(1),P(1),B(1),P(1),B(1),B(0),P(4),P(1),P(1),P(1),P(3),P(1),P(1))
node(19,3,I(3),P(1),P(2),P(1),P(1),P(4),C('S'))
node(13,4,I(4),P(1),P(2),P(1),P(10),P(1),P(1),P(3),P(1))
for f,(vs,normal,xaxis) in enumerate(faces):
 node(14,10+f,I(10+f),P(1),R(N),P(10+f+1 if f<5 else 1),P(10+f-1 if f else 1),P(20+f),P(4),P(30+f),C('+'),P(1),P(1),P(1),P(1),P(1))
 node(15,20+f,I(20+f),P(1),P(40+4*f),P(10+f),P(1))
 node(50,30+f,I(30+f),P(1),P(10+f),P(1),P(1),P(1),C('+'),V(verts[vs[0]]),V(normal),V(xaxis))
keys=list(edges)
for q,(f,j,a,b,key) in enumerate(fins):
 other=next(x for x in edges[key] if x!=q); edge=keys.index(key)
 node(17,40+q,P(1),P(20+f),P(40+f*4+(j+1)%4),P(40+f*4+(j-1)%4),P(100+b),P(40+other),P(70+edge),P(1),P(1),C('+' if a==key[0] else '-'))
for k,(a,b) in enumerate(keys):
 first=next(q for q in edges[(a,b)] if fins[q][2]==a)
 node(16,70+k,I(70+k),P(1),R(N),P(40+first),P(1),P(1),P(120+k),P(1),P(1),P(2))
 direction=tuple((verts[b][i]-verts[a][i])/.01 for i in range(3))
 node(30,120+k,I(120+k),P(1),P(70+k),P(1),P(1),P(1),C('+'),V(verts[a]),V(direction))
for k,v in enumerate(verts):
 fin=next(i for i,x in enumerate(fins) if x[3]==k)
 node(18,100+k,I(100+k),P(1),P(40+fin),P(1),P(1),P(110+k),R(N),P(2))
 node(29,110+k,I(110+k),P(1),P(100+k),P(1),P(1),V(v))
desc=': TRANSMIT FILE created by modeller version 1300000'; schema='SCH_1300000_13006'
b=bytearray(b'PS'+struct.pack('>I',len(desc))+desc.encode()+struct.pack('>I',len(schema))+schema.encode()+struct.pack('>I',0))
t=[f'T{len(desc)} {desc}{len(schema)} {schema}0 ']
for kind,idx,fields in nodes:
 b+=struct.pack('>HH',kind,idx); t.append(f'{kind} {idx-1} ')
 for typ,val in fields:
  if typ=='d':b+=struct.pack('>I',val); t.append(str(val)+' ')
  if typ=='p':b+=struct.pack('>H',val); t.append(str(val-1)+' ')
  if typ=='f':b+=struct.pack('>d',val); t.append('?' if val==N else repr(val)+' ')
  if typ=='c':b+=val.encode(); t.append(val)
  if typ=='u':b+=bytes([val]); t.append(str(val)+' ')
  if typ=='v':
   b+=struct.pack('>ddd',*val); t.append(' '.join(map(str,val))+' ')

b+=b'\x00\x01\x00\x01'; t.append('1 0 ')
(root/'cube10.x_b').write_bytes(b); (root/'cube10.x_t').write_text(''.join(t))
print(len(nodes),len(b))



