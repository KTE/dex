# docs-lint

Mechanical writing gate for everything this repository ships: Rust comments and string literals,
Markdown prose, `#` comments in YAML/TOML/unit files and maintainer scripts, man pages, and the
shipped default config. It enforces the lintable half of the writing rules in `AGENTS.md`. Node ≥ 20,
zero dependencies. The rules needing a reader (pronoun resolution, aphorisms, whether a rejected
alternative was ever on the table) are deliberately out of scope.

## What it checks

- `plan-codes` (E) — `F6`, `T7`, `M5`, `C1` in prose or identifiers.
- `private-refs` (E; W for "the design") — `PLAN.md`, `SPEC`, `§n`, `principle n`, `bug #n`, reviews.
- `dates` (E/W) — date in a heading, date as justification, other ISO dates, and `(later)` /
  `(evening)` heading splits. The time-of-day check reads headings only: in a table cell, "Pi 2 B
  (later)" says which board revision.
- `caps` (E) — ALL-CAPS that is neither a glossary acronym nor a constant; reports "dictionary-word
  emphasis" and "unknown acronym" separately. A name a substitution reads is a constant, not
  emphasis: `READY=1`, the format placeholder `{USAGE}`, and `$HOME` / `${HOME}` are all quiet.
- `bold` (W/E) — bold density outside tables; bold on a verb or negation.
- `intensifiers` (E/W) — deliberately, honestly, genuinely, load-bearing, the whole point, exactly.
- `contrast` (W) — density of "not X, it is Y" / "rather than"; top 10 per file.
- `coinages` (E) — seam, the wrap, soak, live-fire, black wall …, each with its replacement.
- `codenames` (E) — `dexpi4`, the bench, the Dell, today, card IDs. "The bench flag / branch /
  escape hatch / mode / override / -only" is the `--bench-*` CLI mode and is not reported. Writing
  "Cam Link 4K" in full is the introduction §4.8 asks for, so it is never the bare referent, and a
  file that introduces the card and shortens it in the same sentence has still introduced it.
- `first-person` (E) — I / we / us / our, quotations included: a pronoun inside a quotation still
  ships to the reader, so it is reported and `docs/lint-allow.txt` takes the individual line. An
  all-caps run is left to `caps`: `US`, `MY` and `OUR` in a shouted line are not the pronoun, and
  `I` is the one first-person word genuinely spelled in capitals.
- `process-talk` (E) — reviews, earlier drafts, "used to be", CORRECTION / SUPERSEDED. Review counts
  are spelled words (two, three, several); a bare number would read the tail of an ISO date as one.
- `comment-length` (W/E) — doc-comment blocks over 20 / 40 lines, or arguing a decision.
- `structure` (E/W) — emoji in headings and bullets, session-verb headings. `©`, `®`, `™` and the
  arrows are typography, not status markers.
- `links` (E) — a Markdown link, or a bare `docs/…md#section` reference in a comment, whose file or
  whose `#anchor` does not exist. Anchors are matched by GitHub's slug, so a repeated heading is
  reachable as `-1`, `-2`, and an underscore survives. `http(s):` and `mailto:` targets are not
  fetched; a link inside a fence or a code span is an example and is not resolved; a full stop after
  a bare reference ends the sentence and is not part of the anchor.

## What counts as prose

Backtick content is a literal and never reaches a rule: inline spans are blanked in Markdown, in
every comment kind (`//`, `///`, `//!`, `/* */`, and `#` in YAML/TOML/unit files and maintainer
scripts) and in Rust string literals, and fenced blocks are blanked in Markdown and in the `//!` /
`///` runs that are Markdown too. A fence counts however deeply it is
indented, so example blocks nested in an ordered or multi-level list item are code like any other.
`::`-qualified paths (`i64::MAX`, `libc::EINTR`) are code wherever they appear in Rust. Identifiers
are split on `_` and camelCase (`F6_MODE`, `parseF6Config` and `f6_missing_config` all report `F6`),
while `f32` / `f64`, the tails of numeric literals (`0xF6`) and metric fastener sizes (`M2.5
standoffs`, `M3 screws`) are not plan codes. Beyond comments, prose also means: Rust string
literals, `Description=` in a unit file, `description` and `extended-description` in `Cargo.toml`
(they become the `.deb` `Description:` field, the most public sentence the package ships), string
*values* in the shipped JSON config, and the arguments of the roff font macros (`.B`, `.I`, `.BR`,
`.IP` …), which carry whole sentences. Two files keep their own vocabulary: roff uppercases every
`.SH` heading in a man page, and `docs/design/measurements.md` keeps its dates, card IDs and
`MET` / `VOID` / `FAIL` / `PASS` verdicts.

## What a rule reads at a time

One comment block, or one Markdown paragraph — not one line. The lines of a unit are joined with a
single space, and a finding still reports the line and column where the match starts, so a sentence
that wraps reads as the sentence somebody wrote: "used to ⏎ live", "an earlier ⏎ revision" and
"three ⏎ adversarial reviews" are all reported, and "the bench ⏎ escape hatch" is not, because the
exclusion that makes it the `--bench-*` CLI mode sits on the second line. A unit ends where the
writer ended it: a blank line, a blank `//!`, a heading, a table row, and a comment that trails code
on its line (two remarks about two statements are never one sentence). The rules that are about a
line as a line — a heading's date, a heading's `(later)`, bold density, emoji in a heading — still
read lines.

## Two files the linter never walks into

`docs/glossary.md` and `AGENTS.md` are the gate's own inputs. The glossary lists the coinages the
linter retires, in the form its parser reads back (`Also written: seam, the wrap`), and the rulebook
quotes every bad example verbatim — so linting either one fails the gate on the file that defines
it. Both are skipped when a directory is walked; the glossary named by `--glossary` is skipped even
when it is passed on the command line. Naming `AGENTS.md` explicitly does lint it, which is how you
check that its *good* examples are clean.

## How to run

    node scripts/docs-lint.mjs                        # defaults: packages/dexd, docs, CI, README
    node scripts/docs-lint.mjs --format json docs     # machine-readable
    node scripts/docs-lint.mjs --rules caps,coinages  # one rule at a time while rewriting
    node scripts/docs-lint.mjs --warn-only            # report without failing
    node --test scripts/docs-lint.test.mjs            # the test suite

Exit `0` clean (or `--warn-only`), `1` on any error, `2` on a usage or config problem. CI runs it
with no arguments as a required check. The exit code is set on `process.exitCode`, never through
`process.exit()`, so a report piped to `jq` or captured in a CI log arrives whole rather than
truncated at the 64 KB pipe buffer.

A path named on the command line that is not there is one of those config problems: the run stops
with `input path does not exist: <path>` and exit `2`, because a gate that quietly drops a path it
was asked to check reports a clean tree for text nobody read. The built-in default paths are a
superset of any one checkout, so a missing one of *those* is skipped as before.

## Adding an allow entry

`docs/lint-allow.txt`, one entry per line, global or scoped to a path glob, **reason required** —
the linter exits 2 on a reason-less entry:

    caps SAND # Broadcom tile format, a real domain term
    caps path=**/*.rs QZXW # the register name in the vendor datasheet
    caps path=docs/design/measurements.md # the whole file keeps the verdict vocabulary

Every entry must name a token, a `path=` glob, or both. A bare `caps # blanket` would parse, match
every `caps` finding in every file, and turn the rule off repository-wide behind a plausible
sentence; the linter exits 2 on it.

Ask first whether the term belongs in `docs/glossary.md`: a glossary acronym is never reported, so
the glossary is the primary fix and the allowlist is only the residue. An entry that suppresses
nothing is worse than no entry, because it also hides the next real hit — check a candidate with
`--rules <id>` before and after adding it.

## The glossary format

A `##`–`####` heading is the term. The heading may gloss it inline — the term is everything before
the first ` (`, ` — `, ` - ` or `:`, so all four of these register `HEVC`, `EDID`, `KMS` and `PATH`:

    ### HEVC (High Efficiency Video Coding)
    ### EDID — Extended Display Identification Data
    ### KMS - Kernel Mode Setting
    ### PATH: the executable search path

An `Also written:` line within four lines of the heading adds aliases, comma-, semicolon-, slash- or
` or `-separated. A term in `[A-Z][A-Z0-9-]+` form is registered as an acronym and silences `caps`;
anything else is a defined term.

## Where the coinage table comes from

`coinages` ships with the elimination table from the vocabulary sweep (coinage → replacement,
printed in every message). As the glossary work approves more replacements, write them to a
two-column `coinage<TAB>replacement` file and pass `--coinages docs/coinage-table.tsv`: entries
extend the built-in table, and an entry with the same key replaces it.
