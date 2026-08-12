import numpy as np, opensimplex as osx, time, os, sys
from scipy.ndimage import map_coordinates
N=180; W,H=960,540
osx.seed(12)
def s4(x,y,a,r,off): return osx.noise4array(x,y,np.array([r*np.cos(a)+off]),np.array([r*np.sin(a)+off]))[0,0]
def fbm(ax,ay,a,oct,freq,off,mode="perlin",r=1.0,tgain=1.0):
    tot=np.zeros((len(ay),len(ax))); amp=1.;norm=0.;f=freq;rr=r
    for k in range(oct):
        n=s4(ax*f,ay*f,a,rr,off+k*31.7)/0.6
        if mode=="billow": n=2*np.abs(n)-1
        tot+=amp*n;norm+=amp;amp*=.5;f*=2;rr*=tgain
    return tot/norm
def remap(v,a,b,c,d): return c+(v-a)*(d-c)/(b-a)
def sat(v): return np.clip(v,0,1)
ax=np.linspace(0,5,W,endpoint=False); ay=np.linspace(0,5*H/W,H,endpoint=False)
Y,X=np.mgrid[0:H,0:W]
os.makedirs("seq",exist_ok=True)
# warm
fbm(ax[:8],ay[:8],0.,1,1.,0.)
t0=time.time(); cov=0.52
for i in range(N):
    a=2*np.pi*i/N
    wx=fbm(ax,ay,a,2,1.2,100.); wy=fbm(ax,ay,a,2,1.2,200.)
    sh=fbm(ax,ay,a,3,1.6,0.)*0.5+0.5
    bl=sat(fbm(ax,ay,a,4,5.0,50.,mode="billow")*0.5+0.5)
    shw=map_coordinates(sh,[Y+wy*14,X+wx*14],order=1,mode="reflect")
    blw=map_coordinates(bl,[Y+wy*10,X+wx*10],order=1,mode="reflect")
    d=sat(remap(sat(remap(shw,1-cov,1.,0,1)), blw*0.55,1.,0,1))
    (d*255).astype(np.uint8).tofile(f"seq/f{i:04d}.gray")
    if i%40==0: print(i, round(time.time()-t0,1),"s",flush=True)
print("TOTAL",round(time.time()-t0,1),"s for",N,"frames at",W,"x",H)
