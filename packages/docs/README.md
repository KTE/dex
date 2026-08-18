# docs

The dex documentation site, <https://dex.ars.is/docs>. [Starlight](https://starlight.astro.build)
on Astro, static output.

Starlight is an Astro integration, so this package and
[`packages/website`](../website) build with the same toolchain and publish to
one origin under one DNS record.

## Where the text lives

The markdown is the repository root [`docs/`](../../docs) directory. Nothing in this package holds
documentation text. `docs/` is what CODEOWNERS routes for review and what `scripts/docs-lint.mjs`
checks, and a page reads correctly on GitHub as well as on the site.

`scripts/sync-docs.mjs` copies that markdown into `src/content/docs/` and adds the frontmatter
Starlight requires. The output directory is generated and git-ignored; `pnpm dev` and `pnpm build`
run the copy first.

Each page opens with a level-one heading and carries no frontmatter. The script takes the title
from that heading and drops the heading, because Starlight renders the title itself. A file that
starts with frontmatter, or with anything other than a level-one heading, fails the copy with the
file name in the message.

The site's own pages live in `src/site/` and carry their own frontmatter. The landing page is the
one such page.

| Source | Becomes |
|---|---|
| `docs/guides/*.md` | `/docs/guides/…` — user tier |
| `docs/design/*.md` | `/docs/design/…` — developer tier |
| `docs/glossary.md` | `/docs/glossary/` |
| `packages/docs/src/site/index.md` | `/docs/` |

### Adding a page

Write the markdown under `docs/`, starting with a level-one heading. A page under `guides/` or
`design/` appears in the sidebar without further configuration. A page elsewhere needs a line in
the `sidebar` array in `astro.config.mjs`.

## Styling

[`src/styles/dex.css`](src/styles/dex.css) carries the brand. It is the only stylesheet this
package adds, and it works by overriding Starlight's `--sl-*` custom properties, leaving
Starlight's markup alone.

The palette is the pair recorded in [`packages/branding`](../branding), `#8ed1de` and `#2922c8`,
the same pair the project site uses. Body copy is monospace and headings are `system-ui` at weight
800, also following the project site. `system-ui` costs no bytes and no extra files, and resolves
to SF Pro, Segoe UI or Roboto, which are all grotesques. The exact face differs by platform; the
character does not.

The project site treats `h1` and `h2` as grotesque and leaves the rest monospace. The documentation
follows it, which puts the glossary's term headings in monospace — the right result, because each
term is a literal a reader will meet in a file or a command.

### How the file is arranged

A palette sets seven custom properties. One mapping block translates those into the `--sl-*`
properties Starlight reads, written once for both colour schemes. To add a palette, copy a block;
nothing else needs editing.

```
--dex-bg  --dex-surface  --dex-text  --dex-text-dim  --dex-heading  --dex-link  --dex-line
```

Starlight writes `data-theme="light"` or `"dark"` on the root element from its own toggle, so every
palette states both halves. The dark half is the bare `:root` block, matching Starlight's own
dark-first arrangement.

### The colour-scheme control

The header button steps through three colour schemes in one cycle, replacing Starlight's light and
dark select. The palettes differ in more than lightness, so a pair does not describe them.

| Step | Glyph | Palette | Colour scheme | Body contrast |
|---|---|---|---|---|
| Blue | ☁︎ | `brand` | always dark, whatever the reader's system asks for | 5.71:1 — AA |
| Dark | ☾ | `brand-deep` | dark | 9.60:1 — AAA |
| Light | ☀︎ | `brand-deep` | light | 11.41:1 — AAA |

Blue is the project site's colours and is what a reader sees first. From there the cycle goes to
the scheme the reader's system asks for, then to the other one, then back to Blue. So Dark and
Light appear in the order the system prefers, and the second click covers a reader whose system
setting does not suit the room.

The state is held in `localStorage` under `dex-view` and survives a reload.

The button shows the glyph alone. Each step's name reaches a screen reader through the button's
accessible name and a pointer through the tooltip, both reading *Colour scheme: Blue. Change it.*
The glyphs are the text presentations of cloud, last-quarter moon and sun, so each takes the
button's colour instead of drawing itself in an emoji font's own palette. `U+263E` has no emoji
presentation to suppress; the other two carry `U+FE0E`, the text variation selector. The script
also writes the step onto `data-scheme`, which is what the stylesheet uses to put the cloud on the
same optical centre as the other two.

[`src/components/ThemeProvider.astro`](src/components/ThemeProvider.astro) holds the state and
applies it, inlined in the head so no step flashes before the stored one.
[`src/components/ThemeSelect.astro`](src/components/ThemeSelect.astro) is the button. Both are
registered under `components` in `astro.config.mjs`. Starlight renders the control in the header
and in the mobile menu; the script wires every copy and updates every copy on a click.

To change which palettes the cycle uses, edit `resolve()` in `ThemeProvider.astro`. To change the
names or the glyphs, edit `describe()` in the same file. To change the number of steps, edit
`STEPS` there too.

### Palette previews

All five palettes can be previewed with a query parameter, including the two the cycle does not
use. The parameter sets the colours; the cycle still decides the colour scheme. Contrast is against
the background, WCAG 2.1, for body text in each colour scheme.

| URL | palette | contrast |
|---|---|---|
| *(none)* | the recorded pair, one flat field of colour | 5.71:1 dark, 5.71:1 light — AA |
| `?palette=brand-deep` | the same hues over a darker ground | 9.60:1 dark, 11.41:1 light — AAA |
| `?palette=ink` | near-black and near-white, brand on links and headings | 11.79:1 dark, 15.11:1 light — AAA |
| `?palette=amber` | amber on black, the first palette the project site shipped | 9.27:1 dark, 16.11:1 light — AAA |
| `?palette=brand-day` | the recorded pair by day, deeper night | 12.39:1 dark, 5.71:1 light |

`?palette=brand` is a no-op rather than an error: the recorded pair is what the first step uses.

The parameter is `palette` and not `theme` because Starlight already uses `data-theme` for the
colour scheme. `ThemeProvider.astro` copies the parameter onto `<html data-palette>` before first
paint, so the palette never flashes. It accepts `/^[a-z-]{1,20}$/` and ignores anything else.

The dimmed colour has the least room in the default palette: it has to stay under the body text's
5.71:1 to read as secondary and over 4.5:1 to pass AA at the size the table of contents runs.
`#8bc0d1` measures 4.89:1. Check that band before changing it.

To drop the previews once a palette is settled, delete those blocks from `src/styles/dex.css` and
the script from `astro.config.mjs`.

### Why not a community theme

A Starlight theme is a plugin that supplies CSS and component overrides. Most published ones are
palette swaps — Catppuccin, Nord, Gruvbox, Rosé Pine, Flexoki — and dex supplies its own palette,
so they have nothing to add. `starlight-theme-mdbook` and `starlight-theme-black` change layout as
well, and bring colour schemes and a scheme picker of their own to remove again.
`@shuttering/starlight` maps a token contract onto the same `--sl-*` properties this file does,
which is the closest fit, and it carries an organisation's design system that dex is not part of.

The arrangement here is one stylesheet with no dependency, matching how the project site overrides
its own base stylesheet.

## Local development

```sh
pnpm install
pnpm dev      # http://localhost:4321/docs/
pnpm build    # → dist/
pnpm preview
pnpm check    # type-check astro + the content schema
pnpm sync     # copy docs/ into the content collection on its own
```

`base` is `/docs`, so the paths above include it.
