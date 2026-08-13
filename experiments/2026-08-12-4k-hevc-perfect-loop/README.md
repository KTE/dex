# 4K HEVC Perfect Loop — probe harness

Measurement harness for the dex seamless-loop experiment.

- **Spec:** [SPEC.md](SPEC.md) — what is being built and why
- **Running record:** [LOG-4k-hevc-perfect-loop.md](LOG-4k-hevc-perfect-loop.md)
- **Implementation plan:** [archive/IMPLEMENTATION-PLAN.md](archive/IMPLEMENTATION-PLAN.md)
- **Building assets:** [BUILDING-ASSETS.md](BUILDING-ASSETS.md) — the full card → bench-asset
  pipeline, repeatable for new formats

## What this measures

Whether a player loops a video **without a seam** — no black frame, no held frame,
no dropped frame at the wrap point.

The test card carries its own frame index as a binary barcode burned into every frame —
centred, and exactly on the card's grid. Capture decodes that barcode into a stream of integers; analysis compares
the anomaly rate **at wrap transitions** against the anomaly rate **mid-loop**. The
mid-loop rate is the capture noise floor, measured on the same run by the same
instrument — so the verdict is a comparison, never an absolute.

## Install

```bash
pnpm install          # devDependencies only; the harness itself has zero runtime deps
pnpm test             # unit + end-to-end tests, no hardware required
pnpm typecheck
```

Requires `ffmpeg` (with `libx265`), Node >= 22, and `shellcheck` for development.

## Generate assets

One command per lossless test-card export. Resolution, frame rate and duration are
**probed from the file**, so the variant name always describes what the file actually is:

```bash
scripts/build-bench-assets.sh \
  --input ../../packages/example-content/export/lossless/test-card-1s-1080p.mov --h264
```

Produces, in `out/`:

| File | What |
|---|---|
| `dex-test-card-1s-1080p30.mp4` | HEVC — the mpv/pivid/cog asset |
| `dex-test-card-1s-1080p30.h264` | raw Annex-B — the `hello_video` **positive control** |
| `dex-test-card-1s-1080p30.json` | pivid timeline |
| `dex-test-card-1s-1080p30.html` | cog page |
| `lossless/test-card-1s-1080p30-barcoded.mkv` | intermediate |

It picks the bitrate by resolution (40M at 4K, 20M below — both under the Pi 4's ~80 Mbps
HEVC ceiling) and then **decodes the barcode back out of the finished encode**, failing
loudly if any frame mismatches. A silent barcode failure would poison every measurement
taken with that asset.

### `--target-fps` — and why the 4K primary asset is 30 fps

The Cam Link 4K **records** 4K at 30 fps. Pointed at 4K60 content it captures every *other*
frame, so the analyzer sees indices stepping by 2 throughout, flags the whole run anomalous,
and wrap detection breaks. So the primary 4K measurement needs a 30 fps asset:

```bash
# primary: 4K30, decimated from a 60fps export
scripts/build-bench-assets.sh --input test-card-3s-4K.mov --target-fps 30 --bitrate 40M

# stretch check: 4K60 native — capture this one through the 1080p60 path,
# where the Cam Link keeps full frame rate
scripts/build-bench-assets.sh --input test-card-3s-4K.mov --bitrate 60M
```

Decimation is legitimate for this content specifically: it is synthetic graphics with no
motion blur, so every Nth frame is an exact lower-rate sampling of the same motion. The
rotations still complete over the loop, so the wrap stays matched. The source frame rate
must be an exact integer multiple of the target, or the script refuses.

The test card itself comes from `packages/example-content/test-cards/animation/test-cards.aep`.
`scripts/make-test-card.sh` generates a *procedural* card instead — that one exists only as a
unit-test fixture and should not be used on a bench (see SPEC.md §3.2).

## Run a probe (on the Pi)

```bash
scripts/probe.sh --config loop-file --asset dex-test-card-1s-2160p30.mp4 --stats /tmp/mpv-stats.txt

# print the argv without launching — this argv is the experiment's deliverable
scripts/probe.sh --config loop-file --asset dex-test-card-1s-2160p30.mp4 --dry-run
```

Configs: `loop-file`, `ab-loop` (needs `--duration`), `keep-open`.
Override the decode/output path with `--vo` and `--hwdec`.

## Measure (on the Mac, capture device attached)

```bash
bin/capture.mjs --source avfoundation:0 --out out/idx.txt --frames 18000
bin/analyze.mjs --log out/idx.txt --loop-length 30
```

`--source` also takes a file path, which gives replay: any recorded clip can be
re-run through a revised decoder without re-capturing.

> **No resolution flag is needed.** The barcode rectangle is expressed as fractions
> of the frame, so the same decode filter works whether you capture at 4K30 or at
> 1080p60 — which matters because the 4K60 stretch check is captured through the
> 1080p60 path.

## Soak

```bash
scripts/soak.sh --source avfoundation:0 --loop-length 30 --outdir out/soak
```

Writes one row per chunk to `out/soak/summary.tsv` and keeps raw logs **only** for
chunks that did not pass. `--chunks 0` (the default) runs until interrupted.

## Reading a verdict

| Verdict | Meaning |
|---|---|
| `PASS` | Wrap anomaly rate indistinguishable from the mid-loop noise floor |
| `FAIL` | Wrap rate significantly exceeds the noise floor — a seam |
| `VOID` | Wrap rate is significantly *below* the floor — wraps are being misclassified; check `--loop-length` |
| `INSUFFICIENT` | Fewer than 500 wrap transitions; capture longer |

`analyze.mjs` exits `0` only on `PASS`, so shell pipelines can branch on it.

## Design notes worth knowing before changing anything

- **`capture.mjs` decides nothing; `analyze.mjs` touches no hardware.** Keep it that way —
  it is what makes the analyzer testable without hardware and captures replayable.
- **Barcode geometry lives only in `lib/barcode.mjs`.** Shell scripts read it through
  `bin/barcode-filter.mjs`. A burner and decoder that drifted apart would fail silently.
- **`make-test-card.sh` wraps the frame counter with `mod(N,PERIOD)` before the trig.**
  Without it, frame `PERIOD` evaluates `cos(2*PI)`/`sin(2*PI)` — mathematically equal to
  `cos(0)`/`sin(0)` but not numerically identical, which flipped 95 of 61440 pixels by one
  luma level and broke the bit-exact wrap.
- **The GOP settings in `encode-variants.sh` are load-bearing.** An open GOP would make
  frame 0 depend on frames that no longer exist at the wrap — manufacturing the very
  seam being measured.
