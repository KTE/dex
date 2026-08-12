import time, numpy as np, opensimplex
print("numpy", np.__version__)
import opensimplex as osx
print("opensimplex", getattr(osx,"__version__","?"))
osx.seed(7)
for (w,h) in [(240,135),(480,270),(960,540),(1920,1080)]:
    x = np.linspace(0,4,w); y = np.linspace(0,2.25,h)
    z = np.array([0.3]); ww = np.array([0.7])
    t0=time.time(); a = osx.noise4array(x,y,z,ww); t1=time.time()
    print(f"{w}x{h} = {w*h:>9} px -> {t1-t0:8.3f} s  shape={a.shape}  ({(t1-t0)/(w*h)*1e9:.1f} ns/px)")
