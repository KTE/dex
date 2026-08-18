import { defineCollection } from 'astro:content'
import { docsLoader } from '@astrojs/starlight/loaders'
import { docsSchema } from '@astrojs/starlight/schema'

// src/content/docs/ is written by scripts/sync-docs.mjs and is git-ignored.
// The documentation itself lives in the repository root docs/ directory.
export const collections = {
  docs: defineCollection({ loader: docsLoader(), schema: docsSchema() }),
}
