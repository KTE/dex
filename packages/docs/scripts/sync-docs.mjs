// @ts-check
//
// Copy the markdown from the repository root docs/ directory into the Starlight
// content collection, adding the frontmatter Starlight requires.
//
// The documentation is written as plain markdown that reads correctly on
// GitHub: it opens with a `# Heading` and carries no frontmatter. Starlight
// needs a `title` in frontmatter, and it validates that when the collection
// loads, which is before any remark plugin could supply one. So the title is
// lifted out of the first heading here and the heading is dropped, because
// Starlight renders the title itself and a second copy would show twice.
//
// The output directory is generated and git-ignored. Edit docs/, not the copy.
//
// Files in src/site/ are copied in as well and already carry frontmatter. That
// is where the site's own pages live, such as the landing page.

import { cp, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = resolve(packageRoot, '..', '..')

const sourceDir = join(repoRoot, 'docs')
const overlayDir = join(packageRoot, 'src', 'site')
const outputDir = join(packageRoot, 'src', 'content', 'docs')

/**
 * Every markdown file below `dir`, as paths relative to `dir`.
 * @param {string} dir
 * @returns {Promise<string[]>}
 */
async function markdownFiles(dir) {
  const entries = await readdir(dir, { recursive: true, withFileTypes: true })
  return entries
    .filter((entry) => entry.isFile() && entry.name.endsWith('.md'))
    .map((entry) => relative(dir, join(entry.parentPath, entry.name)))
}

/**
 * Split a document into its first heading and the text that follows.
 * @param {string} markdown
 * @returns {{ title: string, body: string }}
 */
function splitTitle(markdown) {
  const match = markdown.match(/^#[^\S\n]+(.+?)[^\S\n]*\n/)
  if (!match) throw new Error('no level-one heading on the first line')
  return { title: match[1].trim(), body: markdown.slice(match[0].length).replace(/^\n+/, '') }
}

/**
 * A YAML double-quoted scalar. Titles are prose, so only the quote and the
 * backslash need escaping.
 * @param {string} value
 * @returns {string}
 */
function yamlString(value) {
  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`
}

await rm(outputDir, { recursive: true, force: true })
await mkdir(outputDir, { recursive: true })

// The site's own pages first, so a name clash reports against the docs file.
await cp(overlayDir, outputDir, { recursive: true })

for (const file of await markdownFiles(sourceDir)) {
  const source = await readFile(join(sourceDir, file), 'utf8')
  if (source.startsWith('---')) {
    throw new Error(`${file}: already has frontmatter; this script would add a second block`)
  }

  let split
  try {
    split = splitTitle(source)
  } catch (error) {
    throw new Error(`${file}: ${error instanceof Error ? error.message : error}`)
  }

  const target = join(outputDir, file)
  await mkdir(dirname(target), { recursive: true })
  await writeFile(target, `---\ntitle: ${yamlString(split.title)}\n---\n\n${split.body}`)
}

const written = await markdownFiles(outputDir)
console.log(`sync-docs: ${written.length} pages in src/content/docs/`)
