# Asset binding

This page explains how dexd establishes that it is playing the intended bytes at the intended frame rate. It covers the sidecar file that carries both, the restricted JSON it is written in, the checksum check and the asset check that reads the leading NAL units at startup, and the `dex-sidecar` tool that produces sidecars. It is written for a developer reading the player's source. Where these checks sit among the others, and the exit code each refusal produces, are in [startup-checks.md](startup-checks.md).

## Rate and identity

A raw HEVC elementary stream carries no timestamps, so the file does not record its own frame rate. A player told 25 fps for a 30 fps video plays it a fifth slow for as long as the exhibition runs, with no error and every counter reading normal. The rate therefore travels beside the video, in a file dexd requires.

The sidecar is `<asset>.json` next to the video: `artwork.265` gets `artwork.265.json`. Two keys are required. `fps` carries the rate as text, and `sha256` the checksum over the video's exact bytes. dexd reads the video into memory, parses the sidecar, takes the rate from it, and refuses to start when the checksum does not match.

The sidecar records no display mode. The same 4K video plays scaled on a 1080p monitor, so the display mode is a property of the installation and lives in the exhibit config (decided) — see [exhibit-config.md](exhibit-config.md).

The checksum and the asset check test different properties, and neither covers the other. The checksum proves the bytes are the ones that were prepared; the asset check proves those bytes have the shape a gapless loop needs. A truncated copy still carries intact leading NAL units, and an open-GOP video hashes correctly.

## Sidecar grammar

The sidecar is one flat JSON object whose values are strings or unsigned integers. Anything else is a parse error, and dexd refuses to start on a parse error: an unparseable sidecar and a missing one are the same operational fact (decided).

| Key | Value | Role |
|---|---|---|
| `fps` | string | required; the frame rate |
| `sha256` | string | required; 64 hex digits |
| `width`, `height` | unsigned integer | optional; read only for the display-mode warning (see [startup-checks.md](startup-checks.md)) |
| `source`, `encoder_cmd` | string or unsigned integer | optional; free text, ignored |
| any other key | string or unsigned integer | ignored |

Unknown keys are ignored, so a preparation tool can record more without breaking players already installed. The value restriction still applies to unknown keys. dexd refuses to start when any key's value is an array, a boolean, `null`, a float, a negative number or a nested object, and the message names the key, with the value or its type.

`fps` is a string rather than a JSON number. `30000/1001` is not expressible as a JSON number at all, and 29.97, as a float, rounds. A 4K video decimated from 59.94 fps has a true rate of 30000/1001. After validation the string goes verbatim to mpv's `container-fps-override`; a rate written as a number is refused with a message giving the two spellings it should have used.

A well-formed rate is a positive integer (`30`), a positive decimal (`29.97`) or a positive rational (`30000/1001`). Leading or trailing spaces, signs, exponent notation, a dangling `.` or `/`, and every spelling of zero are refused, a zero rate being a typo in every case.

`sha256` must be 64 hex digits. Uppercase is accepted and stored lowercase, so a digest pasted from a tool that prints capitals still binds. `width` and `height` must be integers, and a missing required key is named in its refusal.

### Duplicate keys

`{"fps":"30","fps":"25"}` is an error; the document has to be unambiguous. Duplicate rejection is the only rule dexd implements itself: a JSON map keeps the last value with no error, so the sidecar deserialises through a hand-written map visitor over `serde_json` instead of a derived implementation. Lexing, escapes, surrogate pairs, trailing data and structural errors are `serde_json`'s.

The rule covers a repeated key the player never interprets, because the rule is about the document, whatever a key means. A test names duplicate rejection on its own, so a refactor that swapped the visitor for a plain map would fail that test and no other.

### Non-ASCII values

Non-ASCII text is allowed in values, and string escapes include `\uXXXX` with UTF-16 surrogate pairs. Python's `json.dumps` turns every non-ASCII character into `\uXXXX` under its default settings, and Go's `encoding/json` does the same for `<`, `>` and `&`. A `source` filename such as `Zürich.mp4` therefore arrives in one of those forms, and refusing escapes would refuse a byte-perfect, correctly hashed video over an ingest tool's serialiser settings.

Raw UTF-8 and the escaped spelling parse to the same value. A truncated escape, a non-hex digit, and a lone or mispaired surrogate are refused.

## Frame-rate resolution

The sidecar is the source of the rate. `--fps` on the command line is a cross-check.

| Sidecar | `--fps` | Result |
|---|---|---|
| present | absent | the sidecar's rate |
| present | same string | the sidecar's rate |
| present | different string | refused, naming both values |
| absent | any | refused |

Comparison is string equality, so `--fps 30` against a sidecar reading `30/1` is refused although the two name the same number. Dropping `--fps` clears the refusal, whichever value is wrong, since the sidecar decides regardless. Ingest tools and any script that starts the player should settle on one spelling of each rate. The systemd unit in the package passes no `--fps` — see [service-unit.md](service-unit.md).

With no sidecar present, startup refuses. If `--fps` alone worked whenever the sidecar was absent, a deployed player could run unbound, so running without one takes two flags: `--test-rig-no-sidecar` together with `--fps`. Under that pair the sidecar is ignored even when the file exists, and an unusable `--fps` is refused — see [startup-checks.md](startup-checks.md).

## Checksum verification

Once the rate resolves, dexd hashes the bytes it read and compares the result with the sidecar's digest. On a mismatch dexd names both digests and exits 2. The message says that the video or the sidecar is stale, wrong or truncated, and that the video has to be prepared again.

A test binds a sidecar to a full video, removes 20 bytes from the tail of the file on disk, and asserts exit 2 with `sha256` in the message. The leading NAL units stay intact, so the asset check passes the file.

## The asset check

The asset check, in `nal.rs`, passes when the VPS, SPS and PPS parameter sets have all appeared before the first slice NAL unit and that slice is an IDR. An IDR is NAL type 19 or 20 in H.265 Table 7-1 (documented). Everything after the first slice is out of scope; the checksum covers the rest of the file.

Byte 0 has to begin a closed GOP for the loop to be gapless: returning there mid-stream is then an ordinary keyframe rather than a seek. A video of any other shape plays, and then breaks at every loop point with no error — see [endless-stream.md](endless-stream.md).

The check requires an IDR rather than any keyframe of the wider IRAP family. CRA (type 21) starts an open GOP whose leading pictures, RASL, may reference pictures before it, so whether such a video loops cleanly depends on its content; BLA (broken-link access, 16–18) does not come out of a working ingest, and 22 and 23 are reserved (decided).

dexd finds NAL boundaries with a plain search for `00 00 01`. Both the three- and four-byte start-code forms contain that pattern, so one scan handles both. Encoders insert emulation-prevention bytes, padding that keeps `00 00 01` from occurring inside a payload, so on a well-formed stream the scan cannot match anything else.

The scan passes over non-slice units before or among the parameter sets: an access unit delimiter, which marks a picture boundary, or a supplemental-enhancement-information unit, which carries metadata beside the pictures. A start code with fewer than two bytes after it ends the scan like any other end of data.

| Condition | The refusal names |
|---|---|
| no start code anywhere | that this is not a raw HEVC stream, with the ffmpeg command that repackages an MP4 into one without re-encoding |
| `forbidden_zero_bit`, the first bit of the NAL header, is set | a corrupt NAL header |
| first slice before a parameter set | the missing sets, by name |
| first slice is a CRA | CRA, and a closed-GOP re-encode |
| first slice is another type | its type number |
| parameter sets but no slice | that no slice was found |

A parameter set appearing after the first slice does not satisfy the requirement retroactively; the refusal still names it missing.

Two end-to-end tests give the sidecar a matching digest so that only the asset check can refuse: on 64 KiB containing no start-code byte dexd exits 2 with `start code` in the message, and on a CRA-led stream with `CRA`.

## Checksum implementation

The checksum module in `sha256.rs` returns 64 lowercase hex characters and computes them with `sha2`, the SHA-256 implementation from the RustCrypto crates. The module keeps its own API, so its test vectors check the digest the module returns, independent of the crate behind it. The vectors are the NIST ones from FIPS 180-4 (documented) — the empty input, `abc`, the 56-byte message and one million `a` bytes — plus six input lengths chosen around SHA-256's 64-byte padding boundary.

The digest the module returns is part of the sidecar's data format. A change producing different bytes would invalidate every sidecar already written and turn the checksum check into a refusal on every deployed player.

## dex-sidecar

`dex-sidecar write` produces a sidecar and `dex-sidecar check` verifies an existing pair. Both parse the sidecar and verify the digest through the library the player links, and both read the video with the same call the player makes. A sidecar this tool accepts is therefore one dexd accepts, and the format has no second implementation to drift from the first. The command lines and options are in [../guides/reference.md](../guides/reference.md), and the workstation procedure in [../guides/prepare-video.md](../guides/prepare-video.md).

The package does not install the tool: a video is prepared on a workstation, never on the player (decided). `dex-sidecar` links only the parser and the hash, not libmpv and not a DRM device, so it builds and runs on a workstation as it does on the player.

`check` prints `OK fps=… sha256=… width=… height=…` when the sidecar parses and its digest matches the file. Both subcommands share one exit-code contract: 0 on success; 1 when a file cannot be read or a verification fails; 2 on a refusal or a malformed command line.

Beside each encoded video, example-content also ships companion files for other players — a JSON file for pivid, a web page for a browser-based player. Those files are unrelated to this format.

### Frame rate at ingest

Without `--fps`, the rate comes from ffprobe's `r_frame_rate`, which for a raw stream is only as good as the timing the encoder wrote into the SPS. Where there is none, ffprobe answers with its internal timebase, 1200000/1, which is not a frame rate. `dex-sidecar` therefore accepts a reported rate only between 1 and 1000 fps — generous enough for any real camera or encoder, and three orders of magnitude below that timebase.

Outside the bound, or with ffprobe missing or failing, `write` refuses and asks for `--fps`. A rate in range is reduced to lowest terms and printed in a note that says where it came from.

With `--fps` and a rate in range both present, `write` compares the two as decimals rounded to six places and refuses when they differ by more than 0.02 fps. Two spellings of one rate do not disagree; `3` against `30` does, and a difference that size reads as an ingest typo. `--force` proceeds with a warning, and also permits replacing a sidecar that already exists.

`write` validates an explicit `--fps` by building a sidecar around the value and asking dexd's parser whether it reads back unchanged. That also keeps a value carrying quotes or backslashes from reshaping the JSON around it.

### Writing

`write` writes `<stream>.json` unless `--out` names another path; that default is the only name dexd looks for. The written file is one line: `fps` and `sha256`, then `width` and `height` when ffprobe reported both. Where ffprobe reported only one of them or neither, `write` leaves both out and prints a note saying so. dexd reads `width` and `height` only for the display-mode warning — see [startup-checks.md](startup-checks.md).

Before the file reaches its name, `write` parses its own output with dexd's parser, compares the values that come back with the values that went in, and re-verifies the digest against the bytes. On a failure `write` reports a defect in itself and writes nothing. The accepted text then goes to a temporary name in the same directory and is renamed over the target, so an interrupted run cannot leave half a sidecar where dexd will look for a whole one.

## Test scope

The grammar, the frame-rate resolution and the NAL rules are pure functions with unit tests that run on a workstation. The tests that spawn dexd enforce the refusals and their exit codes against real files.

The `write` tests build HEVC streams with ffmpeg in two shapes — one whose encoder wrote frame timing into the stream and one whose encoder did not. They cover:

- a fresh write;
- the refusal to overwrite, and `--force` overwriting;
- an `--fps` that contradicts the stream;
- a rate that has to be supplied because none can be read;
- `check` failing after one byte in the middle of the stream is flipped.

See [development.md](development.md).

## Alternatives

| Option | Outcome | Why not |
|---|---|---|
| Take the frame rate from the command line | A deployed player runs at a rate nobody bound to the file | The sidecar binds the rate to the bytes |
| Record the display mode in the sidecar | A forced mode contradicts the connected display | The mode belongs to the installation, not the file |
| Accept any JSON value under ignored keys | A sidecar with a nested object or an array parses | Skipping arbitrary values needs recursion in a parser whose only job is refusing predictably, and no tool emits them |
| Accept any IRAP keyframe (16–23) | A CRA-led video passes and breaks at every loop point | Whether it loops cleanly depends on the content |
| Refuse non-ASCII bytes in sidecar values | A correctly hashed video is refused over its source filename | Filenames are routinely non-ASCII |
| Reimplement the grammar in the ingest tool | Two grammars drift, and a sidecar passes one and fails the other | The tool links the player's parser |
| Write the sidecar with a shell `printf` | A hand-edited file is first read by the player, at the venue | `dex-sidecar write` reads its own output back |

## Open questions

Whether the IDR-only requirement should ever be relaxed for a particular video is undecided. The check defines no criteria for such a relaxation; relaxing the requirement needs a criterion written down first.

A video corrupted before it was prepared hashes correctly, and the sidecar then records that corruption as the intended bytes. Ending preparation with a test play on real hardware is decided and not built — see [roadmap.md](roadmap.md).
