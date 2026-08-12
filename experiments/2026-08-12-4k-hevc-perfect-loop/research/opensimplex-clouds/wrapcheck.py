import numpy as np, glob
f=sorted(glob.glob("seq/*.gray")); H,W=540,960
a=np.stack([np.fromfile(p,np.uint8).reshape(H,W).astype(np.int16) for p in f])
d=[np.abs(a[(i+1)%180]-a[i]).mean() for i in range(180)]
print(f"frames {a.shape}")
print(f"wrap step (179->0): {d[179]:.4f}")
print(f"mean interior step: {np.mean(d[:179]):.4f}  min {np.min(d[:179]):.4f}  max {np.max(d[:179]):.4f}")
print(f"wrap/mean ratio: {d[179]/np.mean(d[:179]):.4f}   (1.0 = perfect loop)")
print(f"max per-pixel |f179-f0| = {np.abs(a[179]-a[0]).max()} (8-bit levels)")
