import time, numpy as np
from perlin_numpy import generate_perlin_noise_3d, generate_fractal_noise_3d
T,H,W = 180,270,480
t0=time.time()
v = generate_perlin_noise_3d((T,H,W),(6,5,8), tileable=(True,False,False)); t1=time.time()
print("1 octave 180x270x480:", round(t1-t0,3),"s")
# EXACTNESS: frame that would follow the last is frame 0. Generate T+1 grid?
# Instead: continuity test - jump from last->first vs typical frame-to-frame
d_wrap = np.abs(v[0]-v[-1]).mean()
d_typ  = np.mean([np.abs(v[i+1]-v[i]).mean() for i in range(0,T-1)])
print(f"  wrap step {d_wrap:.6f} vs typical {d_typ:.6f} ratio {d_wrap/d_typ:.4f}")
# non-tileable control
v2 = generate_perlin_noise_3d((T,H,W),(6,5,8), tileable=(False,False,False))
print(f"  NON-tileable control ratio {np.abs(v2[0]-v2[-1]).mean()/np.mean([np.abs(v2[i+1]-v2[i]).mean() for i in range(T-1)]):.4f}")
