// @ts-check
import { defineConfig } from 'astro/config'
import starlight from '@astrojs/starlight'

// The documentation is served from the same origin as the project site, under
// /docs, so the site keeps one deployment and one DNS record.
export default defineConfig({
  site: 'https://dex.ars.is',
  base: '/docs',
  trailingSlash: 'always',
  integrations: [
    starlight({
      title: 'dex',
      description: 'A Raspberry Pi video player that loops an artwork seamlessly.',
      logo: { src: './public/icon.svg', replacesTitle: false },
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/KTE/dex' }],
      // Loaded after Starlight's own styles, so these files override its
      // custom properties.
      customCss: ['./src/styles/dex.css'],
      // The colour-scheme control steps through three palettes instead of
      // Starlight's light and dark pair. ThemeProvider holds the state and
      // sets it before first paint; ThemeSelect is the button.
      components: {
        ThemeProvider: './src/components/ThemeProvider.astro',
        ThemeSelect: './src/components/ThemeSelect.astro',
      },
      sidebar: [
        { label: 'Guides', items: [{ autogenerate: { directory: 'guides' } }] },
        { label: 'Design', items: [{ autogenerate: { directory: 'design' } }] },
        { label: 'Reference', items: [{ label: 'Glossary', slug: 'glossary' }] },
      ],
      pagination: false,
      credits: false,
    }),
  ],
})
