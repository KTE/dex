# Looping cumulus via Blender EEVEE — research artifacts

**Date:** 2026-08-12 · **Status:** validated, strongest candidate
**Origin:** research agent dispatched after three rounds of ffmpeg `geq` failed to produce cumulus.

`looping_cumulus.py` is **self-contained**: it builds the entire scene from nothing, so the
artifact is a diffable `.py` rather than a binary `.blend`.

```
blender -b -P looping_cumulus.py -- --out ./frames/ --loop 180
```

## Why the loop is exact

Every animated quantity reaches the shader only through `cos(2πT)` and `sin(2πT)` with
`T = frame/180`. Those return to their exact starting values at `T=1`, and Blender's
Noise/Voronoi are pure deterministic functions of their coordinates — so the frame-180 input
vector *is* the frame-0 input vector.

4D is what allows the time path to be a **circle** rather than a back-and-forth along one axis:

```
Vector = ( u + r_d*cos(2πT),  v + r_d*sin(2πT),  r_e*cos(2πT) )
W      =                                          r_e*sin(2πT)
```

Dimensions 3+4 form the morph circle at constant speed (no ping-pong, so no dead stops); the XY
term is a small **circular translation** — drift whose direction slowly rotates, which reads as
weather and never reverses abruptly.

## Verification actually run (M2, Blender 4.5.12 LTS, macOS)

| Test | Result |
|---|---|
| frame 0 vs frame 180, 4K, PNG IDAT sha256 | **identical** |
| frame 1 differs from frame 0 | yes — the animation is real |
| same frame, two separate Blender processes | **bit-identical** |
| mean-abs-diff across the seam (179 -> 0) | **0.001954** vs interior 0.00196 / 0.00193 / 0.00195 |

**The last row is the strongest evidence in this whole effort.** Bit-exactness only proves frame
180 equals frame 0. A loop can be exact and still visibly jolt if the *rate of change* jumps at
the seam. Matching the wrap delta to the interior deltas shows the loop is **C1-smooth** —
continuous in velocity, not just in value. Any future generator should be held to this bar, not
just to a bit-compare.

## Performance

| Config | Time |
|---|---|
| EEVEE Next, **native 4K**, animation mode | **4.38 s/frame -> ~13 min for 180 frames** |
| ^ with `compression=0` | 3.6 s/frame (PNG compression was ~0.8 s of it) |
| Cycles, 4K | >3 min/frame — no reason to pay this for a flat emission plane |
| Volumetric (EEVEE, 960x540, 8 spp) | 27-31 s/frame -> extrapolates to ~24 h at 4K |

Render the whole animation in one process: the first frame carries a ~15-20 s shader-compile
penalty that then amortises.

## Node graph

All Math/MapRange nodes, no groups:

```
UV -> Mapping(scale=(16/9,1,1)) -> SeparateXYZ
T -> *2pi -> COSINE / SINE -> CombineXYZ(x + r*cos, y + r*sin, r_e*cos), W = r_e*sin

base   = Noise4D(COORD,W)  Scale 1.4  Detail 8  Rough .55  Lacunarity 2.05  Distortion .35
billow = 1 - 1.5*Voronoi4D(COORD,W, SMOOTH_F1, Scale 3.0, Smoothness .35)   [clamped]
detail = Noise4D(COORD,W)  Scale 7  Detail 6  Rough .62

shape  = 0.5*base + 0.5*billow
eroded = shape - 0.30*(detail - 0.5)                        <- Nubis-style erosion, cauliflower edges
alpha  = MapRange(eroded, .36 -> .405, SMOOTHSTEP, clamp)   <- crisp silhouette
core   = MapRange(eroded, .36 -> .56,  SMOOTHSTEP, clamp)   <- soft bright interior
out    = mix(0.45 grey, 0.55 + 0.45*core, alpha) -> Emission
```

The **two-width remap** — narrow for the silhouette, wide for the interior — is what makes it
read as puffy 3D rather than a flat stencil, at no extra cost.

Knobs: `--thr` coverage (lower = more cloud), `--sbase` mass size, `--sdet` / `--wdet` edge
lumpiness. For bitrate, raise `--sbase` and `--sdet` — finer structure carries more entropy.

## The drift-vs-tiling constraint

Worth knowing before anyone asks for "more drift". For an exactly-looping *translation* of a
spatial pattern, the drift distance per loop must equal a spatial period of that pattern. So
slow drift implies a small period implies visible tiling. Over a 3-second loop you cannot have
both slow linear drift and no repetition — the "animate a Mapping offset by exactly one tile"
approach forces at least one screen-width of travel per loop, far too fast to read as weather.

The **circular** XY translation sidesteps this entirely: constant speed, exactly periodic, no
tiling period at all.

## Pitfalls and version traps

- **Pin Blender 4.5.x.** Blender **5.0 replaced the Voronoi/noise hash** (Jenkins lookup3 ->
  PCG3D): "behaves the same, but the literal pattern has changed." A 4.5 render will not
  reproduce on 5.x. `brew install --cask blender` installs latest — pin deliberately.
- **Musgrave no longer exists** — merged into Noise Texture in 4.1. Any tutorial adding a
  Musgrave node predates that.
- **EEVEE headless on macOS works** in 4.5.12 despite older docs claiming Linux-only since 3.4.
  That guidance is stale.
- **Cycles CPU and GPU do not match bit-for-bit** (Blender #101561). Irrelevant for EEVEE, but
  never split one render across devices.
- For Cycles specifically, set `cycles.use_animated_seed = False` so render noise is a *static*
  pattern identical on every frame — the inverse of the usual animation advice, and required
  for the loop to survive path tracing.
- Blender 5.0's compositor gained procedural texture nodes (Noise, Voronoi, Gabor); 4.5 has
  none of these, verified by `bpy.types` introspection.

## Volumetric approach — why it was rejected

`clouds_vol.py`. It loops correctly, but a volume needs 3 spatial dimensions and Blender's noise
tops out at 4D, leaving only W for time — and a 1D time path cannot be a closed circle.
`w = r*sin(2πt)` does close, but it ping-pongs: plays forward then backward with two dead stops.
The agent's fix was to make the *coverage* field 2D in `(x,y)`, freeing dimensions 3+4 for a
genuine circle, with vertical structure from an analytic height profile plus static 3D erosion
noise. That works and loops exactly — but at ~24 h for 180 frames at 4K it is not worth it for
what is ultimately a flat overlay. The look also came out wrong (a black slab), so its 30 s/frame
is a valid *cost* measurement but not a valid *quality* one.

## Note on a misleading source

A [BlenderArtists thread](https://blenderartists.org/t/how-to-loop-procedural-noise-seamlessly/1337153)
on this technique calls the result "approximate". That is **wrong about the loop**. What is
approximate there is the *uniformity of apparent motion* — their setup folds object Z into the
amplitude, so the front evolves faster than the back. Loop closure is exact whenever the input
path is closed, which the sha256 tests confirm.

## Sources

- <https://docs.blender.org/manual/en/latest/render/shader_nodes/textures/noise.html>
- <https://developer.blender.org/docs/release_notes/4.1/nodes_physics/>
- <https://aras-p.info/blog/2025/06/13/Voronoi-Hashing-and-OSL/>
- <https://developer.blender.org/docs/release_notes/5.0/compositor/>
- <https://www.simonaa.media/tutorials/looping-noise-part-1>
