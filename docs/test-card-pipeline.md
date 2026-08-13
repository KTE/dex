# Building dex test cards and bench assets

*How a test card becomes a video file you can measure a player with — and why each
step is the way it is.*

---

## What this is for

dex plays a video on a loop, forever, in a gallery. The question that matters is
whether the loop is **seamless**: no black frame, no held frame, no dropped frame
at the point where the last frame gives way to the first. That question is
surprisingly hard to answer by looking, which is why dex carries a purpose-built
test card and a measurement harness rather than an artwork and an opinion.

This article describes the pipeline that turns the After Effects test card into a
**bench asset**: a video file a player can loop and an analyzer can measure. It
exists so a new format — a different resolution, frame rate or duration — can be
added without rediscovering the reasoning, and without repeating the mistakes.

Those mistakes are the valuable part. Most of them share a shape: a step that
looks correct, produces a plausible file, and is quietly wrong. They are collected
in [Failure modes](#failure-modes) at the end, and referenced from the stages
where they bite.

---

## Two assets per card, not one

Every card produces two bench assets:

| Variant | Bitrate | Purpose |
|---|---|---|
| `dex-test-card-<dur>s-<res><fps>.mp4` | 3–9 Mbps | **clean** — isolates the wrap |
| `…-clouds.mp4` | 20–39 Mbps | **clouds + grain** — realistic decoder load |

The reason for two is diagnostic. The test card is flat colour and static
geometry, so it compresses to roughly a tenth of what real video artwork
produces. A player that loops it perfectly has been asked a much easier question
than a gallery will ask: the decoder spends the whole time idling.

The clouds variant fixes that by adding texture until the bitrate is realistic.
But it would be a mistake to *only* test the loaded case, because then a failure
is ambiguous — was it the loop logic, or the decoder running out of headroom?

Running both resolves that. If clean passes and clouds fails, the cause is
decoder load. One asset can tell you *that* something failed; two tell you
*which thing*.

---

## Stage 1 — The card

**Project:** `packages/example-content/test-cards/animation/test-cards v2 4K.aep`

Nine compositions: `test-card-{1s,2s,3s}-{1080p,4K-30fps,4K-60fps}`.

The card is the Alteka Kard artwork with animation layered on top: a rotating
sweep ring, a burned-in timecode, and — added later — a black bar carrying a
human-readable label.

### The label

Each card states what it is:

```
4K 3840x2160@60fps        HD 1920x1080@30fps
```

This exists because comparing a 4K file against an HD one on screen, they are
indistinguishable. The barcode (below) is machine-readable identity; the label is
the human one. `NAME` is `4K` or `HD`; the rest is literal.

Values are **bold**, separators regular: `x`, `@` and `fps` use
`SourceCodePro-Regular`, everything else `SourceCodePro-Bold`. In character
indices that is 7, 12 and 15–17.

Those indices work in every variant because **every label is exactly 18
characters**, and the face is monospace. Preserve that property when adding a
format — it is what lets one layout serve all of them, and it means a new card
needs no repositioning at all.

### Adding a comp

Duplicate the closest existing comp, set its size, frame rate and duration, then
copy the label and bar across — copying preserves geometry exactly, where
recreating it invites drift. For a non-4K size, scale both proportionally: the
1080p comps use the 4K layers at **50%**, with text at `[1036, 976]` and the bar
at `[960, 540]`.

Watch the [layer order](#the-bar-lands-on-top-of-its-own-label) and the
[360° rule](#a-rotation-that-ends-one-frame-late).

### Rendering masters

```
After Effects  →  .mov   QuickTime RLE (qtrle), via the "Lossless" template
               →  .mkv   libx264rgb -qp 0        ← the committed master
```

```sh
ffmpeg -i test-card-3s-4K-30fps.mov -c:v libx264rgb -qp 0 -an test-card-3s-4K-30fps.mkv
```

The `.mov` intermediates are gitignored; the `.mkv` masters are committed. Use
`libx264rgb`, not `libx264` — see [the lossless trap](#qp-0-that-is-not-lossless).

---

## Stage 2 — The cloud loop

**Project:** `experiments/2026-08-12-4k-hevc-perfect-loop/research/after-effects/looping-cumulus.aep`

The texture that carries the bitrate is a looping cumulus field: one black solid
with a single `Fractal Noise` effect.

| Property | Value | |
|---|---|---|
| Fractal Type | 1 | Basic |
| Noise Type | 4 | Spline |
| Contrast | 550 | with Brightness, this is the coverage remap |
| Brightness | −70 | separates masses; sinks the gaps to black sky |
| Scale | 700 | mass size |
| Complexity | 7 | cauliflower edge detail |
| Sub Influence | 65 | how lumpy those edges are |
| Cycle Evolution | **on** | |
| Cycle (in Revolutions) | **2** | |
| Evolution | **0° → 720°** | two keyframes, both **linear** |

### Why it loops

With `Cycle Evolution` enabled and `Cycle` set to C, the noise pattern repeats
every C revolutions of `Evolution`. Animate `Evolution` from 0 to **C × 360°**
across the loop and the last frame is identical to the first — verified
pixel-exact, not approximately.

Both keyframes must be **linear**. AE's default easing decelerates into the wrap,
which remains bit-exact while visibly slowing and jerking at the loop point. A
bit-compare cannot catch that; see [the seam that is exact and still
visible](#a-loop-that-is-exact-and-still-jerks).

### One render per duration

The cloud loop must close over *its own* length, so it is rendered separately for
each variant. A 90-frame loop truncated to 30 frames does not loop.

For duration `D` seconds at `F` fps:

1. Set the comp to the target size; duration `(D×F + 1) / F` — **one frame longer
   than needed**.
2. Set the layer's `outPoint` to match the comp duration.
3. Move `Evolution` keyframe 2 to time `D`, value `720`, linear.
4. Render frames `0 … D×F − 1` as a TIFF sequence, `f_%04d.tif`.

The extra frame exists so the wrap can be bit-compared against frame 0 — and is
then **discarded**. Keeping it plants a duplicate frame at the loop point, which
is the very defect being measured.

Roughly 11 seconds for 90 frames at 4K.

---

## Stage 3 — Compose and encode

One command per card:

```sh
scripts/build-variant.sh \
  --card   packages/example-content/export/lossless/test-card-3s-4K-30fps.mkv \
  --clouds /tmp/clouds-3s-4K-30
```

It produces both variants and verifies each. Three things happen inside.

### The matte

Texture must land only on the grey background grid — never on the circles, bars,
wedges or colour wheels, because those are the card's measurement elements and
texturing them corrupts what they measure.

The matte is generated by walking **every frame** of the card and keeping a pixel
only if it is background — exactly `RGB(211, 211, 211)` — in *all* of them.
Anything that moves through a pixel at any point in the loop excludes it
permanently.

That is deliberately conservative. Failing to texture a few background pixels
costs nothing; texturing a measurement element costs the measurement. And it must
be built from the animation, not the artwork — see
[the matte that was blind to the animation](#a-matte-built-from-the-wrong-thing).

### The composite

Clouds are blended over the card through the matte at 85% opacity, into a
lossless ffv1 intermediate. Grain is deliberately **not** applied here.

### The encode

The barcode is burned in, then HEVC with a closed GOP: `keyint = min-keyint =
fps`, `scenecut=0`, `open-gop=0`. Grain is applied at this point, not earlier,
because its entire purpose is to defeat compression — baking it into a lossless
intermediate would bloat that file enormously for no benefit.

The GOP settings are load-bearing. An **open GOP** would make frame 0 depend on
frames that no longer exist at the wrap, manufacturing the exact seam the
experiment is trying to detect.

Finally the encoder's own output is decoded again and the barcode read back. A
silent barcode failure would poison every measurement taken with that asset, so
it is checked rather than assumed.

---

## The barcode

Machine-readable frame identity, burned into the **left half** of the card's black
bar; the label occupies the right half.

| | 4K | 1080p |
|---|---|---|
| Rectangle | `1092×92 + 941 + 1884` | `546×46 + 471 + 942` |
| Cells | 14 — 2 sync + 12 data | 14 |
| Cell width | 78 px | 39 px |

Cell 0 is sync **white**, cell 1 sync **black**, and cells 2–13 carry the frame
index with the least significant bit first. Decoding thresholds against the two
sync cells rather than against fixed levels, which is what lets one decoder read a
file, an HDMI capture card, and a camera pointed at a screen — all three shift and
compress the level range differently.

Twelve data bits cover 4096 frames: 68 seconds at 60 fps, far beyond any dex loop.
An earlier version used sixteen, and since a loop is around 90 frames the top four
bits were permanently zero — four dead cells, and every remaining cell 28%
narrower than it needed to be.

Geometry lives in exactly one place, `lib/barcode.mjs`, expressed as fractions of
the frame. Both the burn and decode filters are therefore resolution-independent
ffmpeg `iw`/`ih` expressions, and the capture tool needs no resolution flag at
all. A burner and decoder that drifted apart would fail silently, so they share a
single definition by construction.

---

## Adding a new format

1. **Card** — duplicate the nearest comp in AE, set size/rate/duration, copy the
   label and bar across, update the label text, scale if not 4K.
2. **Masters** — render to `.mov`, transcode with `libx264rgb -qp 0` to `.mkv`.
3. **Clouds** — set the cumulus comp to the same size, rate and duration, move the
   `Evolution` keyframe, render `D×F` frames plus one.
4. **Assets** — `scripts/build-variant.sh --card … --clouds …`.
5. **Verify** — see below.

`build-variant.sh` refuses to run if the cloud sequence and the card differ in
frame count, because a cloud loop of a different length does not close where the
card does.

---

## Verifying

```sh
ffprobe -v error -select_streams v:0 \
  -show_entries stream=codec_name,width,height,avg_frame_rate,nb_frames,bit_rate \
  -of default=nw=1 out/dex-test-card-….mp4

bin/capture.mjs --source out/dex-test-card-….mp4 --out /tmp/idx.txt
```

Expect `hevc`, the intended geometry, **one keyframe per second**, and a barcode
decoding `0 … N−1` exactly. The build script already asserts the barcode; the rest
is worth an eyeball when adding a format.

---

## Failure modes

Every one of these produced a plausible-looking file. That is what makes them worth
writing down.

### `-qp 0` that is not lossless

`libx264 -qp 0` on RGB source silently converts to `yuv444p`, and the colour
transform rounds. "qp 0" reads as proof of losslessness, so nothing appears wrong.

Use **`libx264rgb`**. Check `ffprobe -show_entries stream=pix_fmt` — the masters
must read `gbrp`. Verify by decoding one frame from both the `.mov` and the `.mkv`
and comparing them byte for byte.

### A matte built from the wrong thing

The artwork's circle outline spans radius 636–642, but the ring animated over it
in AE spans 628–652 — wider on both sides. A matte keyed from the static PNG
therefore calls that annulus "background" and paints texture over the ring, at
every angle, in every asset built from it.

Measured before and after the fix: **1586 → 2** ring pixels wrongly included.

A static matte cannot describe an animated card. Build it from the animation.

### A rotation that ends one frame late

A rotation must land exactly on frame `D`, never `D + 1`. Frame `D` is never
rendered — it *is* frame 0 of the next cycle. Ending later leaves a two-frame jump
at the wrap: the asset carries the artifact the experiment exists to detect.

### A loop that is exact and still jerks

Bit-exactness proves frame N equals frame 0. It does not prove the *rate of
change* is continuous across the wrap. An eased keyframe decelerates into the loop
point and then jumps back to full speed — bit-exact, visibly wrong.

Test it by comparing the per-frame change **across the wrap** against the
distribution of interior frame-to-frame changes. They should be indistinguishable.

### A layer that stops before the comp does

Changing a comp's duration does not change its layers' `outPoint`. Leave it and
the final frame renders blank — which reads as "the loop is broken" when the loop
is fine.

The same applies to size: changing a comp from 1080p to 4K does **not** move its
layers. A layer left at `[960, 540]` covers only the upper-left of a 4K frame.

### The bar lands on top of its own label

Copying layers between comps inserts at the top of the stack, so copying the bar
after the text leaves the black bar painting over the label. Copy the text last,
or reorder afterwards.

### Two builds, one filename

The asset builder derives its output name from the input's dimensions and
duration — so a clean build and a clouds build of the same card produce the *same*
name, and the second silently overwrites the first. `build-variant.sh` exists to
give the clouds variant its own suffix.

### Comparing files instead of pixels

PNG and TIFF carry metadata that differs between otherwise identical renders.
Hashing the files reports a mismatch where the pixels are identical. Decode to
rawvideo and compare that.

---

## Where things live

| Path | What |
|---|---|
| `packages/example-content/test-cards/animation/` | the card project |
| `packages/example-content/export/lossless/*.mkv` | committed masters |
| `packages/example-content/export/lossless/*.mov` | AE intermediates (gitignored) |
| `experiments/2026-08-12-4k-hevc-perfect-loop/scripts/` | the build pipeline |
| `experiments/2026-08-12-4k-hevc-perfect-loop/out/` | built assets (gitignored) |
| `…/research/after-effects/looping-cumulus.aep` | the cloud project |
