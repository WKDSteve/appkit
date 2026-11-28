// What an app's bundler keeps of the kit when the app imports from the root (`appkit-ui`): the markdown parser
// (markdown-it, about 100 kB) only when the app parses markdown. A build with Vite, as the apps have it, of an entry
// that exists only in memory (the kit has no Node types, so no file is written).
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { build, type Plugin, type Rollup } from 'vite';
import { describe, expect, it } from 'vitest';

/** The path of a file of the kit beside this test, as Vite takes it (`C:/…` on Windows, `/…` elsewhere). */
function kitFile(name: string): string {
  const path = decodeURIComponent(new URL(`./${name}`, import.meta.url).pathname);
  return /^\/[A-Za-z]:\//.test(path) ? path.slice(1) : path;
}
const ROOT = kitFile('index.ts');
// `appkit-ui/markdown`, the one way to the parser.
const MARKDOWN = kitFile('markdown.ts');

const ENTRY = 'virtual:root-import-entry';

/** The code of a production build of an entry with this source. */
async function bundle(source: string): Promise<string> {
  const entry: Plugin = {
    name: 'root-import-entry',
    resolveId: (id) => (id === ENTRY ? `\0${ENTRY}` : null),
    load: (id) => (id === `\0${ENTRY}` ? source : null),
  };
  const out = (await build({
    configFile: false,
    logLevel: 'silent',
    plugins: [entry, svelte()],
    resolve: { conditions: ['browser'] },
    build: { write: false, minify: false, rollupOptions: { input: ENTRY } },
  })) as Rollup.RollupOutput | Rollup.RollupOutput[];
  const outputs = Array.isArray(out) ? out : [out];
  return outputs
    .flatMap((o) => o.output)
    .map((chunk) => (chunk.type === 'chunk' ? chunk.code : ''))
    .join('\n');
}

// Words that only markdown-it's own rules carry (its block and inline rule names).
const parserIn = (code: string): boolean => code.includes('html_block') && code.includes('linkify');

describe('a root import of appkit-ui', () => {
  it('keeps markdown-it when the app parses markdown (the check can see the parser)', async () => {
    const code = await bundle(
      `import { signOut } from '${ROOT}';\nimport { parseMarkdown } from '${MARKDOWN}';\n` +
        `console.log(signOut, parseMarkdown('# a'));\n`,
    );
    expect(parserIn(code)).toBe(true);
  }, 120_000);

  it('leaves markdown-it out when the app takes only other parts', async () => {
    const code = await bundle(
      `import { safeHref, signOut } from '${ROOT}';\nconsole.log(safeHref('https://a.example'), signOut);\n`,
    );
    expect(parserIn(code)).toBe(false);
  }, 120_000);
});
