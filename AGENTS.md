# Writing and working rules for the dex repository

These rules apply to everything a reader outside the project can see: the documentation under `docs/`, `README.md` files, man pages, `--help` and error text, code comments and doc-comments, commit messages that will survive a squash, and the Debian changelog. They were derived from a review of the project's own earlier text and are enforced by `scripts/docs-lint.mjs` where a machine can check them and by review where it cannot.

Agents: read this file and `docs/glossary.md` before writing or editing any of the above. When a rule and your instinct disagree, the rule wins; when a rule is wrong, change the rule in a pull request rather than working around it.

## Who the text is for

- **User documentation** (`docs/guides/`, `packages/dexd/README.md`, man pages, messages the player prints): a venue technician or an artist's helper who can use a terminal and follow a recipe. Assume no knowledge of video codecs, Linux graphics or Rust. Every technical term is either plain English or a glossary entry marked *user*.
- **Developer documentation** (`docs/design/`, code comments, doc-comments): a competent Linux or Rust developer who has never seen this project. Assume general technical knowledge; every video, display or Raspberry Pi specific term is a glossary entry.
- Text that reads only to someone who followed the project's history is a defect, however accurate.

## The rules

1. Plan codes stay private. Names such as `F6`, `T7`, `M5`, `C1`, `§5c`, `A3` never appear in public text, identifiers, test names or messages. Name the thing (`the exhibit config`, `the sidecar check`, `the forced-recovery test`). *(lint)*
2. Private documents are not citations. `SPEC.md`, `PLAN.md`, `IMPLEMENTATION-PLAN.md`, the experiment log, `the story`, `the plan`, `the review`, `principle N`, `bug #N` are not citations. State the fact, or link a file under `docs/design/`. *(lint)*
3. Dates are provenance, not structure or justification. No date in a heading; no `as of YYYY-MM-DD` describing current behaviour. History goes to the changelog or the measurement record, with the date there. *(lint)*
4. Emphasis by word order, not typography. `ALL-CAPS` only for acronyms in the glossary, constants and environment variables; bold only for a literal the reader must type or will see on screen. *(lint)*
5. House intensifiers go. `deliberately`, `honest(ly)`, `genuinely`, `load-bearing`, `the whole point / story / trick`, `measured not argued`, `exactly` (unless before a number). If the sentence loses nothing without the word, the word goes. *(lint)*
6. Say what it is, not what it isn't. `Not X, it is Y` is allowed only when the reader plausibly believes X. Otherwise state Y. *(reader; density warned by lint)*
7. Use the field's word; coinages are replaced or defined once. The retired-words table below and `docs/lint-coinages.tsv` list the project's own coinages and what to write instead. Any term that is neither plain English nor in `docs/glossary.md` is a defect. *(lint)*
8. Introduce every referent in the document that uses it. No `the bench Pi`, `the Dell`, `the capture card`, `the box test`, `today's session`. Say what the device or event is the first time. *(lint for codenames; reader for the rest)*
9. Third person, no confession. No `I` / `we` / `us` / `our`, no `we measured`, `the honest answer`. State the fact and its provenance label. *(lint)*
10. Shipped text carries no session, review or revision talk. `three reviews`, `an earlier draft`, `this originally`, `used to be`, `at review time` belong in the changelog or a private log. Shipped text describes current behaviour only. *(lint for the keyword list; reader for the rest)*
11. Doc-comments describe; design docs argue. A doc-comment gives what the item does, its contract, and at most one sentence of why, with a link. Rationale longer than three sentences, rejected alternatives and incident stories move to `docs/design/`. *(reader; length warned by lint)*
12. Every *this / that / it / here* has its noun in the same or the previous sentence. When in doubt, repeat the noun. *(reader)*
13. One qualifier per claim, and a label instead of adverbs. Use *measured / decided / documented / derived / assumed / not tested*, once. *(reader)*
14. Structure by subject; status in words. Headings name what, never when; no emoji as status; no "(later)" splits. *(lint)*
15. A rule, not an aphorism; a mechanism, not a metaphor. If a sentence could be printed on a poster, replace it with the instruction it stands for. *(reader)*

Numbers carry their unit and conditions ("29.1 fps at 3840×2160, 30 fps, Raspberry Pi 4"). Measured values say so briefly and link `docs/design/measurements.md` for the full conditions rather than repeating them.

## Vocabulary

`docs/glossary.md` is the only list of technical terms the documentation may use without explaining them. Its entries were approved one by one by the project owner. Rules for the file:

- **Agents propose, never approve.** To add or change an entry, open a pull request that touches `docs/glossary.md`; `.github/CODEOWNERS` routes it to the owner. Do not merge glossary changes yourself, and do not paraphrase an existing definition.
- A user-tier definition must be understandable with no other entry. A developer-tier definition may reference other entries with "(see …)".
- The retired words below are never used in public text, whatever the tier. `scripts/docs-lint.mjs` reports the ones a machine can catch.

### Names that were decided

| Use | Not |
|---|---|
| `dexd` — the package, the binary, the service; the player | `dex-loop`, `dex_loop`, "the looper" |
| `dex-sidecar write` / `dex-sidecar check` | `make-sidecar.sh`, `sidecar-check` |
| `dex-exhibit-apply`, `dex-wait-hdmi` | — |
| **exhibit config** — the file `/etc/dex/exhibit.json` or `.yaml` | "the exhibit" on its own |
| **asset** / **video asset** — the video file; the **artwork** is the whole installation it plays in | "the artwork" for the file |
| **forced display mode** in prose; `kms_force` only as the literal config key | "kms force", "KMS forcing" |
| **system log** in prose; `journalctl -u dexd` in commands | "the journal" |
| **dex card** — the SD card that makes a Raspberry Pi a player | `player card` |
| **video container** | "container" alone |
| **loop point**; **gapless** / **seamless** (property); **a held frame**, **a freeze**, **the picture is stuck** (defect) | `seam`, `the wrap`, `hold`, `wrap point` |
| **long-running test**, **24-hour test** | `soak` |
| **test video** (an encoded test file); **test card** (the synthetic picture it is made from) | `bench asset` |
| **test rig** — one test setup; the **bench** — the development workstation and its hardware | "bench" for a setup |
| `--test-rig-no-sidecar`, `--test-rig-hang-after-secs`, `--test-rig-force-recovery-after-secs`; `(test rig only)` | `--bench-*`, `BENCH ONLY`, `wedge` |
| **unresponsive**, **hangs**, **is hanging** | `wedged`, `hung` |
| **supervisor thread** | `event thread` |
| `loops=` in the heartbeat; **loop count** or **loop iterations** in prose | `wraps=`, `wrap count` |
| **check** — the sidecar check, the asset check, the cmdline check | `gate` |
| **prepare the video** (user text and messages); *ingest* only in developer text | `re-ingest` |
| **refuses to start rather than guess** (user text); *fail-closed* only in developer text | `fail-closed in a guide` |
| **frame-duration histogram** | `dwell histogram` |
| **written into**, **stored in**, **saved copy of the EDID**, **build-id file** | `baked`, `baked-in`, `stamped`, `stamp file` |
| **in-place recovery**, **process restart by systemd**, **reboot escalation (planned)** | `tier 0 / 1 / 2 / 3` |
| **the pass criteria** | `the bar`, `the pass bar` |
| dexOS (the brand); `dex-os` (the repository) | `Dexbian` |


### Retired words — the full table

| Retired | Write instead |
|---|---|
| `soak` / `soak test` / `24 h soak` / `soak run` / `soak harness` / `thermal soak` | long-running test / 24-hour test / long-term test (name the duration where it matters); 'the long-running-test harness'  |
| `seam` / `the seam` / `seamless-loop as noun` / `'no seam'` / `'a seam'` | place: 'the loop point'; property: 'gapless' or 'seamless'; defect: 'a visible pause / a held frame / a stutter at the loop point'  |
| `the wrap` / `wrap point` / `at the wrap` / `wrap-join` / `wrap transition` / `wr` | 'the loop point' (place); 'one loop' / 'one repeat' (the pass); 'loop count' (the counter); 'loop-position arithmetic' (the code)  |
| `hold` / `holds` / `hold at the wrap` / `held (as noun)` | 'a freeze' / 'the picture is stuck at the loop point' / 'the frame stays on screen for N ms' — describe the defect plainly (Max: freeze / stuck are mo  |
| `bench asset` / `bench-ready asset` / `the card (meaning the encoded video)` | 'test video' / 'the reference test video used for measurements' (a test video made from a test card)  |
| `gaplessness premise` / `loop-ability` | 'the requirement that the loop is gapless' / 'whether a file can loop gaplessly'  |
| `tier 0` / `tier-0` / `tier 1` / `tier 2` / `tier 3` | 'in-place recovery' (0), 'process restart by systemd' (1), 'reboot escalation (planned)' (2), 'hardware watchdog (planned)' (3)  |
| `fail closed (user tier)` / `fail-closed contract` / `fail-silent` | user tier: 'refuses to start rather than guess'; developer tier: 'fail-closed' allowed as a glossary term (? — needs Max)  |
| `live-fire` / `live-fire probe` / `live-fire test` | 'against a real mpv instance' / 'on real hardware' / 'the forced-recovery test'  |
| `wedged` / `wedge` / `core-wedge` / `display-wedged` / `'the wedge check'` | 'unresponsive' / 'hangs' / 'is hanging' / 'stopped responding while the process stays alive' — never `hung`; the flag becomes --test-rig-hang-after-se  |
| `pinned (a behaviour is 'pinned' by a test)` | 'locked in by a test' / 'a test enforces'  |
| `the loser` / `delete the loser` | 'the unwanted config file' / 'delete the one you do not mean'  |
| `drift generator` | 'would make the boot config and the player's config diverge'  |
| `black-wall time` | 'the worst-case time the screen can stay dark'  |
| `spins hot` | 'busy-loops, using a full CPU core'  |
| `belt-and-braces` | 'a fallback' / 'a second safeguard'  |
| `the honest count` / `'honest' as an intensifier` | state the number: 'the binary links 228 shared objects'  |
| `green CI` | 'CI passes' / 'a passing CI run'  |
| `trap point` | 'the point inside mpv where the wait would unblock'  |
| `event-shape` | 'the sequence of events' / 'this event'  |
| `the classic monorepo trap` | 'a required check that can silently never run, blocking every merge'  |
| `the crux` | 'the central tension: dexOS is buster, the player needs trixie'  |
| `the box` / `the box test` / `'shares the box'` | 'the device' / 'the sealed-case thermal test'  |
| `the rig` / `capture rig` / `'Bench = …'` | 'the measurement setup (a Pi 4, an HDMI capture device and the analysis scripts)'  |
| `the wrong-panel case` | 'a resolution the connected display cannot show'  |
| `venue truth, not asset truth` | 'the display mode belongs to the installation, not to the video file'  |
| `the mains switch is the shutdown path` | 'there is no graceful shutdown; power is simply cut, and the player is built to survive that'  |
| `field journal` / `field failure` / `in the field` / `on site` / `gallery devic` | 'the log' / 'a failure at the venue' / 'at the venue' / 'deployed players'  |
| `deploy path` / `bench escape hatch` | 'normal startup (sidecar required)' / 'the test-rig-only override (`--test-rig-no-sidecar --fps`)'  |
| `the binding` / `asset+fps binding` / `F3 gate` / `sidecar gate` / `NAL gate` / `` | 'the sidecar's checksum match' / 'the sidecar check' / 'the asset check' / 'the cmdline check' — 'check' in prose; 'gate' allowed as alias (? — needs   |
| `THE EXTENSION DECIDES THE PARSER (all caps)` / `BENCH ONLY` / `ARMED (shou` | sentence case: 'the file extension selects the parser'; the literal warning line stays as shipped  |
| `escalation ladder` / `'escalate per the fixed ladder'` | 'the pre-committed fallback order (pivid, then GStreamer, then a custom player)'  |
| `annulus` / `fps honesty` / `matched wrap` / `'the wrap is matched by constru` | 'ring-shaped region' / 'how far a detected frame rate can be trusted' / plain description  |
| `cleanroom extraction` / `cleanroom` | 'rewritten from scratch for publication'  |
| `buster ceiling` | 'the buster limitation' / describe: 'gapless hardware playback only on buster (32-bit), so no upgrades and no Pi 5'  |
| `the rotation trap` | 'sideways video from phone footage: the container's rotation flag is lost on extraction' (see elementary stream)  |
| `(nogit)` / `+dirty as prose` | 'an unidentified build' / 'a build from uncommitted changes' — the literal version-string markers stay  |
| `hello_video positive control` / `dexOS card` / `'the dexOS positive contro` | 'the known-good reference (the legacy hello_video player on its own test video)'  |
| `mp_dispatch_lock` / `run_locked` / `mp_cond_wait` / `mp_dispatch_queue_proce` | describe the behaviour ('a synchronous property read waits with no timeout for mpv's core thread'); cite the mpv source location in a footnote if prov  |
| `Rust identifiers used as prose nouns (HealthMonitor, ObservedCounter,` | in docs: describe the behaviour and name the module once ('the health policy in health.rs'); identifiers belong in code and API docs, not in guides  |
| `supervisor thread (health.rs) vs event thread (heartbeat.rs, watchdog.` | 'supervisor thread' everywhere (one thread; DEC-010)  |
| `gst1223` / `+rpt2 check` / `'the rpt2 criterion' as bare labels` | 'a GStreamer 1.22 attempt' / 'whether Raspberry Pi's patched ffmpeg build (+rpt2) is required on the Pi 5 — unresolved'  |
| `USV` | 'battery backup (`UPS`)'  |
| `starved feed` / `'signature of a starved feed'` | 'the data source not keeping up (frames held at random points, not at the loop point)'  |
| `the linger bug` | 'the tmux session died with the last SSH login (systemd user session not lingering)' — an operations note for the private record, not dexd  |
| `kiosk (flags` / `mode)` / `argv` / `'the working argv'` | 'fullscreen with no on-screen controls' / 'the mpv command line'  |
| `baked` / `baked EDID` / `baked-in` / `stamped` / `stamp file` / `build stamp` | 'written into' / 'stored in' / 'saved copy of the EDID' / 'build-id file' — the words `baked` and `stamped` appear nowhere (DEC-016)  |
| `hung` | 'hangs' / 'is hanging' / 'unresponsive' — never `hung` (DEC-013)  |
| `--bench-no-sidecar` / `--bench-wedge-after-secs` / `--force-recovery-after` | `--test-rig-no-sidecar` / `--test-rig-hang-after-secs` / `--test-rig-force-recovery-after-secs` / `(test rig only)` (DEC-008; code rename)  |
| `wraps=` / `WRAP_COUNT` / `wrap count` | loops= (heartbeat field, code rename) / 'loop count' / 'loop iterations' in prose (DEC-006)  |
| `event thread` | 'supervisor thread' (DEC-010)  |
| `gate (as the noun for a startup refusal)` / `F3 gate` / `cmdline gate` / `NA` | 'check' — the sidecar check, the asset check, the cmdline check (DEC-009; no alias)  |
| `fail-closed` / `fail closed (user tier)` | 'refuses to start rather than guess' at user tier; developer tier keeps the glossary entry fail-closed (DEC-011)  |
| `kms_force (in prose)` | 'forced display mode' in prose; `kms_force` only as the literal config key  |
| `journal` / `the journal (in prose)` | 'system log' in prose; `journalctl` in commands  |
| `dwell` / `dwell histogram` / `dwell counts` | 'frame duration' / 'frame-duration histogram'  |
| `player card` | 'dex card' (flagged: Max suggested 'dex card or something')  |
| `container (alone)` | 'video container'  |
| `re-ingest the asset (shipped message)` | 'prepare the video again with dex-sidecar write' (DEC-007; code change)  |
| `ingest (user tier)` | 'prepare the video' / 'preparing a video' (DEC-007); developer tier may say ingest  |

### Names of people, places and things

- No artist names, artwork titles, venues, exhibition names, SD-card ids, hostnames of development machines, or the owner's name in public text. `The project decided` replaces a person's name.
- A forum handle may appear in prose when the person's real name is unknown or the account is pseudonymous — always marked and explained: `*Foo*, a Raspberry Pi engineer on the official forums, …`. Otherwise `a Raspberry Pi engineer on the official forums`, with the link as the citation.
- The measurement instrument may be named once, as the instrument's identity, in `docs/design/measurements.md` (`an Elgato Cam Link 4K HDMI capture device`); one display model may serve as a worked example of a forced display mode. Everywhere else: `the capture device`, `a 2560×1440 monitor`.

## Provenance labels

Every measured number, decision and assumption in the documentation traces to a source. In text use one label, once: *measured* (say on what: "measured on a Raspberry Pi 4"), *decided*, *documented* (name the manual or spec), *derived*, *assumed*, *not tested*. The measurement record (`docs/design/measurements.md`) holds the conditions; other documents link it.

## The mechanical gate

```
node scripts/docs-lint.mjs                 # default paths: packages/dexd, docs, .github/workflows/dexd.yml, AGENTS.md, README.md
node scripts/docs-lint.mjs docs/guides/prepare-video.md
```

Errors fail the run; warnings are printed. It reads `docs/glossary.md` (the acronym allow-set), `docs/lint-allow.txt` (per-token or per-path exceptions — every entry needs a reason after `#`, or the tool refuses to start) and `docs/lint-coinages.tsv` (retired words). It runs in CI on `docs/`, `AGENTS.md` and `README.md` files; the crate's comments join the gate when their rewrite lands. Fix an error by rewording; add an allowlist entry only for a true false positive, with the reason.

## Where things go

| Content | Place |
|---|---|
| How to build a dex card, prepare a video, configure the exhibit, run and troubleshoot | `docs/guides/` (user tier) |
| Reference: config keys, options, exit codes, every refusal message and its fix | `docs/guides/reference.md` |
| Why the player is built the way it is; how it fails and recovers; packaging and CI; measurements | `docs/design/` (developer tier) |
| The one-page front door | `packages/dexd/README.md` |
| Terms | `docs/glossary.md` |
| What changed between releases | `packages/dexd/deploy/changelog` (Debian format) |
| Rationale moved out of a code comment | the `docs/design/` page the comment links |

## Commits and code

- Commit subjects: `E:` for code and packaging, `D:` for documentation, imperative, ≤ 72 characters; the *why* in the body. No AI attribution lines.
- No behaviour change rides along with a wording change. Renames that the vocabulary requires (a flag, a heartbeat field, an identifier) are their own commit with tests updated.
- Test names are prose: `an_fps_that_contradicts_the_stream_is_refused`, not `f6_bad_fps`.
