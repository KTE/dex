#!/usr/bin/env node
// @ts-check
// SPDX-License-Identifier: MIT-0
/**
 * docs-lint — the mechanical writing gate for the public dex repository.
 *
 * Checks shipped text (Rust comments and string literals, Markdown prose, `#`
 * comments in YAML/TOML/unit/shell files, man pages) against the writing rules
 * in AGENTS.md. Every rule here is mechanical; the rules that need a reader
 * (pronoun resolution, "was X ever on the table", aphorisms) are not attempted.
 *
 * Usage: node scripts/docs-lint.mjs [options] [paths…]
 *
 * Zero dependencies: node:fs and node:path only.
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * @typedef {'E'|'W'} Severity
 *
 * @typedef {Object} Finding
 * @property {string} path
 * @property {number} line   1-based
 * @property {number} col    1-based
 * @property {Severity} severity
 * @property {string} rule
 * @property {string} message
 * @property {string} token
 *
 * @typedef {Object} AllowEntry
 * @property {string} rule
 * @property {string|null} pathGlob
 * @property {RegExp|null} tokenRe
 * @property {string|null} tokenLiteral
 * @property {string} reason
 * @property {number} lineNo
 *
 * @typedef {Object} Coinage
 * @property {string} key           the coinage as written in the table
 * @property {RegExp} re            global, case-insensitive matcher
 * @property {string} replacement
 *
 * @typedef {Object} Extraction
 * @property {string} kind          'md' | 'rust' | 'hash' | 'toml' | 'unit' | 'man' | 'json' | 'text'
 * @property {string[]} rawLines
 * @property {string[]} prose       one entry per raw line, non-prose blanked (columns preserved)
 * @property {boolean[]} isHeading
 * @property {boolean[]} isTableLine
 * @property {boolean[]} isStructural   roff .TH lines and the like: exempt from caps/date rules
 * @property {number[]} groupId     lines sharing an id >= 0 are one comment block / paragraph; -1 stands alone
 * @property {{line:number,col:number,text:string}[]} identifiers
 * @property {{start:number,end:number,kind:string}[]} commentBlocks
 * @property {number} words
 * @property {Segment[]} [segments] filled in by `extract`; the unit every prose rule scans
 *
 * @typedef {Object} Segment
 * @property {number[]} lines       0-based line indices, in order
 * @property {string} text          those lines' prose, joined with a single space
 * @property {number[]} offsets     offset in `text` where each line's content starts
 * @property {number[]} startCols   0-based column in the source line that offset corresponds to
 *
 * @typedef {Object} FileCtx
 * @property {string} path          display path
 * @property {string} absPath
 * @property {string} text
 * @property {Extraction} ex
 * @property {LintCtx} ctx
 *
 * @typedef {Object} LintCtx
 * @property {Set<string>} acronyms
 * @property {AllowEntry[]} allow
 * @property {Coinage[]} coinages
 * @property {Set<string>} dict
 * @property {Set<string>|null} ruleFilter
 */

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TEXT_EXTS = new Set(['.rs', '.md', '.toml', '.service', '.sh', '.1', '.yml', '.yaml']);
const SKIP_DIRS = new Set(['target', 'LICENSES', 'node_modules', '.git', '.github/ISSUE_TEMPLATE']);

/**
 * Never collected by a directory walk. `AGENTS.md` and `glossary.md` are the
 * linter's own inputs: the rulebook quotes every bad example verbatim, and the
 * glossary lists the coinages it retires ("Also written: seam, the wrap"), so
 * linting either one reports the gate against itself. Naming one on the command
 * line still lints it; the glossary passed as `--glossary` is skipped even then.
 */
const SKIP_FILES = new Set(['Cargo.lock', 'LICENSE', 'COPYING', 'NOTICE', 'AGENTS.md', 'glossary.md']);
const DEFAULT_PATHS = [
  'packages/dexd',
  'docs',
  '.github/workflows/dexd.yml',
  'README.md',
];

/** Rust float types: `f32`/`f64` look like plan codes to 4.1 but are the language. */
const RUST_FLOAT_TYPES = new Set(['f32', 'f64']);

/** Verdict vocabulary of the measurement record (§4.4), allowed there only. */
const VERDICT_CAPS = new Set(['MET', 'VOID', 'FAIL', 'PASS']);

/** Rust convention words that are allowed in ALL CAPS. */
const RUST_CAPS = new Set(['SAFETY', 'TODO', 'FIXME', 'NOTE', 'PANICS', 'ERRORS', 'INVARIANTS']);

/** Man-page section names, allowed in ALL CAPS inside `.1` files only. */
const MAN_SECTIONS = new Set([
  'NAME', 'SYNOPSIS', 'DESCRIPTION', 'OPTIONS', 'EXIT', 'STATUS', 'FILES', 'ENVIRONMENT',
  'EXAMPLES', 'SEE', 'ALSO', 'NOTES', 'BUGS', 'AUTHOR', 'AUTHORS', 'COPYRIGHT', 'REPORTING',
  'HISTORY', 'STANDARDS', 'DIAGNOSTICS', 'SECURITY', 'CAVEATS', 'VERSION', 'CONFIGURATION',
]);

/**
 * Roff macros whose arguments are the sentence, not a directive: the font
 * macros and the indented-paragraph tags. Their text is prose and is linted.
 */
const MAN_TEXT_MACROS = new Set(['B', 'I', 'BR', 'RB', 'BI', 'IB', 'IR', 'RI', 'SM', 'SB', 'IP']);

/**
 * The dictionary-word fallback from the spec (§4.4): all-caps tokens seen in the
 * corpus that are ordinary English words used for emphasis. Used when
 * /usr/share/dict/words is unavailable.
 */
const CAPS_DICT_FALLBACK = new Set([
  'NOT', 'THE', 'ONLY', 'AND', 'SAME', 'BEFORE', 'WHICH', 'THIS', 'OTHER', 'ANY', 'REQUIRED',
  'BOTH', 'REAL', 'ALL', 'WITHOUT', 'WITH', 'WHY', 'NEVER', 'MUST', 'EVER', 'FOREVER', 'SAFE',
  'SHAPE', 'STAYS', 'LOUD', 'IDENTICAL', 'CONSECUTIVE', 'HOURS', 'NATIVE', 'LIVE', 'OWN', 'FROM',
  'ARE', 'BENCH', 'STRING', 'INVARIANT', 'CORRECTION', 'ALWAYS', 'EVERY', 'EACH', 'FIRST', 'LAST',
  'HERE', 'THERE', 'WHEN', 'WHERE', 'WHAT', 'WHOLE', 'ENTIRE', 'AFTER', 'INSTALLATION', 'DELETE',
  'REJECTED', 'IGNORED', 'DISASTER', 'EXTENSION', 'DECIDES', 'PARSER', 'WRONG', 'RIGHT', 'GOOD',
  'BAD', 'NEW', 'OLD', 'YES', 'NOW', 'ONE', 'TWO', 'MORE', 'LESS', 'MOST', 'THAN', 'THAT',
]);

/** Bold content that is a bare verb or negation: an error, not a density warning (§4.4). */
const BOLD_VERB_RE =
  /^(is|are|was|were|be|been|not|no|never|must|will|shall|does|do|did|has|have|had|can|cannot|can't|stops?|stopped|always|only|all|any|both|same|every|exists?|existing)\b/i;

/** The built-in coinage → replacement table (§4.7 plus the human guidance in the sweep brief). */
const DEFAULT_COINAGES = [
  ['/\\bseams?\\b/', 'loop point (the place); a visible pause / a held frame at the loop point (the defect); seamless or gapless (the property)'],
  ['/\\bthe wrap\\b|\\bwrap point\\b|\\bwrap-transition\\b|\\bat the wrap\\b|\\bacross the wrap\\b/', 'the loop point'],
  ['/\\bsoak(s|ed|ing)?\\b/', '24-hour test / long-running test (name the duration)'],
  ['/\\blive-fire\\b/', 'against a real mpv'],
  ['/\\bblack[- ]wall\\b/', 'black screen'],
  ['/\\bsilent wrongness\\b|\\bwrongness class\\b/', 'name the wrong output plainly (for example: plays at the wrong rate without erroring)'],
  ['/\\bthe loser\\b/', 'the superseded file / the copy that is not used'],
  ['/\\bdrift generator\\b/', 'name the mechanism that lets the two copies diverge'],
  ['/\\bgreen nothing\\b/', 'a green frame'],
  ['/\\bthe crux\\b/', 'the constraint that decides it'],
  ['/\\bbuster (ceiling|pin)\\b/', 'the Debian 10 (buster) version limit'],
  ['/\\bthe pass bar\\b/', 'the acceptance criteria'],
  ['/\\bthe hold\\b|\\bhold (at|on|before) the loop\\b/', 'the held frame / a visible pause at the loop point'],
];

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/** @param {string} s */
function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Replace every character of `s` with a space, keeping newlines. @param {string} s */
function blank(s) {
  return s.replace(/[^\n]/g, ' ');
}

/**
 * Turn a glob into a RegExp. `**\/` collapses to an optional path prefix.
 * @param {string} glob
 */
function globToRe(glob) {
  let out = '';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') {
        if (glob[i + 2] === '/') { out += '(?:.*/)?'; i += 2; } else { out += '.*'; i += 1; }
      } else {
        out += '[^/]*';
      }
    } else if (c === '?') {
      out += '[^/]';
    } else {
      out += escapeRe(c);
    }
  }
  return new RegExp('^' + out + '$');
}

/**
 * Ranges of double-quoted text on a line, so quoted speech can be skipped.
 * @param {string} line
 * @returns {[number, number][]}
 */
function quotedRanges(line) {
  /** @type {[number, number][]} */
  const ranges = [];
  let open = -1;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '"' || c === '“' || c === '”' || c === '«' || c === '»') {
      if (open < 0) open = i;
      else { ranges.push([open, i]); open = -1; }
    }
  }
  return ranges;
}

/** @param {[number,number][]} ranges @param {number} idx */
function inRanges(ranges, idx) {
  return ranges.some(([a, b]) => idx > a && idx < b);
}

/**
 * Blank every inline code span (any backtick run), backticks included. Backtick
 * content is a literal, never prose — §4 strips it before any rule runs.
 * @param {string} line
 */
function blankCodeSpans(line) {
  return line.replace(/(`+)(?:(?!\1)[\s\S])*\1/g, (m) => blank(m));
}

/**
 * Blank `::`-qualified paths (`i64::MAX`, `libc::EINTR`, `std::fs::File`): code
 * written inside prose, which §4.4 excludes from the caps rule.
 * @param {string} line
 */
function blankQualifiedPaths(line) {
  return line.replace(/\b[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+\b/g, (m) => blank(m));
}

/**
 * Apply the Markdown blanking rules (fenced blocks and inline spans) to the
 * prose of every `//!` / `///` run — doc comments are Markdown.
 * @param {string[]} prose        modified in place
 * @param {{start:number,end:number,kind:string}[]} blocks
 */
function blankMarkdownInDocComments(prose, blocks) {
  for (const b of blocks) {
    if (b.kind !== '//!' && b.kind !== '///') continue;
    let inFence = false;
    let marker = '';
    for (let i = b.start - 1; i < b.end && i < prose.length; i++) {
      const line = prose[i];
      const fence = line.match(/^\s*(```+|~~~+)/);
      if (fence) {
        if (!inFence) { inFence = true; marker = fence[1][0]; }
        else if (fence[1][0] === marker) { inFence = false; }
        prose[i] = blank(line);
        continue;
      }
      prose[i] = inFence ? blank(line) : blankCodeSpans(line);
    }
  }
}

/** @param {string} text */
function countWords(text) {
  const m = text.match(/[A-Za-z][A-Za-z'’-]*/g);
  return m ? m.length : 0;
}

// ---------------------------------------------------------------------------
// Extractors — one per file type. Every one returns lines of the same length as
// the source lines, with non-prose characters replaced by spaces, so that the
// line and column of every finding is the line and column in the real file.
// ---------------------------------------------------------------------------

/**
 * @param {string} text
 * @returns {Extraction}
 */
function extractMarkdown(text) {
  const rawLines = text.split('\n');
  const prose = [];
  const isHeading = [];
  const isTableLine = [];
  /** A heading and a table row are each their own unit; running prose joins. */
  const groupId = [];
  let inFence = false;
  let fenceMarker = '';
  for (const raw of rawLines) {
    // `^\s*`, not CommonMark's `^\s{0,3}`: a fence nested in an ordered or a
    // multi-level list item is indented four or more spaces, and its contents
    // are still code. Treating it as prose is how `F6`, `soak` and `the wrap`
    // reached the rules from inside example blocks.
    const fence = raw.match(/^\s*(```+|~~~+)/);
    if (fence) {
      if (!inFence) { inFence = true; fenceMarker = fence[1][0]; prose.push(blank(raw)); }
      else if (fence[1][0] === fenceMarker) { inFence = false; prose.push(blank(raw)); }
      else prose.push(blank(raw));
      isHeading.push(false);
      isTableLine.push(false);
      groupId.push(-1);
      continue;
    }
    if (inFence) {
      prose.push(blank(raw));
      isHeading.push(false);
      isTableLine.push(false);
      groupId.push(-1);
      continue;
    }
    let line = raw;
    // inline code spans (any backtick run) → blanked, backticks included
    line = blankCodeSpans(line);
    // bare URLs and markdown link targets carry dates and paths that are not prose
    line = line.replace(/\]\([^)\s]*/g, (m) => ']' + blank(m.slice(1)));
    line = line.replace(/https?:\/\/\S+/g, (m) => blank(m));
    prose.push(line);
    const heading = /^\s{0,3}#{1,6}\s/.test(raw);
    const table = /^\s*\|/.test(raw) || /^\s*[:\-| ]+$/.test(raw) && raw.includes('|');
    isHeading.push(heading);
    isTableLine.push(table);
    groupId.push(heading || table ? -1 : 0);
  }
  return {
    kind: 'md', rawLines, prose, isHeading, isTableLine,
    isStructural: rawLines.map(() => false),
    groupId,
    identifiers: [], commentBlocks: hashOrSlashBlocks(),
    words: countWords(prose.join('\n')),
  };
}

/**
 * Rust: comments and string literals are prose; identifiers in code are
 * collected separately, split on `_` and camelCase boundaries.
 * @param {string} text
 * @returns {Extraction}
 */
function extractRust(text) {
  const n = text.length;
  const mask = new Array(n).fill(' ');
  const code = new Array(n).fill(' ');
  for (let i = 0; i < n; i++) if (text[i] === '\n') { mask[i] = '\n'; code[i] = '\n'; }

  // Offset → 0-based line, so the walk can record which lines each comment owns.
  /** @type {number[]} */
  const lineStarts = [0];
  for (let p = 0; p < n; p++) if (text[p] === '\n') lineStarts.push(p + 1);
  /** @param {number} off */
  const lineOf = (off) => {
    let lo = 0;
    let hi = lineStarts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (lineStarts[mid] <= off) lo = mid; else hi = mid - 1;
    }
    return lo;
  };
  /** @type {{ln:number, kind:string, standalone:boolean}[]} */
  const lineComments = [];
  /** @type {[number, number][]} */
  const blockComments = [];

  /** @param {number} from @param {number} to */
  const keepProse = (from, to) => {
    for (let i = from; i < to && i < n; i++) if (text[i] !== '\n') mask[i] = text[i];
  };
  /** @param {number} at */
  const keepCode = (at) => { if (text[at] !== '\n') code[at] = text[at]; };
  /** @param {number} i */
  const isIdentChar = (i) => i >= 0 && i < n && /[A-Za-z0-9_]/.test(text[i]);

  let i = 0;
  while (i < n) {
    const c = text[i];
    if (c === '/' && text[i + 1] === '/') {
      let j = i;
      while (j < n && text[j] !== '\n') j++;
      let start = i + 2;
      if (text[start] === '/' || text[start] === '!') start++;
      const ln = lineOf(i);
      lineComments.push({
        ln,
        kind: text.slice(i, start),
        // A comment that follows code on its line is a remark about that line,
        // not part of a running block; joining it to the next one would invent
        // a sentence nobody wrote.
        standalone: /^\s*$/.test(text.slice(lineStarts[ln], i)),
      });
      keepProse(start, j);
      i = j;
      continue;
    }
    if (c === '/' && text[i + 1] === '*') {
      let depth = 1;
      let j = i + 2;
      const start = j;
      while (j < n && depth > 0) {
        if (text[j] === '/' && text[j + 1] === '*') { depth++; j += 2; continue; }
        if (text[j] === '*' && text[j + 1] === '/') { depth--; j += 2; continue; }
        j++;
      }
      keepProse(start, Math.max(start, j - 2));
      blockComments.push([lineOf(i), lineOf(Math.max(i, j - 1))]);
      i = j;
      continue;
    }
    if ((c === 'r' || c === 'b') && !isIdentChar(i - 1)) {
      // raw / byte string: r"…", r#"…"#, br#"…"#, b"…"
      let k = i;
      if (text[k] === 'b' && text[k + 1] === 'r') k++;
      if (text[k] === 'r') {
        let h = k + 1;
        let hashes = 0;
        while (text[h] === '#') { hashes++; h++; }
        if (text[h] === '"') {
          const start = h + 1;
          const closing = '"' + '#'.repeat(hashes);
          const end = text.indexOf(closing, start);
          const stop = end < 0 ? n : end;
          keepProse(start, stop);
          i = end < 0 ? n : end + closing.length;
          continue;
        }
      } else if (text[i] === 'b' && text[i + 1] === '"') {
        const start = i + 2;
        let j = start;
        while (j < n && text[j] !== '"') { if (text[j] === '\\') j++; j++; }
        keepProse(start, j);
        i = j + 1;
        continue;
      }
    }
    if (c === '"') {
      const start = i + 1;
      let j = start;
      while (j < n && text[j] !== '"') { if (text[j] === '\\') j++; j++; }
      keepProse(start, j);
      i = j + 1;
      continue;
    }
    if (c === "'") {
      // lifetime (`'a`, `'static`) vs char literal (`'x'`, `'\n'`)
      if (/[A-Za-z_]/.test(text[i + 1] || '') && text[i + 2] !== "'") {
        let j = i + 1;
        while (j < n && /[A-Za-z0-9_]/.test(text[j])) j++;
        i = j;
        continue;
      }
      let j = i + 1;
      while (j < n && text[j] !== "'") { if (text[j] === '\\') j++; j++; }
      i = j + 1;
      continue;
    }
    keepCode(i);
    i++;
  }

  const rawLines = text.split('\n');
  const prose = mask.join('').split('\n');
  const commentBlocks = docCommentBlocks(rawLines);
  // Doc comments are Markdown: fenced blocks and inline spans are literals.
  blankMarkdownInDocComments(prose, commentBlocks);
  // Backtick content is a literal in *every* comment kind and in string
  // literals, not only in `//!` / `///` runs: `` `--bench-only` `` and
  // `` "{PATH}" `` are code wherever a Rust file writes them.
  // `i64::MAX` and friends are code even when they sit in a sentence.
  for (let k = 0; k < prose.length; k++) prose[k] = blankQualifiedPaths(blankCodeSpans(prose[k]));

  /** @type {number[]} */
  const groupId = rawLines.map(() => -1);
  let gid = 0;
  for (const [s, e] of blockComments) {
    gid++;
    for (let L = s; L <= e && L < groupId.length; L++) groupId[L] = gid;
  }
  for (let k = 0; k < lineComments.length;) {
    const head = lineComments[k];
    gid++;
    groupId[head.ln] = gid;
    let m = k + 1;
    while (head.standalone && m < lineComments.length
      && lineComments[m].standalone
      && lineComments[m].kind === head.kind
      && lineComments[m].ln === lineComments[m - 1].ln + 1) {
      groupId[lineComments[m].ln] = gid;
      m++;
    }
    k = m;
  }

  const codeLines = code.join('').split('\n');
  /** @type {{line:number,col:number,text:string}[]} */
  const identifiers = [];
  codeLines.forEach((line, idx) => {
    const re = /[A-Za-z_][A-Za-z0-9_]*/g;
    let m;
    while ((m = re.exec(line)) !== null) {
      // a run that starts right after a digit is the tail of a numeric literal
      // (`0xF6`, `1e6`), not an identifier
      if (/[0-9]/.test(line[m.index - 1] || '')) continue;
      for (const part of splitIdentifier(m[0], m.index)) {
        identifiers.push({ line: idx + 1, col: part.col + 1, text: part.text });
      }
    }
  });
  return {
    kind: 'rust', rawLines, prose,
    isHeading: rawLines.map(() => false),
    isTableLine: rawLines.map(() => false),
    isStructural: rawLines.map(() => false),
    groupId,
    identifiers,
    commentBlocks,
    words: countWords(prose.join('\n')),
  };
}

/**
 * Split `f6_missing_config` / `fooBarBaz` into words, keeping each word's column.
 * @param {string} ident
 * @param {number} base
 */
function splitIdentifier(ident, base) {
  /** @type {{text:string,col:number}[]} */
  const out = [];
  for (const chunk of ident.split(/(_)/)) {
    if (chunk === '_' || chunk === '') { base += chunk.length; continue; }
    // `[A-Z]+[0-9]*(?![a-z])` keeps `F6` in `F6_MODE` and `parseF6Config` whole;
    // without the digits the token split to `F` + `6` and never reached 4.1.
    const re = /[A-Z]+[0-9]*(?![a-z])|[A-Z][a-z0-9]*|[a-z0-9]+/g;
    let m;
    while ((m = re.exec(chunk)) !== null) out.push({ text: m[0], col: base + m.index });
    base += chunk.length;
  }
  return out;
}

/**
 * Files whose prose lives in `#` comments: YAML, TOML, shell, systemd units,
 * maintainer scripts, lintian overrides. Unit files additionally carry prose in
 * `Description=`; TOML files carry it in `description` and
 * `extended-description`, which become the `Description:` field of the built
 * `.deb` — the single most public sentence the package ships.
 * @param {string} text
 * @param {'hash'|'unit'|'toml'} kind
 * @returns {Extraction}
 */
function extractHashComments(text, kind) {
  const rawLines = text.split('\n');
  /** closing delimiter of a `"""…"""` value opened on an earlier line, or null */
  let multilineClose = /** @type {string|null} */ (null);
  const prose = rawLines.map((raw) => {
    if (multilineClose) {
      const close = raw.indexOf(multilineClose);
      if (close < 0) return raw;
      multilineClose = null;
      return raw.slice(0, close) + blank(raw.slice(close));
    }
    if (kind === 'toml') {
      const m = raw.match(/^(\s*(?:extended-)?description\s*=\s*)(.*)$/i);
      if (m) {
        const head = ' '.repeat(m[1].length);
        const rest = m[2];
        if (rest.startsWith('"""') || rest.startsWith("'''")) {
          const delim = rest.slice(0, 3);
          const end = rest.indexOf(delim, 3);
          if (end < 0) { multilineClose = delim; return head + '   ' + rest.slice(3); }
          return head + '   ' + rest.slice(3, end) + blank(rest.slice(end));
        }
        const q = rest[0];
        if (q === '"' || q === "'") {
          const end = rest.lastIndexOf(q);
          if (end > 0) return head + ' ' + rest.slice(1, end) + blank(rest.slice(end));
        }
        return blank(raw);
      }
    }
    const idx = unquotedHash(raw);
    if (idx >= 0) {
      const after = raw[idx + 1] === ' ' ? idx + 2 : idx + 1;
      return ' '.repeat(after) + raw.slice(after);
    }
    if (kind === 'unit') {
      const m = raw.match(/^(Description\s*=\s*)(.*)$/);
      if (m) return ' '.repeat(m[1].length) + m[2];
    }
    return blank(raw);
  // Backticks quote a command or a flag in a `#` comment exactly as they do in
  // Markdown: the content is a literal and never reaches a rule.
  }).map(blankCodeSpans);
  /** A run of whole-line `#` comments is one block; a trailing one stands alone. */
  /** @type {number[]} */
  const groupId = rawLines.map(() => -1);
  let gid = 0;
  let inRun = false;
  rawLines.forEach((raw, idx) => {
    const isComment = /^\s*#/.test(raw) && !/^#!/.test(raw);
    if (!isComment) { inRun = false; return; }
    if (!inRun) gid++;
    groupId[idx] = gid;
    inRun = true;
  });
  return {
    kind, rawLines, prose,
    isHeading: rawLines.map(() => false),
    isTableLine: rawLines.map(() => false),
    isStructural: rawLines.map(() => false),
    groupId,
    identifiers: [],
    commentBlocks: hashBlocks(rawLines),
    words: countWords(prose.join('\n')),
  };
}

/**
 * Index of the first `#` outside quotes, or -1. A `#` in the very first line of
 * a shebang is not a comment worth linting, but it costs nothing to keep.
 * @param {string} line
 */
function unquotedHash(line) {
  let q = '';
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (q) {
      if (c === '\\') { i++; continue; }
      if (c === q) q = '';
      continue;
    }
    if (c === '"' || c === "'") { q = c; continue; }
    if (c === '#') return i;
  }
  return -1;
}

/**
 * Man page: lines that do not start with a dot are prose; `.SH`/`.TH` lines are
 * headings. Roff escapes are blanked in place so columns survive.
 * @param {string} text
 * @returns {Extraction}
 */
function extractMan(text) {
  const rawLines = text.split('\n');
  const prose = [];
  const isHeading = [];
  const isStructural = [];
  for (const raw of rawLines) {
    const macro = raw.match(/^\.(\w+)\s*/);
    if (macro) {
      const name = macro[1].toUpperCase();
      if (name === 'SH' || name === 'SS' || name === 'TH') {
        prose.push(' '.repeat(macro[0].length) + deroff(raw.slice(macro[0].length)));
        isHeading.push(true);
        isStructural.push(name === 'TH');
        continue;
      }
      if (MAN_TEXT_MACROS.has(name)) {
        // `.B the file extension decides which:` is a sentence, not a directive.
        // Blanking the whole line hid every word a font macro carries.
        prose.push(' '.repeat(macro[0].length) + deroff(raw.slice(macro[0].length)));
        isHeading.push(false);
        isStructural.push(false);
        continue;
      }
      prose.push(blank(raw));
      isHeading.push(false);
      isStructural.push(false);
      continue;
    }
    prose.push(deroff(raw));
    isHeading.push(false);
    isStructural.push(false);
  }
  return {
    kind: 'man', rawLines, prose, isHeading,
    isTableLine: rawLines.map(() => false),
    isStructural,
    // roff lines are their own units: a `.B` argument is a macro call, and a
    // stray backtick in roff is a quote character, not a code span.
    groupId: rawLines.map(() => -1),
    identifiers: [], commentBlocks: [],
    words: countWords(prose.join('\n')),
  };
}

/** Blank roff escapes without changing the string length. @param {string} s */
function deroff(s) {
  return s.replace(/\\f[BIPR]/g, '   ').replace(/\\[&*-]/g, '  ').replace(/["]/g, ' ');
}

/**
 * JSON-ish shipped config (`exhibit.json.default`): string *values* are prose.
 * @param {string} text
 * @returns {Extraction}
 */
function extractJsonValues(text) {
  const n = text.length;
  const mask = new Array(n).fill(' ');
  for (let i = 0; i < n; i++) if (text[i] === '\n') mask[i] = '\n';
  let i = 0;
  while (i < n) {
    if (text[i] === '"') {
      const start = i + 1;
      let j = start;
      while (j < n && text[j] !== '"') { if (text[j] === '\\') j++; j++; }
      // a value is a string whose closing quote is not followed by a colon
      let k = j + 1;
      while (k < n && /\s/.test(text[k])) k++;
      if (text[k] !== ':') {
        for (let p = start; p < j; p++) if (text[p] !== '\n') mask[p] = text[p];
      }
      i = j + 1;
      continue;
    }
    i++;
  }
  const rawLines = text.split('\n');
  return {
    kind: 'json', rawLines, prose: mask.join('').split('\n'),
    isHeading: rawLines.map(() => false),
    isTableLine: rawLines.map(() => false),
    isStructural: rawLines.map(() => false),
    groupId: rawLines.map(() => -1),
    identifiers: [], commentBlocks: [],
    words: countWords(mask.join('')),
  };
}

/** Plain text (changelog, README without markdown structure). @param {string} text @returns {Extraction} */
function extractPlain(text) {
  const rawLines = text.split('\n');
  return {
    kind: 'text', rawLines, prose: rawLines.slice(),
    isHeading: rawLines.map(() => false),
    isTableLine: rawLines.map(() => false),
    isStructural: rawLines.map(() => false),
    groupId: rawLines.map(() => -1),
    identifiers: [], commentBlocks: hashBlocks(rawLines),
    words: countWords(text),
  };
}

/** Runs of `//!` or `///` lines. @param {string[]} rawLines */
function docCommentBlocks(rawLines) {
  /** @type {{start:number,end:number,kind:string}[]} */
  const blocks = [];
  let start = -1;
  let kind = '';
  rawLines.forEach((raw, idx) => {
    const m = raw.match(/^\s*(\/\/[!/])/);
    const k = m ? m[1] : '';
    if (k && k === kind) return;
    if (start >= 0) { blocks.push({ start: start + 1, end: idx, kind }); start = -1; kind = ''; }
    if (k) { start = idx; kind = k; }
  });
  if (start >= 0) blocks.push({ start: start + 1, end: rawLines.length, kind });
  return blocks;
}

/** Runs of `#` comment lines. @param {string[]} rawLines */
function hashBlocks(rawLines) {
  /** @type {{start:number,end:number,kind:string}[]} */
  const blocks = [];
  let start = -1;
  rawLines.forEach((raw, idx) => {
    const isComment = /^\s*#/.test(raw) && !/^#!/.test(raw);
    if (isComment) { if (start < 0) start = idx; return; }
    if (start >= 0) { blocks.push({ start: start + 1, end: idx, kind: '#' }); start = -1; }
  });
  if (start >= 0) blocks.push({ start: start + 1, end: rawLines.length, kind: '#' });
  return blocks;
}

/** Markdown has no comment blocks; keep the signature uniform. */
function hashOrSlashBlocks() { return []; }

/**
 * Group the prose lines into the units the rules scan: one comment block, one
 * Markdown paragraph. A sentence that wraps ("used to ⏎ live") is one string
 * here, so a multi-word pattern sees it whole and a pattern whose exclusion
 * lives on the next line ("the bench ⏎ escape hatch") is excluded correctly.
 * @param {Extraction} ex
 * @returns {Segment[]}
 */
function buildSegments(ex) {
  /** @type {Segment[]} */
  const segs = [];
  const n = ex.prose.length;
  for (let i = 0; i < n;) {
    if (!ex.prose[i].trim()) { i++; continue; }     // a blank line ends the unit
    const g = ex.groupId[i];
    const lines = [i];
    i++;
    if (g >= 0) {
      while (i < n && ex.groupId[i] === g && ex.prose[i].trim()) { lines.push(i); i++; }
    }
    segs.push(makeSegment(ex, lines));
  }
  return segs;
}

/**
 * Join one unit's lines with a single space, remembering where each line's
 * content starts so a match offset maps back to a real line and column.
 * @param {Extraction} ex
 * @param {number[]} lines
 * @returns {Segment}
 */
function makeSegment(ex, lines) {
  let text = '';
  /** @type {number[]} */
  const offsets = [];
  /** @type {number[]} */
  const startCols = [];
  for (let k = 0; k < lines.length; k++) {
    const line = ex.prose[lines[k]];
    const start = line.length - line.trimStart().length;
    if (k > 0) text += ' ';
    offsets.push(text.length);
    startCols.push(start);
    text += line.trimEnd().slice(start);
  }
  return { lines, text, offsets, startCols };
}

/**
 * Offset within a segment → 1-based line and column in the source file.
 * @param {Segment} seg
 * @param {number} off
 */
function segPos(seg, off) {
  let lo = 0;
  let hi = seg.offsets.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (seg.offsets[mid] <= off) lo = mid; else hi = mid - 1;
  }
  return { line: seg.lines[lo] + 1, col: seg.startCols[lo] + (off - seg.offsets[lo]) + 1 };
}

/**
 * @param {string} filePath
 * @param {string} text
 * @returns {Extraction}
 */
function extract(filePath, text) {
  const ex = extractByKind(filePath, text);
  ex.segments = buildSegments(ex);
  return ex;
}

/**
 * @param {string} filePath
 * @param {string} text
 * @returns {Extraction}
 */
function extractByKind(filePath, text) {
  const base = path.basename(filePath);
  const ext = path.extname(base).toLowerCase();
  if (ext === '.md' || ext === '.markdown') return extractMarkdown(text);
  if (ext === '.rs') return extractRust(text);
  if (ext === '.1' || /\.[1-9]$/.test(base)) return extractMan(text);
  if (ext === '.service' || ext === '.socket' || ext === '.timer') return extractHashComments(text, 'unit');
  if (ext === '.toml') return extractHashComments(text, 'toml');
  if (ext === '.yml' || ext === '.yaml' || ext === '.sh') return extractHashComments(text, 'hash');
  if (/^\s*[{[]/.test(text)) return extractJsonValues(text);
  if (/^#!/.test(text)) return extractHashComments(text, 'hash');
  if (/^changelog/i.test(base)) return extractPlain(text);
  if (/^\s*#/.test(text)) return extractHashComments(text, 'hash');
  return extractPlain(text);
}

// ---------------------------------------------------------------------------
// Scanning helpers used by the rules
// ---------------------------------------------------------------------------

/**
 * Run a regex over every prose *unit* — one comment block, one Markdown
 * paragraph — calling back with the match and the line:col where it starts.
 *
 * Scanning per line missed every pattern a line break fell inside ("used to ⏎
 * live", "three ⏎ adversarial reviews") and reported the ones whose exclusion
 * sat on the next line ("the bench ⏎ escape hatch" is the `--bench-*` CLI
 * mode). The unit is joined with a single space per line break, so a wrapped
 * sentence reads as the sentence the writer wrote.
 * @param {FileCtx} f
 * @param {RegExp} re                 must carry the g flag
 * @param {(m: RegExpExecArray, line: number, col: number) => void} cb
 * @param {{skipQuoted?: boolean, skipHeadings?: boolean, skipStructural?: boolean}} [opts]
 */
function scanProse(f, re, cb, opts = {}) {
  for (const seg of f.ex.segments || []) {
    if (opts.skipStructural && seg.lines.some((i) => f.ex.isStructural[i])) continue;
    if (opts.skipHeadings && seg.lines.some((i) => f.ex.isHeading[i])) continue;
    const quoted = opts.skipQuoted ? quotedRanges(seg.text) : [];
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(seg.text)) !== null) {
      if (m[0].length === 0) { re.lastIndex++; continue; }
      if (opts.skipQuoted && inRanges(quoted, m.index)) continue;
      const pos = segPos(seg, m.index);
      cb(m, pos.line, pos.col);
    }
  }
}

/**
 * @param {FileCtx} f
 * @param {number} line
 * @param {number} col
 * @param {Severity} severity
 * @param {string} rule
 * @param {string} message
 * @param {string} token
 * @returns {Finding}
 */
function finding(f, line, col, severity, rule, message, token) {
  return { path: f.path, line, col, severity, rule, message, token };
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/** @type {{id: string, describe: string, run: (f: FileCtx) => Finding[]}[]} */
const RULES = [
  {
    id: 'plan-codes',
    describe: 'private plan item codes (F6, T7, M5, C1) in public text',
    run(f) {
      const out = [];
      // The lookahead keeps metric fastener sizes out: `M2.5 standoffs` and
      // `M3 screws` are the Pi mounting hardware, not plan item M2 or M3.
      scanProse(f, /\b(?!M[0-9](?:\.[0-9]| (?:screw|standoff|bolt|nut)))[FTMC][0-9]{1,2}\b/g, (m, line, col) => {
        out.push(finding(f, line, col, 'E', 'plan-codes',
          'plan item code in public text — name the thing instead', m[0]));
      });
      for (const id of f.ex.identifiers) {
        if (RUST_FLOAT_TYPES.has(id.text.toLowerCase())) continue;   // `f32`/`f64` are types
        if (/^[ftmc][0-9]{1,2}$/i.test(id.text)) {
          out.push(finding(f, id.line, id.col, 'E', 'plan-codes',
            'plan item code in an identifier — name the thing instead', id.text));
        }
      }
      return out;
    },
  },

  {
    id: 'private-refs',
    describe: 'pointers to private documents, sections, reviews and numbered principles',
    run(f) {
      const out = [];
      /** @type {[RegExp, string][]} */
      const pats = [
        [/\b(SPEC|PLAN|IMPLEMENTATION-PLAN)(\.md)?\b/g, 'private plan document'],
        [/\bLOG-4k[\w-]*\b/g, 'private running log'],
        [/\bSTORY-[\w-]+\.md\b/g, 'private story file'],
        [/§\s?[0-9]/g, 'section sign — public docs cite headings, not §'],
        [/\bprinciple\s+[0-9]\b/gi, 'numbered principle from a private document'],
        [/\bbug\s+#[0-9]+\b/gi, 'private bug number'],
        [/\b(adversarial|gallery-ops|rust) review\b/gi, 'review event, not a citation'],
        [/\bDOSSIER-?[0-9]\b/g, 'private dossier'],
      ];
      for (const [re, why] of pats) {
        scanProse(f, re, (m, line, col) => {
          out.push(finding(f, line, col, 'E', 'private-refs',
            `${why} — restate the fact or link a file under docs/design/`, m[0]));
        });
      }
      scanProse(f, /\bthe (story|plan|brief|task brief|hardening plan|design)\b/gi, (m, line, col) => {
        const isDesign = /design/i.test(m[1]);
        out.push(finding(f, line, col, isDesign ? 'W' : 'E', 'private-refs',
          'pointer to a document the reader does not have — restate the fact', m[0]));
      });
      return out;
    },
  },

  {
    id: 'dates',
    describe: 'dates used as structure or as justification',
    run(f) {
      if (datesAllowedInPath(f.path)) return [];
      const out = [];
      const ISO = /\b20[0-9]{2}-[0-9]{2}-[0-9]{2}\b/;
      f.ex.prose.forEach((line, idx) => {
        const raw = f.ex.rawLines[idx];
        if (f.ex.isStructural[idx]) return;                    // roff .TH line
        if (/copyright|\(c\)|©|SPDX-FileCopyrightText/i.test(raw)) return;  // licence year lines
        const lineNo = idx + 1;
        if (f.ex.isHeading[idx]) {
          // Headings only: "(later)" is the time-of-day split this rule is
          // about. In running text it is an ordinary parenthesis — a hardware
          // table's "Pi 2 B (later)" says which revision, not which evening.
          const tod = /\((later|evening|later still|morning|night)\)/gi;
          let t;
          while ((t = tod.exec(line)) !== null) {
            out.push(finding(f, lineNo, t.index + 1, 'E', 'dates',
              'time-of-day split — headings name what, never when', t[0]));
          }
          const m = line.match(ISO);
          if (m) {
            out.push(finding(f, lineNo, (m.index || 0) + 1, 'E', 'dates',
              'date in a heading — structure by subject; dates belong in the changelog', m[0]));
            return;
          }
        }
        const just = /\b(as of|since|until|before|after|on)\s+(20[0-9]{2}-[0-9]{2}-[0-9]{2})\b/gi;
        let m;
        let reported = false;
        while ((m = just.exec(line)) !== null) {
          out.push(finding(f, lineNo, m.index + 1, 'E', 'dates',
            'date as justification — describe current behaviour without the date', m[0]));
          reported = true;
        }
        if (reported) return;
        const other = /\b20[0-9]{2}-[0-9]{2}-[0-9]{2}\b/g;
        while ((m = other.exec(line)) !== null) {
          out.push(finding(f, lineNo, m.index + 1, 'W', 'dates',
            'ISO date in shipped text — provenance belongs in the changelog or the measurement record', m[0]));
        }
      });
      return out;
    },
  },

  {
    id: 'caps',
    describe: 'ALL-CAPS used for emphasis rather than for an acronym or a constant',
    run(f) {
      const out = [];
      const isMan = f.ex.kind === 'man';
      const isMeasurements = isMeasurementRecord(f.path);
      f.ex.prose.forEach((line, idx) => {
        if (!line.trim()) return;
        if (f.ex.isStructural[idx]) return;
        // roff uppercases every section heading, custom ones included (§4.4)
        if (isMan && f.ex.isHeading[idx]) return;
        const re = /[A-Z][A-Z-]*[A-Z]/g;
        let m;
        while ((m = re.exec(line)) !== null) {
          const token = m[0];
          if (token.length < 3) continue;
          const before = line[m.index - 1];
          const after = line[m.index + token.length];
          if (before && /[A-Za-z0-9_-]/.test(before)) continue;         // inside a longer token
          if (after && /[A-Za-z0-9_=-]/.test(after)) continue;          // HDMI-A-1, READY=1, MPV_EVENT_…
          // A name a substitution reads, exactly like `READY=1` above: the
          // Rust format placeholder `{USAGE}` and the environment references
          // `$HOME` / `${CARGO_HOME}` all name a constant, never emphasis.
          if (before === '{' && after === '}') continue;
          if (before === '$') continue;
          if (/^\.[a-z]{1,4}\b/.test(line.slice(m.index + token.length))) continue;   // a file name: PLAN.md
          if (f.ctx.acronyms.has(token)) continue;
          if (RUST_CAPS.has(token)) continue;
          if (isMan && MAN_SECTIONS.has(token)) continue;
          if (isMeasurements && VERDICT_CAPS.has(token)) continue;
          const dictionary = f.ctx.dict.has(token.toLowerCase()) || CAPS_DICT_FALLBACK.has(token);
          const message = dictionary
            ? 'dictionary-word emphasis — emphasise by word order, not typography'
            : 'unknown acronym — add to glossary or allowlist';
          out.push(finding(f, idx + 1, m.index + 1, 'E', 'caps', message, token));
        }
      });
      return out;
    },
  },

  {
    id: 'bold',
    describe: 'bold used for emphasis rather than for a literal',
    run(f) {
      if (f.ex.kind !== 'md') return [];
      const out = [];
      let spans = 0;
      /** @type {{line:number,col:number,token:string}|null} */
      let first = null;
      f.ex.prose.forEach((line, idx) => {
        if (f.ex.isTableLine[idx]) return;
        const re = /\*\*([^*`]+)\*\*/g;
        let m;
        while ((m = re.exec(line)) !== null) {
          const content = m[1].trim();
          if (!content) continue;                                           // bold around a code span
          if (content.includes('/') || content.startsWith('-')) continue;   // a path or a flag
          spans++;
          if (!first) first = { line: idx + 1, col: m.index + 1, token: m[0] };
          if (BOLD_VERB_RE.test(content) && content.split(/\s+/).length <= 3) {
            out.push(finding(f, idx + 1, m.index + 1, 'E', 'bold',
              'bold on a verb or a negation — rewrite the sentence so the word order carries it', m[0]));
          }
        }
      });
      const per1k = f.ex.words ? (spans / f.ex.words) * 1000 : 0;
      if (per1k > 3 && first) {
        out.push(finding(f, first.line, first.col, 'W', 'bold',
          `${spans} bold spans outside tables = ${per1k.toFixed(1)} per 1000 words (limit 3)`, first.token));
      }
      return out;
    },
  },

  {
    id: 'intensifiers',
    describe: 'house intensifiers that carry no information',
    run(f) {
      const out = [];
      /** @type {RegExp[]} */
      const errors = [
        /\bdeliberate(ly)?\b/gi,
        /\bgenuinely\b/gi,
        /\bload-bearing\b/gi,
        /\bthe (whole|entire) (point|story|trick|reason|payoff)\b/gi,
        /\bmeasured,? not (argued|assumed|inferred)\b/gi,
        /\b(which|that) is the point\b/gi,
        /\bworth (recording|stating|keeping straight|noting)\b/gi,
        /\bnot incidental\b/gi,
      ];
      for (const re of errors) {
        scanProse(f, re, (m, line, col) => {
          out.push(finding(f, line, col, 'E', 'intensifiers',
            'house intensifier — if the sentence loses nothing without the word, the word goes', m[0]));
        });
      }
      // §4.5 names one false positive for this word: "be honest with me" inside
      // quoted speech is the speaker's word, not the house tic.
      scanProse(f, /\bhonest(ly)?\b/gi, (m, line, col) => {
        out.push(finding(f, line, col, 'E', 'intensifiers',
          'house intensifier — if the sentence loses nothing without the word, the word goes', m[0]));
      }, { skipQuoted: true });
      /** @type {Finding[]} */
      const exactly = [];
      scanProse(f, /\bexactly\b(?!\s+(one|two|three|zero|[0-9]|the same|as|like|when|where|what|which|at|once))/gi,
        (m, line, col) => {
          exactly.push(finding(f, line, col, 'W', 'intensifiers',
            '"exactly" not in front of a number — usually removable', m[0]));
        });
      const per1k = f.ex.words ? (exactly.length / f.ex.words) * 1000 : 0;
      if (per1k > 1) out.push(...exactly);
      return out;
    },
  },

  {
    id: 'contrast',
    describe: 'density of "not X, it is Y" constructions',
    run(f) {
      /** @type {Finding[]} */
      const hits = [];
      /** @type {RegExp[]} */
      const pats = [
        /\brather than\b/gi,
        /\bnot merely\b/gi,
        /\bnot (a|an|the) [^.;:]{1,40}, (it is|but|it's|rather)\b/gi,
        /,\s+not\s+(a|an|the|by|from|to)\b/gi,
        /\bnot\s+"[^"]+",\s+(it is|but)\b/gi,
      ];
      for (const re of pats) {
        scanProse(f, re, (m, line, col) => {
          hits.push(finding(f, line, col, 'W', 'contrast',
            'says what it is not — state what it is, unless the reader plausibly believes otherwise', m[0]));
        });
      }
      const per1k = f.ex.words ? (hits.length / f.ex.words) * 1000 : 0;
      if (per1k <= 2) return [];
      hits.sort((a, b) => a.line - b.line || a.col - b.col);
      const top = hits.slice(0, 10);
      top[0] = { ...top[0], message: `${top[0].message} [${hits.length} in this file = ${per1k.toFixed(1)} per 1000 words, limit 2; top 10 shown]` };
      return top;
    },
  },

  {
    id: 'coinages',
    describe: 'project coinages that have a plain replacement',
    run(f) {
      const out = [];
      for (const c of f.ctx.coinages) {
        scanProse(f, c.re, (m, line, col) => {
          out.push(finding(f, line, col, 'E', 'coinages',
            `coinage — use: ${c.replacement}`, m[0]));
        });
        for (const id of f.ex.identifiers) {
          c.re.lastIndex = 0;
          if (c.re.test(id.text)) {
            out.push(finding(f, id.line, id.col, 'E', 'coinages',
              `coinage in an identifier — use: ${c.replacement}`, id.text));
          }
        }
      }
      return out;
    },
  },

  {
    id: 'codenames',
    describe: 'bench codenames and referents only a session participant can resolve',
    run(f) {
      const out = [];
      const introduced = firstIntroLine(f);
      scanProse(f, /\bdexpi[0-9](\.local)?\b/gi, (m, line, col) => {
        out.push(finding(f, line, col, 'E', 'codenames',
          'bench host name — say what the device is, generically', m[0]));
      });
      // "the pass bar" is handled once, by the coinage table, with its replacement.
      // `the bench` needs the lookahead: `--bench-only` is a shipped CLI mode, so
      // "the bench flag", "the bench branch" and "the bench escape hatch" name a
      // feature of the program, not the room the program was tested in.
      // `Cam Link(?! 4K)`: "the Cam Link 4K" names the product in full, which is
      // the introduction §4.8 asks for, so it is never the bare referent.
      scanProse(f, /\bthe (bench Pi|bench(?![- ](?:flag|branch|escape hatch|only|mode|override))|Dell|Cam Link(?! 4K)|box test|field record|task brief|session|orchestrator|implementer|reviewer)\b/gi,
        (m, line, col) => {
          // `>=`, not `>`: a file that introduces the card and uses the short
          // name in the same sentence has still introduced it.
          if (/cam link/i.test(m[1]) && introduced >= 0 && line >= introduced) return;
          out.push(finding(f, line, col, 'E', 'codenames',
            'referent from the session — introduce it generically the first time', m[0]));
        });
      // no "this run": in shipped text it means this process execution, not a bench session
      scanProse(f, /\b(today|tonight|this (morning|evening|session|room|week)|the same evening|earlier (tonight|today))\b/gi,
        (m, line, col) => {
          out.push(finding(f, line, col, 'E', 'codenames',
            'time reference that only makes sense in the session', m[0]));
        });
      scanProse(f, /\bdexeye\b|\bthermal\b(?= tmux| session)/gi, (m, line, col) => {
        out.push(finding(f, line, col, 'E', 'codenames', 'bench session name', m[0]));
      });
      if (!/docs\/design\/measurements\.md$/.test(f.path)) {
        scanProse(f, /\b[ed]0[0-9]\b/g, (m, line, col) => {
          out.push(finding(f, line, col, 'E', 'codenames',
            'test-card ID — only the measurement record may use card IDs', m[0]));
        });
      }
      return out;
    },
  },

  {
    id: 'first-person',
    describe: 'first person in shipped text',
    run(f) {
      const out = [];
      const re = /\b(I'm|I've|I'd|I|we're|we've|we|us|our|ours|my|mine|me)\b/gi;
      // §4.9 is explicit that quotes are NOT stripped here: a pronoun inside a
      // quotation still ships to the reader, so it is reported and the
      // allowlist takes the individual line. (`intensifiers` does skip quoted
      // speech, because §4.5 names "be honest with me" as a false positive.)
      scanProse(f, re, (m, line, col) => {
        const token = m[0];
        // "I" is first person only in upper case; a stray lower-case "i" is noise.
        if (token[0] === 'i') return;
        // An all-caps run is a unit, an abbreviation or a shouted heading —
        // "US", "ME", "MY", "OUR" are not the pronoun. `I` is the one word that
        // is genuinely spelled in capitals; the caps rule owns the rest.
        if (token !== 'I' && token === token.toUpperCase()) return;
        const text = f.ex.prose[line - 1] || '';
        if (token === 'I' && (text[col] === '/' || text[col - 2] === '/')) return;   // I/O
        out.push(finding(f, line, col, 'E', 'first-person',
          'first person in shipped text — state the fact and its provenance label', token));
      });
      return out;
    },
  },

  {
    id: 'process-talk',
    describe: 'session, review and revision talk in shipped text',
    run(f) {
      const out = [];
      const ext = path.extname(f.path).toLowerCase();
      /** @type {[RegExp, string][]} */
      const pats = [
        [/\b(we|I) (measured|found|read|checked|tried|chased|invented|claimed|concluded|walked past)\b/gi, 'narrates the work instead of stating the result'],
        // Spelled numbers only. `[0-9]+` read the tail of an ISO date as a
        // count and called "2026-08-17 review" a review count.
        [/\b(three|two|several) (independent |adversarial |further )?reviews?\b/gi, 'review count — belongs in the changelog or a sessionlog'],
        [/\b(an|the) earlier (draft|revision|version)\b/gi, 'revision history — shipped text describes current behaviour only'],
        [/\boriginally\b/gi, 'revision history — shipped text describes current behaviour only'],
        [/\bused to (be|have|do|hardcode|live|sit|fire|run|say)\b/gi, 'revision history — shipped text describes current behaviour only'],
        [/\bat review time\b/gi, 'review talk'],
        [/\bSettle (on bench|by reading)\b/gi, 'research to-do — belongs in an issue, not in shipped text'],
        [/\bthis (task|crate)'s (practice|hard constraint|session notes)\b/gi, 'process talk'],
      ];
      for (const [re, why] of pats) {
        scanProse(f, re, (m, line, col) => {
          out.push(finding(f, line, col, 'E', 'process-talk', why, m[0]));
        });
      }
      if (ext === '.rs' || ext === '.md') {
        scanProse(f, /\bCORRECTION\b|\bREMOVED 20|\bSUPERSEDED\b|\bSUSPECTED\b|\bMINOR\b|\bMAJOR\b/g, (m, line, col) => {
          out.push(finding(f, line, col, 'E', 'process-talk',
            'review-triage vocabulary — corrections belong in the changelog or an errata note', m[0]));
        });
      }
      return out;
    },
  },

  {
    id: 'comment-length',
    describe: 'doc-comment blocks that argue instead of describing',
    run(f) {
      const out = [];
      const ARGUE = /\b(Why|because|rather than|the obvious alternative|proposed|rejected)\b/gi;
      for (const b of f.ex.commentBlocks) {
        const len = b.end - b.start + 1;
        if (len > 40) {
          out.push(finding(f, b.start, 1, 'E', 'comment-length',
            `${b.kind} block is ${len} lines — move the rationale to docs/design/`, `${b.kind} block`));
        } else if (len > 20) {
          out.push(finding(f, b.start, 1, 'W', 'comment-length',
            `${b.kind} block is ${len} lines — a doc-comment gives the contract, not the argument`, `${b.kind} block`));
        }
        const body = f.ex.prose.slice(b.start - 1, b.end).join('\n');
        const argue = body.match(ARGUE);
        if (argue && argue.length > 2) {
          out.push(finding(f, b.start, 1, 'W', 'comment-length',
            `${argue.length} argument markers in one comment block — design docs argue, doc-comments describe`,
            argue.slice(0, 3).join(', ')));
        }
      }
      return out;
    },
  },

  {
    id: 'structure',
    describe: 'emoji and session verbs in headings',
    run(f) {
      if (f.ex.kind !== 'md') return [];   // `#` in other file types is a comment, not a heading
      const out = [];
      // Extended_Pictographic also covers ©, ®, ™ and the arrows U+2194–2199,
      // which are typography, not status markers.
      const EMOJI = /(?![\u00a9\u00ae\u2122\u2194-\u2199])\p{Extended_Pictographic}/u;
      // Read the extracted prose, not the raw lines: `# Verified on the Pi`
      // inside a ```bash fence is a shell comment, not a Markdown heading, and
      // an emoji inside a code span is a literal being shown to the reader.
      f.ex.prose.forEach((line, idx) => {
        if (!line.trim()) return;
        const lineNo = idx + 1;
        if (f.ex.isHeading[idx]) {
          const m = line.match(EMOJI);
          if (m) {
            out.push(finding(f, lineNo, (m.index || 0) + 1, 'E', 'structure',
              'emoji in a heading', m[0]));
          }
          const verb = line.match(/^\s{0,3}#{1,6}\s+(Reviewed|Resolved|Observed|Verified|Corrected)\b/);
          if (verb) {
            out.push(finding(f, lineNo, line.indexOf(verb[1]) + 1, 'E', 'structure',
              'session-verb heading — headings name the subject', verb[1]));
          }
        } else if (/^\s*[-*]\s/.test(line)) {
          const m = line.match(EMOJI);
          if (m) {
            out.push(finding(f, lineNo, (m.index || 0) + 1, 'W', 'structure',
              'emoji status marker — say the status in words', m[0]));
          }
        }
      });
      return out;
    },
  },
];

/** The measurement record: dates and verdict words are its content. @param {string} p */
function isMeasurementRecord(p) {
  return /docs\/design\/measurements\.md$/.test(p);
}

/** @param {string} p */
function datesAllowedInPath(p) {
  const base = path.basename(p).toLowerCase();
  if (base.startsWith('changelog')) return true;
  if (isMeasurementRecord(p)) return true;
  return false;
}

/** Line of the first proper introduction of the capture card, or -1. @param {FileCtx} f */
function firstIntroLine(f) {
  for (let i = 0; i < f.ex.prose.length; i++) {
    if (/Cam Link 4K|capture card/i.test(f.ex.prose[i])) return i + 1;
  }
  return -1;
}

// ---------------------------------------------------------------------------
// Glossary, allowlist and coinage table
// ---------------------------------------------------------------------------

/**
 * `### Term` headings, optionally followed by `Also written: a, b, c`.
 *
 * A heading may gloss the term inline — `### HEVC (High Efficiency Video
 * Coding)`, `### EDID — Extended Display Identification Data`, `### PATH: the
 * search path` — so the term is the heading up to the first ` (`, ` —`, ` - `
 * or `:`. Without this, the glossed headings registered nothing and every
 * acronym they defined was still reported as unknown.
 * @param {string} text
 * @returns {{acronyms: Set<string>, terms: Set<string>}}
 */
function parseGlossary(text) {
  const acronyms = new Set();
  const terms = new Set();
  const lines = text.split('\n');
  /** @param {string} head */
  const headTerm = (head) => {
    const m = head.match(/^(.*?)(?:\s+\(|\s+[—–]\s|\s+-\s|:)/);
    return (m ? m[1] : head).trim();
  };
  /** @param {string} raw */
  const addTokens = (raw) => {
    for (const tok of raw.split(/[,;/]| or /)) {
      const t = tok.trim().replace(/[.`*]/g, '');
      if (!t) continue;
      terms.add(t);
      if (/^[A-Z][A-Z0-9-]+$/.test(t) && t.length >= 2) acronyms.add(t);
    }
  };
  lines.forEach((raw, idx) => {
    const h = raw.match(/^#{2,4}\s+(.+?)\s*$/);
    if (h) {
      addTokens(headTerm(h[1]));
      for (let j = idx + 1; j < Math.min(idx + 5, lines.length); j++) {
        const also = lines[j].match(/^\s*(?:\*\*)?Also written(?:\*\*)?\s*:\s*(.+)$/i);
        if (also) addTokens(also[1]);
        if (/^#{1,6}\s/.test(lines[j])) break;
      }
    }
  });
  return { acronyms, terms };
}

/**
 * One entry per line: `<rule-id> <token-or-regex> [# reason]` or
 * `<rule-id> path=<glob> [<token-or-regex>] [# reason]`.
 * @param {string} text
 * @param {string} label   file name for error messages
 * @returns {AllowEntry[]}
 */
function parseAllowlist(text, label = 'docs/lint-allow.txt') {
  /** @type {AllowEntry[]} */
  const out = [];
  const ruleIds = new Set(RULES.map((r) => r.id));
  text.split('\n').forEach((raw, i) => {
    const lineNo = i + 1;
    const line = raw.trim();
    if (!line) return;
    if (line.startsWith('#')) return;
    const hash = line.indexOf('#');
    if (hash < 0) {
      throw new ConfigError(`${label}:${lineNo}: allowlist entry without a reason — append "# why this is allowed"`);
    }
    const reason = line.slice(hash + 1).trim();
    if (!reason) {
      throw new ConfigError(`${label}:${lineNo}: allowlist entry without a reason — append "# why this is allowed"`);
    }
    const body = line.slice(0, hash).trim();
    const parts = body.split(/\s+/).filter(Boolean);
    if (parts.length === 0) {
      throw new ConfigError(`${label}:${lineNo}: allowlist entry without a rule id`);
    }
    const rule = parts.shift() || '';
    if (!ruleIds.has(rule)) {
      throw new ConfigError(`${label}:${lineNo}: unknown rule id "${rule}"`);
    }
    let pathGlob = null;
    if (parts[0] && parts[0].startsWith('path=')) pathGlob = parts.shift().slice(5);
    const tokenText = parts.join(' ');
    let tokenRe = null;
    let tokenLiteral = null;
    if (tokenText) {
      if (tokenText.length > 1 && tokenText.startsWith('/') && tokenText.endsWith('/')) {
        tokenRe = new RegExp(tokenText.slice(1, -1));
      } else {
        tokenLiteral = tokenText;
      }
    }
    // A bare rule id with a reason parses, matches every finding of that rule in
    // every file, and silently turns the rule off repository-wide. The reason
    // requirement is the gate; without this check the gate has a hole in it.
    if (!pathGlob && !tokenText) {
      throw new ConfigError(
        `${label}:${lineNo}: allowlist entry "${rule}" has no token and no path= — a bare rule id ` +
        'silences the whole rule everywhere; name the token, or scope it with path=<glob>');
    }
    out.push({ rule, pathGlob, tokenRe, tokenLiteral, reason, lineNo });
  });
  return out;
}

/**
 * @param {Finding} finding_
 * @param {AllowEntry[]} allow
 */
function isAllowed(finding_, allow) {
  for (const a of allow) {
    if (a.rule !== finding_.rule) continue;
    if (a.pathGlob) {
      const re = globToRe(a.pathGlob);
      const p = finding_.path;
      if (!re.test(p) && !re.test(path.basename(p)) && !re.test('/' + p)) continue;
    }
    if (a.tokenRe && !a.tokenRe.test(finding_.token)) continue;
    if (a.tokenLiteral && a.tokenLiteral !== finding_.token) continue;
    return true;
  }
  return false;
}

/**
 * Two columns, `coinage<TAB>replacement`. A coinage wrapped in slashes is a
 * regex; anything else is a literal phrase matched as whole words.
 * @param {string} text
 * @returns {Coinage[]}
 */
function parseCoinages(text) {
  /** @type {Coinage[]} */
  const out = [];
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const parts = line.split('\t').map((s) => s.trim()).filter((s) => s !== '');
    if (parts.length < 2) continue;
    const [key, replacement] = parts;
    out.push({ key, re: coinageRe(key), replacement });
  }
  return out;
}

/** @param {string} key */
function coinageRe(key) {
  if (key.length > 1 && key.startsWith('/') && key.endsWith('/')) {
    return new RegExp(key.slice(1, -1), 'gi');
  }
  return new RegExp('\\b' + escapeRe(key).replace(/\s+/g, '\\s+') + '\\b', 'gi');
}

/** @returns {Coinage[]} */
function defaultCoinages() {
  return DEFAULT_COINAGES.map(([key, replacement]) => ({ key, re: coinageRe(key), replacement }));
}

// ---------------------------------------------------------------------------
// File collection
// ---------------------------------------------------------------------------

class ConfigError extends Error {}

/** @param {string} p */
function looksBinary(p) {
  const fd = fs.openSync(p, 'r');
  try {
    const buf = Buffer.alloc(8192);
    const n = fs.readSync(fd, buf, 0, 8192, 0);
    for (let i = 0; i < n; i++) if (buf[i] === 0) return true;
    return false;
  } finally {
    fs.closeSync(fd);
  }
}

/**
 * Is this file in scope when walking a directory?
 * @param {string} p
 */
function isDefaultTextFile(p) {
  const base = path.basename(p);
  if (SKIP_FILES.has(base)) return false;
  if (base.startsWith('.')) return false;
  const ext = path.extname(base).toLowerCase();
  if (ext === '.json') return false;
  const inDeploy = p.split(path.sep).includes('deploy');
  if (inDeploy) return true;                  // changelog, postinst, *.1, *.service, *.default
  if (/^\.[1-9]$/.test(ext)) return true;
  return TEXT_EXTS.has(ext);
}

/**
 * @param {string[]} inputs
 * @param {string} cwd
 * @param {Set<string>} [skip]   absolute paths never linted, named or not
 * @param {{requireExisting?: boolean}} [opts]  true when `inputs` were named on
 *   the command line: a path the caller asked for and that is not there is a
 *   configuration error, never a silently smaller scope. The built-in default
 *   paths are a superset of any one checkout, so they are collected leniently.
 * @returns {string[]} absolute paths
 */
function collectFiles(inputs, cwd, skip = new Set(), opts = {}) {
  /** @type {string[]} */
  const files = [];
  const seen = new Set();
  /** @param {string} abs @param {boolean} explicit */
  const add = (abs, explicit) => {
    if (seen.has(abs)) return;
    if (skip.has(abs)) return;
    if (!explicit && !isDefaultTextFile(abs)) return;
    if (looksBinary(abs)) return;
    seen.add(abs);
    files.push(abs);
  };
  /** @param {string} dir */
  const walk = (dir) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (SKIP_DIRS.has(ent.name)) continue;
      const abs = path.join(dir, ent.name);
      if (ent.isDirectory()) walk(abs);
      else if (ent.isFile()) add(abs, false);
    }
  };
  for (const input of inputs) {
    if (/[*?]/.test(input)) {
      const idx = input.search(/[*?]/);
      const prefix = input.slice(0, idx);
      const rootDir = path.resolve(cwd, prefix.includes('/') ? prefix.slice(0, prefix.lastIndexOf('/')) : '.');
      const re = globToRe(path.resolve(cwd, input));
      if (!fs.existsSync(rootDir)) continue;
      const stack = [rootDir];
      while (stack.length) {
        const dir = stack.pop();
        if (dir === undefined) break;
        for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
          if (SKIP_DIRS.has(ent.name)) continue;
          const abs = path.join(dir, ent.name);
          if (ent.isDirectory()) stack.push(abs);
          else if (ent.isFile() && re.test(abs)) add(abs, false);
        }
      }
      continue;
    }
    const abs = path.resolve(cwd, input);
    if (!fs.existsSync(abs)) {
      // A gate that quietly drops a path it was asked to check reports a clean
      // tree for text nobody read. Typos in a CI invocation die here.
      if (opts.requireExisting) throw new ConfigError(`input path does not exist: ${input}`);
      continue;
    }
    const st = fs.statSync(abs);
    if (st.isDirectory()) walk(abs);
    else add(abs, true);
  }
  return files;
}

// ---------------------------------------------------------------------------
// Linting one source
// ---------------------------------------------------------------------------

/**
 * @param {string} displayPath
 * @param {string} text
 * @param {LintCtx} ctx
 * @returns {Finding[]}
 */
function lintSource(displayPath, text, ctx) {
  const ex = extract(displayPath, text);
  /** @type {FileCtx} */
  const f = { path: displayPath, absPath: displayPath, text, ex, ctx };
  /** @type {Finding[]} */
  let out = [];
  for (const rule of RULES) {
    if (ctx.ruleFilter && !ctx.ruleFilter.has(rule.id)) continue;
    out = out.concat(rule.run(f));
  }
  return out.filter((x) => !isAllowed(x, ctx.allow));
}

/** @param {Partial<LintCtx>} [over] @returns {LintCtx} */
function makeCtx(over = {}) {
  return {
    acronyms: over.acronyms || new Set(),
    allow: over.allow || [],
    coinages: over.coinages || defaultCoinages(),
    dict: over.dict || new Set(),
    ruleFilter: over.ruleFilter || null,
  };
}

/** Load /usr/share/dict/words if present. @returns {Set<string>} */
function loadSystemDictionary() {
  const out = new Set();
  for (const p of ['/usr/share/dict/words', '/usr/dict/words']) {
    try {
      const text = fs.readFileSync(p, 'utf8');
      for (const w of text.split('\n')) {
        const t = w.trim().toLowerCase();
        if (t.length >= 3) out.add(t);
      }
      break;
    } catch { /* no dictionary on this box; the built-in list stands in */ }
  }
  return out;
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

const USAGE = `docs-lint — mechanical writing gate for the dex repository

  node scripts/docs-lint.mjs [options] [paths…]

Options
  --glossary FILE    glossary to read acronyms from (default docs/glossary.md)
  --allow FILE       allowlist (default docs/lint-allow.txt)
  --coinages FILE    extra coinage table, "coinage<TAB>replacement" per line
  --format text|json output format (default text)
  --warn-only        report as usual but exit 0
  --rules a,b,c      run only these rules
  -h, --help         this text

Rules: ${RULES.map((r) => r.id).join(', ')}
`;

/**
 * @param {string[]} argv
 * @returns {{glossary: string, allow: string, coinages: string|null, format: string, warnOnly: boolean, rules: string[]|null, paths: string[]}}
 */
function parseArgs(argv) {
  const opts = {
    glossary: 'docs/glossary.md',
    allow: 'docs/lint-allow.txt',
    /** @type {string|null} */ coinages: null,
    format: 'text',
    warnOnly: false,
    /** @type {string[]|null} */ rules: null,
    /** @type {string[]} */ paths: [],
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    /** @param {string} name */
    const value = (name) => {
      const v = argv[++i];
      if (v === undefined) throw new ConfigError(`${name} needs a value`);
      return v;
    };
    if (a === '--glossary') opts.glossary = value(a);
    else if (a === '--allow') opts.allow = value(a);
    else if (a === '--coinages') opts.coinages = value(a);
    else if (a === '--format') opts.format = value(a);
    else if (a === '--warn-only') opts.warnOnly = true;
    else if (a === '--rules') opts.rules = value(a).split(',').map((s) => s.trim()).filter(Boolean);
    else if (a === '-h' || a === '--help') throw new ConfigError('__help__');
    else if (a.startsWith('--')) throw new ConfigError(`unknown option ${a}`);
    else opts.paths.push(a);
  }
  if (opts.format !== 'text' && opts.format !== 'json') {
    throw new ConfigError(`--format must be text or json, got "${opts.format}"`);
  }
  if (opts.rules) {
    const known = new Set(RULES.map((r) => r.id));
    for (const r of opts.rules) if (!known.has(r)) throw new ConfigError(`unknown rule "${r}"`);
  }
  return opts;
}

/**
 * @param {string[]} argv
 * @param {{log?: (s: string) => void, err?: (s: string) => void, cwd?: string}} [io]
 * @returns {number} exit code
 */
function run(argv, io = {}) {
  const log = io.log || ((s) => process.stdout.write(s + '\n'));
  const err = io.err || ((s) => process.stderr.write(s + '\n'));
  const cwd = io.cwd || process.cwd();

  let opts;
  try {
    opts = parseArgs(argv);
  } catch (e) {
    if (e instanceof ConfigError && e.message === '__help__') { log(USAGE); return 0; }
    err(`docs-lint: ${e instanceof Error ? e.message : String(e)}`);
    err(USAGE);
    return 2;
  }

  // glossary
  let acronyms = new Set();
  const glossaryPath = path.resolve(cwd, opts.glossary);
  if (fs.existsSync(glossaryPath)) {
    acronyms = parseGlossary(fs.readFileSync(glossaryPath, 'utf8')).acronyms;
  } else {
    err(`docs-lint: warning: no glossary at ${opts.glossary} — every unknown acronym will be reported`);
  }

  // allowlist
  /** @type {AllowEntry[]} */
  let allow = [];
  const allowPath = path.resolve(cwd, opts.allow);
  if (fs.existsSync(allowPath)) {
    try {
      allow = parseAllowlist(fs.readFileSync(allowPath, 'utf8'), opts.allow);
    } catch (e) {
      err(`docs-lint: ${e instanceof Error ? e.message : String(e)}`);
      return 2;
    }
  }

  // coinages
  let coinages = defaultCoinages();
  if (opts.coinages) {
    const cPath = path.resolve(cwd, opts.coinages);
    if (!fs.existsSync(cPath)) {
      err(`docs-lint: --coinages file not found: ${opts.coinages}`);
      return 2;
    }
    const extra = parseCoinages(fs.readFileSync(cPath, 'utf8'));
    const byKey = new Map(coinages.map((c) => [c.key, c]));
    for (const c of extra) byKey.set(c.key, c);
    coinages = [...byKey.values()];
  }

  const ctx = makeCtx({
    acronyms,
    allow,
    coinages,
    dict: loadSystemDictionary(),
    ruleFilter: opts.rules ? new Set(opts.rules) : null,
  });

  const inputs = opts.paths.length ? opts.paths : DEFAULT_PATHS;
  // The glossary is this run's own configuration. It lists the coinages the
  // linter retires, in the form the parser reads ("Also written: seam, the
  // wrap"), so linting it fails the gate on the file that defines the gate.
  /** @type {string[]} */
  let files;
  try {
    files = collectFiles(inputs, cwd, new Set([glossaryPath]),
      { requireExisting: opts.paths.length > 0 });
  } catch (e) {
    err(`docs-lint: ${e instanceof Error ? e.message : String(e)}`);
    return 2;
  }
  if (files.length === 0) {
    err('docs-lint: no files to lint');
    return 2;
  }

  /** @type {Finding[]} */
  let findings = [];
  for (const abs of files) {
    const rel = path.relative(cwd, abs);
    const display = rel.startsWith('..') ? abs : rel;
    let text;
    try {
      text = fs.readFileSync(abs, 'utf8');
    } catch (e) {
      err(`docs-lint: cannot read ${display}: ${e instanceof Error ? e.message : String(e)}`);
      return 2;
    }
    findings = findings.concat(lintSource(display, text, ctx));
  }

  findings.sort((a, b) => a.path.localeCompare(b.path) || a.line - b.line || a.col - b.col || a.rule.localeCompare(b.rule));

  const errors = findings.filter((x) => x.severity === 'E').length;
  const warnings = findings.length - errors;

  if (opts.format === 'json') {
    log(JSON.stringify(findings, null, 2));
  } else {
    for (const x of findings) {
      log(`${x.path}:${x.line}:${x.col} [${x.severity}] ${x.rule}: ${x.message} (${x.token})`);
    }
    log('');
    log(`-- docs-lint summary --`);
    log(`files scanned: ${files.length}   errors: ${errors}   warnings: ${warnings}`);
    /** @type {Map<string, {E: number, W: number}>} */
    const perRule = new Map();
    for (const x of findings) {
      const e = perRule.get(x.rule) || { E: 0, W: 0 };
      e[x.severity]++;
      perRule.set(x.rule, e);
    }
    const ids = [...perRule.keys()].sort((a, b) =>
      (perRule.get(b).E + perRule.get(b).W) - (perRule.get(a).E + perRule.get(a).W));
    for (const id of ids) {
      const e = perRule.get(id);
      log(`  ${id.padEnd(16)} E:${String(e.E).padStart(4)}  W:${String(e.W).padStart(4)}`);
    }
  }

  if (opts.warnOnly) return 0;
  return errors > 0 ? 1 : 0;
}

/**
 * True when this file was started as the program, false when it was imported.
 * Both sides go through `realpathSync` so that a symlinked script path, a
 * relative argv[1], and a path containing spaces or other characters that
 * `new URL(...).pathname` would percent-encode all compare equal.
 * @returns {boolean}
 */
function isMainModule() {
  try {
    if (!process.argv[1]) return false;
    return fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

// `process.exitCode`, never `process.exit()`: stdout to a pipe is asynchronous,
// and exiting truncates the report at the pipe buffer (64 KB on macOS and
// Linux). `… --format json | jq` then reads invalid JSON and a CI log capture
// loses everything past the first 64 KB, while the exit code still says 1.
// Setting the code lets Node drain the pipe and exit with it.
if (isMainModule()) process.exitCode = run(process.argv.slice(2));

export {
  run,
  parseArgs,
  parseGlossary,
  parseAllowlist,
  parseCoinages,
  defaultCoinages,
  collectFiles,
  extract,
  lintSource,
  makeCtx,
  isAllowed,
  globToRe,
  splitIdentifier,
  ConfigError,
  RULES,
};
