import numpy as np, opensimplex as osx, time
from PIL import Image
from scipy.ndimage import map_coordinates

N=180; W,H=960,540
def s4(x,y,a,r,off=0.0):
    return osx.noise4array(x,y,np.array([r*np.cos(a)+off]),np.array([r*np.sin(a)+off]))[0,0]

def fbm(ax,ay,a,oct=4,freq=2.0,lac=2.0,gain=0.5,r=1.0,tgain=1.0,off=0.0,mode="perlin"):
    tot=np.zeros((len(ay),len(ax))); amp=1.0; norm=0.0; f=freq; rr=r
    for k in range(oct):
        n=s4(ax*f,ay*f,a,rr,off+k*31.7)
        n=n/0.6                      # opensimplex range ~[-0.6,0.6] -> ~[-1,1]
        if mode=="billow": n=2.0*np.abs(n)-1.0      # puffy lobes (libnoise billow)
        elif mode=="ridged": n=1.0-2.0*np.abs(n)
        tot+=amp*n; norm+=amp; amp*=gain; f*=lac; rr*=tgain
    return tot/norm

def remap(v,a,b,c,d): return c+(v-a)*(d-c)/(b-a)
def sat(v): return np.clip(v,0,1)

# ---- looping Worley: feature points on a jittered grid, each orbiting a small circle in time
def worley_loop(a, cells=12, W=W, H=H, seedn=3):
    rng=np.random.default_rng(seedn)
    cx,cy=cells, max(1,int(cells*H/W))
    base=rng.random((cy,cx,2)); ph=rng.random((cy,cx,2))*2*np.pi; rad=0.16
    px=(np.arange(cx)[None,:]+base[:,:,0]+rad*np.cos(a+ph[:,:,0]))/cx
    py=(np.arange(cy)[:,None]+base[:,:,1]+rad*np.cos(a+ph[:,:,1]))/cy
    gx=(np.arange(W)+0.5)/W; gy=(np.arange(H)+0.5)/H
    d=np.full((H,W),1e9)
    for oy in(-1,0,1):
        for ox in(-1,0,1):
            fx=(px+ox).ravel(); fy=(py+oy).ravel()
            dx=gx[None,:,None]-fx[None,None,:]; dy=gy[:,None,None]-fy[None,None,:]
            dd=(dx*dx*(W/H)**2+dy*dy).min(axis=2)
            d=np.minimum(d,dd)
    d=np.sqrt(d); return d/d.max()

ax=np.linspace(0,5,W,endpoint=False); ay=np.linspace(0,5*H/W,H,endpoint=False)
osx.seed(12); a=2*np.pi*40/N
t0=time.time()

# domain warp fields (loop exactly -> any composition loops)
wx=fbm(ax,ay,a,oct=2,freq=1.2,off=100.); wy=fbm(ax,ay,a,oct=2,freq=1.2,off=200.)
Y,X=np.mgrid[0:H,0:W]
def warp(field,amt):
    return map_coordinates(field,[Y+wy*amt, X+wx*amt],order=1,mode="reflect")

shape=fbm(ax,ay,a,oct=3,freq=1.6)*0.5+0.5          # LOW freq -> big masses
bil  =sat(fbm(ax,ay,a,oct=4,freq=5.0,mode="billow")*0.5+0.5)
wor  =worley_loop(a,cells=14)
print("fields",round(time.time()-t0,1),"s")

V={}
cov=0.52
# D: low-freq shape + coverage, eroded by billow  (HZD-style)
b=sat(remap(shape,1-cov,1.0,0,1))
V["D_shape+billow_erode"]=sat(remap(b, sat(bil)*0.55, 1.0, 0, 1))
# E: same but domain-warped -> cauliflower
V["E_D+domainwarp"]=sat(remap(sat(remap(warp(shape,14.0),1-cov,1.,0,1)), warp(bil,10.)*0.55,1.,0,1))
# F: Perlin-Worley (HZD): dilate perlin by inverted worley, then coverage
inv_w=1.0-wor
pw=sat(remap(shape, (inv_w-1.0)*0.45, 1.0, 0.0, 1.0))
V["F_perlin-worley"]=sat(remap(pw,1-cov,1.0,0,1))
# G: F + billow erosion + warp
g=sat(remap(warp(pw,12.),1-cov,1.,0,1))
V["G_PW+erode+warp"]=sat(remap(g, warp(bil,9.)*0.5,1.,0,1))
print("shaped",round(time.time()-t0,1),"s")

tiles=[(k,(sat(v)*255).astype(np.uint8)) for k,v in V.items()]
sheet=Image.new("L",(W*2,H*2))
for i,(k,img) in enumerate(tiles): sheet.paste(Image.fromarray(img),((i%2)*W,(i//2)*H))
sheet.save("contact_v2.png"); print([t[0] for t in tiles])
