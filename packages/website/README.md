# website

The dex project site, <https://dex.ars.is>. Astro, static output, deployed to GitHub Pages by
[`.github/workflows/pages.yml`](../../.github/workflows/pages.yml) on every push to `main` that
touches this directory.

## Editing the content

The page is assembled from markdown files in [`src/content/blocks/`](src/content/blocks). One file
per block:

```
00-header.md         <header> — the h1, the tagline, and the <head> metadata
10-lead.md           what it does, who it is for
20-facts.md          the bullet list
30-status.md         <section> — version
40-documentation.md  <section> — links out
90-footer.md         <footer> — credits
```

Edit the markdown. That is the whole workflow — there is no separate template to update.

Frontmatter:

| key | default | meaning |
|---|---|---|
| `slot` | `main` | which landmark the block renders into: `header`, `main` or `footer` |
| `section` | `false` | wrap the block in its own `<section>`; use it for main blocks that open with a heading |
| `title` | — | the `<title>` and browser tab text. Set on one block only |
| `description` | — | the `<meta name="description">` |

**Order comes from the filename.** Blocks are sorted by id, so the numeric prefixes decide what
appears where. Renaming a file reorders the page and there is no index to keep in sync. Gaps in the
numbering are deliberate: they leave room to insert without renaming neighbours.

To add a block, drop in a file. To park one without deleting it, rename it to start with `_` — the
loader skips those.

The schema lives in [`src/content.config.ts`](src/content.config.ts) and is enforced at build time,
so a typo in `slot:` fails the build rather than silently dropping the block.

## Styling

The base stylesheet is [`@eins78/styles`](https://github.com/eins78/styles) (CC0), a dependency
pinned to a commit:

```json
"@eins78/styles": "github:eins78/styles#421dc0969409257e13e6370554e9a4a589b55bc5"
```

Pinned by hash, so the page cannot change when that repo does, and `pnpm-lock.yaml` records the
resolved tarball. To take a newer version, change the hash and run `pnpm install` — a visible,
reviewable commit rather than a silent drift.

[`src/styles/dex.css`](src/styles/dex.css) then overrides its three colour custom properties with
the dex brand pair, and gives headings a heavy grotesque. Both are imported in
`src/pages/index.astro`, **in that order** — the cascade depends on it.

The palette is the pair recorded in [`packages/branding`](../branding): a randoma11y result saved
2024-04-11, `#8ed1de` on `#2922c8`, with the light theme its exact inverse. Because the two colours
are simply swapped between modes, both halves measure the same — body 5.71:1, AA.

Body copy stays monospace, as cloud-docs sets it. Headings are `system-ui` at weight 800 with tight
tracking, following the branding sketch. `system-ui` rather than a webfont because it costs no
bytes and no extra files, and resolves to SF Pro, Segoe UI or Roboto — all grotesques. The exact
face differs by platform; the character does not.

Note the upstream variable is `--color--link`, with two hyphens. An override that writes one hyphen
fails silently and falls back to upstream white/black.

`astro.config.mjs` sets `build.inlineStylesheets: 'always'`, so the CSS ends up inside the HTML
rather than as an `_astro/` asset. On a one-page site a separate request buys nothing, and with no
build-time assets the page has no absolute `/_astro/` URLs — it renders correctly at `dex.ars.is`
and at the `kte.github.io/dex/` fallback alike. The whole built site is two files: `index.html` and
`icon.svg`.

### Palette previews

Three alternative palettes can be previewed with a query parameter:

| URL | palette | contrast (body) |
|---|---|---|
| *(none)* | the brand pair, as recorded | 5.71:1 both — AA |
| `?theme=amber` | amber on black, the first palette this site shipped | 9.27:1 dark, 16.11:1 light — AAA |
| `?theme=brand-deep` | same hues, darker ground | 9.60:1 dark, 11.41:1 light — AAA |
| `?theme=brand-day` | the recorded pair by day, deeper night | 12.39:1 dark, 5.71:1 light |

`?theme=brand` is a no-op rather than an error: the brand pair *is* the default.

In `brand-deep`, links are the brand indigo rather than black, so they read as brighter than the
body text instead of heavier — black against that already-dark navy inverted the usual affordance.

They are defined as `[data-theme="…"]` blocks at the end of `src/styles/dex.css`, selected by a
short inline script in `src/pages/index.astro`. **The script needs `is:inline`** — without it Astro
bundles it into an `_astro/` chunk, which would be the page's only build-time asset and would break
at the `kte.github.io/dex/` fallback path.

The script only accepts `/^[a-z-]{1,20}$/`; anything else is ignored and the default palette stands.
Verified in a browser across both colour schemes: each parameter produces exactly its declared
colours, and a malformed value falls back.

To drop the previews once a palette is settled, delete that CSS section and the script.

## Local development

```sh
pnpm install
pnpm dev      # http://localhost:4321
pnpm build    # → dist/
pnpm check    # type-check the astro + content schema
```

## Custom domain

There is no `CNAME` file, and adding one would do nothing: GitHub ignores `CNAME` files when a site
is published from a workflow. The domain is stored in the repository's Pages settings instead.
