import { defineConfig } from 'astro/config'

export default defineConfig({
  site: 'https://dex.ars.is',
  build: {
    // Inline the stylesheet instead of emitting an _astro/ asset. It is a few
    // kB on a one-page site, so a separate request buys nothing - and with no
    // build-time assets the page has no absolute /_astro/ URLs, so it renders
    // correctly at dex.ars.is and at the kte.github.io/dex fallback alike.
    inlineStylesheets: 'always',
  },
})
