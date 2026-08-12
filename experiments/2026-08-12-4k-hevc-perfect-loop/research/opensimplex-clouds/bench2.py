import time, numpy as np, opensimplex as osx
osx.seed(7)
# warm JIT
osx.noise4array(np.linspace(0,1,8), np.linspace(0,1,8), np.array([0.]), np.array([0.]))

# 4K single octave
W,H = 3840,2160
x = np.linspace(0,8,W); y=np.linspace(0,4.5,H)
t0=time.time(); a=osx.noise4array(x,y,np.array([0.3]),np.array([0.7])); t1=time.time()
per=t1-t0
print(f"4K single octave: {per:.3f} s  -> 180 frames x 5 octaves = {per*180*5/60:.1f} min")
print("dtype",a.dtype,"nbytes",a.nbytes/1e6,"MB range",a.min(),a.max())

# reduced res
for (w,h) in [(480,270),(960,540)]:
    xx=np.linspace(0,8,w); yy=np.linspace(0,4.5,h)
    t0=time.time()
    for _ in range(5): osx.noise4array(xx,yy,np.array([0.3]),np.array([0.7]))
    t1=time.time()
    print(f"{w}x{h} 5 octaves: {t1-t0:.3f} s/frame -> 180 frames = {(t1-t0)*180:.1f} s")
