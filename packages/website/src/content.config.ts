import { defineCollection } from 'astro:content'
import { glob } from 'astro/loaders'
import { z } from 'astro/zod'

/**
 * The page is assembled from one markdown file per block.
 *
 * Order comes from the filename. Blocks are sorted by id, so the numeric
 * prefixes decide what appears where — renaming a file reorders the page, and
 * there is no index to keep in sync. Files starting with `_` are skipped, which
 * is how a block is parked without deleting it.
 */
const blocks = defineCollection({
  loader: glob({ base: './src/content/blocks', pattern: '**/[^_]*.md' }),
  schema: z.object({
    /** Which landmark the block renders into. */
    slot: z.enum(['header', 'main', 'footer']).default('main'),
    /** Wrap the block in its own `<section>`. Use for main blocks that open with a heading. */
    section: z.boolean().default(false),
    /** `<head>` metadata. Set these on one block only; the header block is the natural home. */
    title: z.string().optional(),
    description: z.string().optional(),
  }),
})

export const collections = { blocks }
