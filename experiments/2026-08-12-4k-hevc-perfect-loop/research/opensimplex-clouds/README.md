# Looping cumulus via 4D OpenSimplex — research artifacts

**Date:** 2026-08-12 · **Status:** validated, not yet adopted
**Origin:** research agent dispatched after three rounds of ffmpeg `geq` failed to produce cumulus.

These files are preserved research, **not** part of the harness. They were produced in an
ephemeral scratchpad and kept here because they are measured results that would be expensive
to reproduce.

## Why the ffmpeg approach failed, and why this one works

`scripts/make-clouds.sh` sums sinusoids at integer frequencies. That gives an exact loop, but
summed sinusoids are **inherently directional**, so three rounds produced diagonal moiré, then
liquid marble, then elongated streaks when thresholded. Real Perlin/simplex noise is
**isotropic** because it is built on a randomised lattice.

The fix is to sample **4D OpenSimplex on a time-circle**: two spatial dimensions plus two
tracing a closed circle, `n(x·f, y·f, r·cos(2πi/N), r·sin(2πi/N))`. The loop is then exact
*structurally* rather than by tuning — and isotropy comes for free. `view_R1_f0.png` shows the
result: separated puffy masses with cauliflower edges, no directional artefact.

## Measured results (agent's runs, M2 Mac)

Generation, 180-frame loop:

| Render strategy | Time |
|---|---|
| 960x540 then Lanczos upscale to 4K | **202.6 s** |
| native 4K | 174 s/frame — **8.7 hours** |

Clouds have no pixel-scale detail, so the upscale costs nothing visually. Fine detail comes
from the grain layer instead.

Bitrate, 3840x2160 60 fps, HEVC **CRF 20 `veryfast`** (see caveat below):

| Layer | Bitrate |
|---|---|
| flat grey baseline | 0.15 Mbps |
| clouds only, 20% amplitude | 1.76 |
| clouds only, full amplitude | 5.97 |
| clouds 35% + grain `alls=2` | 2.99 |
| clouds 35% + grain `alls=4` | 3.34 |
| clouds 35% + grain `alls=6` | **11.33** |
| clouds 35% + grain `alls=9` | **118.07** |
| clouds 20% + grain `alls=12` | 294.98 |

**Clouds cannot carry the bitrate** — even at full amplitude they reach ~6 Mbps against a
20 Mbps target, because they are low-frequency by construction and HEVC eats them. The grain
layer does essentially all of it, and the knob is violently non-linear: `alls` 6 -> 9 spans
11 -> 118 Mbps.

> **These numbers are not ours.** They were measured at CRF 20 `veryfast`; the dex pipeline
> uses `-b:v` ABR at libx265's default `medium`, which lands lower for the same `alls`. With a
> curve this steep, borrowing the number would miss by an order of magnitude. Sweep it against
> the real encoder settings and record the curve, not a single value.

## Recipe that produced cumulus

1. **Low-frequency base**, 3 octaves. Six octaves gives wispy mush — stratus, not cumulus.
2. **Coverage remap** at ~0.52 (Horizon Zero Dawn style): `sat(remap(shape, 1-cov, 1, 0, 1))`.
   This is what creates *separation* — discrete masses with clear sky between them.
3. **Billow erosion** `2*|n|-1` for cauliflower edges. **Watch the sign** — inverted it gives
   ridged/veiny networks instead of puffy lobes.
4. **Domain warp** via `scipy.ndimage.map_coordinates` on a *looping* warp field. Because the
   warp loops, the composition loops.

## Two pitfalls that silently break the loop

- **Do not scale the time-circle radius per octave.** If radius tracks octave frequency, high
  octaves churn faster than low ones and you get exactly the per-frame flicker the cloud layer
  exists to avoid. Keep radius constant across octaves.
- **Render `i = 0..N-1`.** Frame N is never rendered because it *is* frame 0 of the next
  cycle. Render N+1 to bit-compare the wrap, then discard the extra — keeping it plants a
  duplicate frame at the loop point, i.e. the exact defect being measured.

## Files

| File | What |
|---|---|
| `gen_seq.py` | full 180-frame generator |
| `cumulus2.py` | shaping variants (coverage, billow, warp) |
| `wrapcheck.py` | numerical loop verifier — compares wrap step against interior distribution |
| `bitrate_test.sh` | encode sweep |
| `bench1-3.py` | noise library performance benchmarks |
| `view_R1_f0.png` | **the result** — frame 0 of the recommended recipe |
| `contact_v2.png`, `H_three.png` | variant comparisons |

Dependencies: `numpy`, `opensimplex`, `scipy`, `PIL`, `perlin-numpy`. Python is justified here
under the workspace rule (wrapping numerical libraries with no Node equivalent), unlike the
harness itself which is dependency-free `.mjs`.

## Loop verification method

Worth reusing: rather than only bit-comparing frame N to frame 0, `wrapcheck.py` compares the
**wrap step magnitude against the distribution of interior frame-to-frame steps**. A correct
loop gives a wrap step inside the normal range (measured: 1.22 against an interior range of
0.96-2.05); the non-tileable control scored 14.1x. This catches a loop that is *approximately*
closed, which a bit-compare would also catch but a visual check would not.

## Alternatives evaluated

- **perlin-numpy `generate_fractal_noise_3d(tileable=(True,False,False))`** — verified to give
  an exact loop with only 3D noise, simpler maths. Costs: whole volume in RAM (4K x 180 ~ 6 GB
  float32) and `shape` must be divisible by `res*2^(octaves-1)`.
- **GLSL via moderngl headless** — `stegu/webgl-noise` provides `pnoise(vec3 x, vec3 period)`,
  the GLSL analogue; set `period.z` to the loop length. GPU would make native 4K trivial. Not
  built.
- **glslViewer** — evidence contradictory on headless output flags. Verify before relying on it.
- **FastNoiseLite** has no 4D mode (2D/3D only). `pyfastnoisesimd` is x86 SIMD, likely
  unavailable on Apple Silicon.

## Sources

- <https://github.com/Dennis-van-Gils/opensimplex-loops>
- <https://bleuje.com/tutorial3/>
- <https://www.guerrilla-games.com/read/the-real-time-volumetric-cloudscapes-of-horizon-zero-dawn>
- <https://github.com/stegu/webgl-noise>
- <https://github.com/pvigier/perlin-numpy>
- <https://thebookofshaders.com/12/>
- <https://shadergif.com/guides/how-to-make-a-perfect-loop/>
