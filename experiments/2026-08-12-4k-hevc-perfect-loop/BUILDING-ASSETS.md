# Building bench assets — the full pipeline

How a test card becomes a measurable bench asset, end to end, so a new format
(different resolution, frame rate or duration) can be added without rediscovering
any of it.

Every step below has a reason recorded next to it. Several were learned by getting
them wrong first; those are marked **⚠ learned the hard way** so nobody re-derives
them.

---

## The two artifacts, and why both exist

| Variant | Bitrate (measured) | What it is for |
|---|---|---|
| `dex-test-card-<dur>s-<res><fps>.mp4` | 3.1–8.7 Mbps | **clean** — isolates the wrap |
| `…-clouds.mp4` | 19.6–39.2 Mbps | **clouds + grain** — realistic decoder load |

Run **both** at the bench. If clean passes and clouds fails, the cause is decoder
load rather than loop logic. One asset alone can only tell you *that* something
failed, never *why*.

The card compresses to roughly a tenth of a real artwork's bitrate because it is
flat colour and static geometry. A pass on the clean asset therefore says little
about a 40 Mbps artwork — which is what the clouds variant exists to fix.

---

## Stage 1 — the card, in After Effects

Project: `packages/example-content/test-cards/animation/test-cards v2 4K.aep`

Nine comps: `test-card-{1s,2s,3s}-{1080p,4K-30fps,4K-60fps}`.

**Each comp contains, top to bottom:**

1. the label text layer — `SourceCodePro-Bold`, 64px, fill 235-grey
2. `Shape Layer 1` — the black bar, 1990x104, centred, in the lower third
3. `line spin`
4. the `circle spin timecode <fps>` precomp (ring + timecode)
5. `AltekaKard-4K.png`
6. `AltekaKard-1080p.png` (disabled)

**The label reads `$NAME $WIDTHx$HEIGHT@$FPSfps`** — `4K 3840x2160@60fps`,
`HD 1920x1080@30fps`. `NAME` is `4K` or `HD`.

Character weights: values **bold**, separators regular. Character indices
**7** (`x`), **12** (`@`) and **15–17** (`fps`) get `SourceCodePro-Regular`.

> Every label is exactly **18 characters**, so in a monospace face all variants
> are identical in width and the same indices work everywhere. Keep that property
> when adding a format — it is what makes the layout portable.

**Adding a new format:** duplicate the closest comp, set size/rate/duration, then
copy the label + bar from an existing comp (`layer.copy_to_comp` preserves the
geometry exactly). For a non-4K size, scale both layers proportionally — the
1080p comps use the 4K layers at **50%**, text position `[1036, 976]`, bar
position `[960, 540]`.

**⚠ learned the hard way — layer order.** `copy_to_comp` inserts at index 1, so
copying bar-then-text leaves the *bar on top* and the label invisible. Copy the
text last, or move it above afterwards.

**⚠ learned the hard way — the 360° rule.** A rotation must land exactly on frame
`D`, never `D + 1`. Frame `D` is never rendered — it *is* frame 0 of the next
cycle. Ending later leaves a two-frame jump at the wrap, i.e. the asset carries
the very artifact the experiment measures.

### Rendering the masters

```
AE  →  .mov  (QuickTime RLE / qtrle, the "Lossless" output template)
    →  .mkv  (libx264rgb -qp 0)          ← the committed master
```

```bash
ffmpeg -i test-card-3s-4K-30fps.mov -c:v libx264rgb -qp 0 -an test-card-3s-4K-30fps.mkv
```

**⚠ learned the hard way — `libx264rgb`, not `libx264`.** Plain `libx264` silently
converts RGB to `yuv444p`, which is **not** lossless even at `-qp 0` because the
colour transform rounds. Check `ffprobe … -show_entries stream=pix_fmt`: the
masters must read **`gbrp`**. Verify by decoding one frame from both the `.mov`
and the `.mkv` to rawvideo and `cmp`-ing them.

The `.mov` intermediates are gitignored; the `.mkv` masters are committed.

---

## Stage 2 — the cloud loop, in After Effects

Project: `research/after-effects/looping-cumulus.aep` (comp `clouds`).

One black solid + `Fractal Noise`:

| Property | Value |
|---|---|
| Fractal Type | 1 (Basic) |
| Noise Type | 4 (Spline) |
| Contrast | 550 |
| Brightness | −70 |
| Scale | 700 |
| Complexity | 7 |
| Sub Influence | 65 |
| **Cycle Evolution** | **on** |
| **Cycle (in Revolutions)** | **2** |
| **Evolution** | keyframed **0° → 720°**, both keyframes **LINEAR** |

**What makes it loop:** with `Cycle Evolution` on and `Cycle` = C, the noise
repeats every C revolutions. Animate `Evolution` from 0 to **C × 360°** across the
loop and the last frame equals the first.

**⚠ Both keyframes must be LINEAR.** AE's default easing decelerates into the
wrap: still bit-exact, but visibly slowing and jerking at the loop point — the
failure a bit-compare cannot catch.

### For each duration and rate

The cloud loop must close over **its own** length, so it is re-rendered per
variant — a 90-frame loop truncated to 30 frames does not loop.

For duration `D` seconds at `F` fps:

1. comp size = target size; comp duration = `(D*F + 1) / F` — **one extra frame**
2. layer `outPoint` = comp duration
3. `Evolution` keyframe 2 at time `D`, value `720`, linear
4. render frames `0 … D*F − 1` as a **TIFF sequence** to `f_%04d.tif`

**⚠ learned the hard way — the extra frame.** Changing comp duration does **not**
change the layer's `outPoint`. Leave it and frame `D` renders blank, which reads
as "the loop is broken" when the loop is fine. The extra frame exists only so the
wrap can be bit-compared; **discard it** — keeping it plants a duplicate frame at
the loop point.

**⚠ learned the hard way — layer position.** Changing a comp from 1080p to 4K does
*not* move its layers. A layer left at `[960, 540]` covers only the upper-left of
a 4K frame. Set position to the new centre.

Render is ~11s for 90 frames at 4K.

---

## Stage 3 — compose and encode

One command per card:

```bash
scripts/build-variant.sh \
  --card   ../../packages/example-content/export/lossless/test-card-3s-4K-30fps.mkv \
  --clouds /tmp/clouds-3s-4K-30
```

It produces **both** variants and verifies each. Internally:

1. **Matte** — `make-matte-from-video.mjs` walks every frame of the card and keeps
   a pixel only if it is background (exactly `RGB(211,211,211)`) in **all** of
   them. Anything that moves through a pixel at any point — sweep ring, hands,
   timecode digits — is excluded permanently.

   **⚠ learned the hard way — never key the matte from the static PNG.** The
   artwork's circle outline is r636–642, but the ring animated over it spans
   r628–652. A static matte calls that annulus "background" and paints texture
   over the ring at **every angle**. Measured before/after: 1586 → 2 wrongly
   included ring pixels.

2. **Composite** (lossless ffv1) — clouds over the card through the matte at
   `--opacity 0.85`. Grain is deliberately *not* applied here.

3. **Barcode + encode** — `build-bench-assets.sh` burns the frame-index barcode,
   encodes HEVC with a closed GOP (`keyint=min-keyint=fps`, `scenecut=0`,
   `open-gop=0`), applies `--grain` **at encode time**, and then decodes the
   barcode back out of the finished file, failing loudly on any mismatch.

   Grain belongs at encode because its whole purpose is to defeat compression;
   baking it into the lossless intermediate would bloat that file enormously.

   An **open GOP** would make frame 0 depend on frames that no longer exist at the
   wrap — manufacturing the very seam being measured.

**⚠ learned the hard way — name collision.** `build-bench-assets.sh` derives its
output name from the input's dimensions and duration, so a clean build and a
clouds build of the same card produce the *same* name and the second silently
overwrites the first. `build-variant.sh` exists to give the clouds variant its own
`-clouds` suffix.

**Frame-count guard:** `build-variant.sh` refuses to run if the cloud sequence and
the card differ in frame count, because a cloud loop of a different length does
not close where the card does.

---

## The barcode

Machine-readable frame identity, burned into the **left half of the card's black
bar**; the label occupies the right half.

| | 4K | 1080p |
|---|---|---|
| rect | `1092x92 + 941 + 1884` | `546x46 + 471 + 942` |
| cells | 14 (2 sync + 12 data) | 14 |
| cell width | 78px | 39px |

Geometry lives **only** in `lib/barcode.mjs`, expressed as fractions of the frame,
so both filters are resolution-independent ffmpeg `iw`/`ih` expressions and
`capture.mjs` needs no `--width`/`--height`.

12 data bits cover 4096 frames — 68s at 60fps, far beyond any dex loop. An earlier
version used 16 bits (65,536 frames); the extra range was unreachable, so four
cells were permanently black and every cell was 28% narrower than it needed to be.

Cell 0 is sync **white**, cell 1 sync **black**; decoding thresholds against those
two rather than fixed levels, which is what lets one decoder read a file, an HDMI
capture card, and a camera pointed at a screen.

---

## Verifying a new asset

```bash
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,width,height,avg_frame_rate,nb_frames,bit_rate \
  -of default=nw=1 out/dex-test-card-….mp4

# barcode must decode 0..N-1 exactly
bin/capture.mjs --source out/dex-test-card-….mp4 --out /tmp/idx.txt
```

Expect: `hevc`, the intended geometry, **one keyframe per second**, and a barcode
that decodes exactly. `build-variant.sh` already asserts the barcode; the rest is
worth an eyeball when adding a format.

**Compare pixels, never file bytes.** PNG and TIFF carry metadata that differs
between otherwise identical renders — hashing files reports mismatches where the
pixels are identical.
