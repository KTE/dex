# After Effects — automation and output findings

**Date:** 2026-08-12 · **Status:** ⚠️ **PARTIAL — the cumulus recipe was lost**

## What happened

A research agent investigated AE-native cumulus generation and delivered an **addendum** to a
main report that never arrived; its transcript was gone before it could be re-queried. So this
file preserves the addendum's findings only.

**Missing, and not recoverable from here:** the effect stack that produces cumulus, the exact
loop mechanics (which parameters to animate, which silently break the loop), and render cost at
4K. The agent said the recipe was "verified by byte-identical 4K renders" but the details of
what was rendered and compared are lost.

**It was also never asked the seam-quality question** the other two approaches answered: whether
the loop is *C1-smooth* — i.e. whether the per-frame change across the wrap matches the interior.
A Fractal Noise Evolution that ping-pongs would be bit-exact and still visibly decelerate into
the loop point. Treat AE's loop as unverified on that axis.

Everything below IS verified — the agent enumerated properties and probed the binaries locally.

## Headless automation actually works, and more easily than expected

`aerender` is **not** a separate renderer. It launches AE, which runs
`Scripts/Startup/commandLineRenderer.jsx`, and aerender RPCs into it over a socket.
**Therefore Startup scripts execute under `aerender`** — so a fully headless build+render needs
**no sudo and no patching**. Drop the `.jsx` into:

```
~/Library/Preferences/Adobe/After Effects/26.3/Scripts/Startup/
```

There is no argument channel: read job parameters from a file or env var. Startup scripts run
before any project opens, so open your own.

### Myths to avoid

- **`aerender -r foo.jsx` exits 3** (`EXIT_SYNTAX_ERROR`). Any documented `aerender -r` you find
  online is **nexrender**, which patches `commandLineRenderer.jsx` — it is not an Adobe flag.
- There is **no `afterfx` binary and no `-r` flag on macOS**, so `open -a ... --args -r` is a dead
  end. The JXA `doscriptfile` route is the documented macOS path.

### The constraint that decides whether AE can be automated at all

**AE on macOS requires a logged-in GUI (Aqua) session.** A LaunchDaemon, or a cold
`ssh mac-zrh aerender ...`, will not work — aerender talks to AE over a local socket and non-Aqua
contexts break it (the classic `-1701` error).

- Use a LaunchAgent with `LimitLoadToSessionType = Aqua`, or bounce in via `launchctl asuser`.
- On Apple Silicon, also wrap with `arch -arm64`.
- **If a truly no-GUI pipeline is a hard requirement, AE is the wrong tool.**

## Output format

**16-bit TIFF sequence.** The only option that is simultaneously mathematically lossless, fast to
write, trivially ffmpeg-decodable, and free of its own colour-encoding transform.
`"TIFF Sequence with Alpha"` is the only shipped still-sequence template that is not PSD.

For a **grayscale source in an 8-bpc project, plain 8-bpc "Millions of Colors" TIFF is equally
exact and smaller.**

Avoid:

| Format | Why not |
|---|---|
| PNG | **20-30x slower to write** (two independent benchmarks), and no built-in template |
| ProRes 4444 | "Visually lossless", **not mathematically lossless** — Apple's own white paper says so, and Apple explicitly disowns ffmpeg's implementation |
| DPX | Applies a Logarithmic Conversion by default |

`-renderSettings` and `-outputSettings` override individual settings *after* a template is
applied (e.g. `"Depth: Trillions of Colors"`), so custom templates need not be installed — useful
given there is no built-in PNG Sequence template.

## The real bit-exactness risk is colour management, not codecs

**AE converts Working -> Output colour space on render, silently rewriting every pixel.** This is
the failure that actually bites, and neither the agent nor I flagged it initially.

Fixes, best first:

1. Set **Output Color Space = Working Color Space** (identity)
2. Tick **Preserve RGB** in Output Module -> Color
3. Set **Working Color Space = None**

Also untick **Embed color space** if byte-identical files across runs are wanted.

**Gate it, do not trust it.** Render one frame of a comp with known values
(`0,0,0` / `255,0,0` / `128,128,128`), decode it, and assert the pixels are exactly what was
authored. **Comparing two AE exports against each other will not catch this** — both would be
wrong identically. That is the same class of error as an unvalidated analyzer reporting "no
anomalies": a self-consistent measurement that is self-consistently wrong.

## Scripting details verified locally

- **Fractal Noise properties are FLAT, not nested.** `Transform` (idx 7), `Sub Settings` (17) and
  `Evolution Options` (25) are group *headers with zero children*; `Rotation`, `Scale` and
  `Evolution` are siblings at the effect's top level. So
  `fn.property("Transform").property("Rotation")` **throws**. (The agent's walker did emit nested
  paths for `Compositing Options > Masks`, so the flatness is real, not a traversal bug.)
- The prefs section is **`Main Pref Section v2`**, not `Main Pref Section`. AE appears to alias
  the un-suffixed name, but use `v2`.
- **`Roughen Edges` and `Cell Pattern` are 8-bpc-only plugins.** If either is added for edge
  detail it becomes the precision bottleneck for the whole chain.

## Standing recommendation

Two alternatives are already validated end-to-end on this machine with measured render times and
verified loop smoothness (see `../blender-clouds/` and `../opensimplex-clouds/`). AE's advantage
would be **integration** — the test card is itself an AE comp — but its cumulus quality and loop
guarantees are unverified here, and the GUI-session requirement is a real constraint on
automation.

Re-dispatch AE research only if integration cost turns out to dominate. The findings above stand
on their own for the separate AE-automation effort regardless of which cloud generator wins.
