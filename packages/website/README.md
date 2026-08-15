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

[`public/style.css`](public/style.css) vendors
[`eins78/styles/cloud-docs.css`](https://github.com/eins78/styles/blob/main/cloud-docs.css) (CC0),
pinned at commit `421dc09`, then overrides its three custom properties for an amber-on-black
palette. It is copied rather than linked so the page cannot change when that repo does.

Note the upstream variable is `--color--link`, with two hyphens. An override that writes one hyphen
fails silently and falls back to upstream white/black.

The stylesheet and favicon live in `public/` and are referenced with relative URLs, so the page
renders correctly both at `dex.ars.is` and at the `kte.github.io/dex/` fallback.

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
