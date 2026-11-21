// Packs the page of an app's web variant into one file, and checks that the file loads nothing.
//
// In:  the script and the style a `vite build` made (`readDist`), and the parts the page carries as data (a worker, a
//      WASM module, sources; any number, also none), each as bytes.
// Out: one HTML file. Everything is in it: the script and the style inline, each part gzip-compressed as base64 in a
//      `<script id="<part>" type="application/octet-stream">`, and a Content-Security-Policy as a meta tag that forbids
//      every request. The page is opened from disk (file://) and needs no server.
//
// The check (`verifyShell`) refuses a file whose shell loads anything: an element with a src or a href, a script that
// is not inline, a URL, a refresh, a policy that allows more than the page's own inline code and data (`policyProblems`:
// `default-src`, `connect-src`, `base-uri` and `form-action` 'none' alone, no address, no wildcard, no eval), or a part
// that was named but is not there. `assemble` refuses such a policy before it writes the file.
// What the compiled script does at run time is the browser test's to check (`appkit-web/e2e`): it opens the file and
// counts the requests.
//
// Plain JavaScript on purpose: Node runs it from the app's node_modules, where it strips no types.

import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { gzipSync } from 'node:zlib';

// Both policies forbid every request (`connect-src 'none'`), every outside source, forms and a base; they differ in
// what a script may start.

/**
 * The policy of a page that runs an engine: scripts inline, a WASM module compiled from embedded bytes, a worker started
 * from a blob. (The policy tasks-web has shipped with.)
 */
export const CSP_WASM_WORKER = [
  "default-src 'none'",
  "script-src 'unsafe-inline' 'wasm-unsafe-eval'",
  "style-src 'unsafe-inline'",
  'img-src data: blob:',
  'worker-src blob:',
  "connect-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
].join('; ');

/**
 * The policy of a page without WASM and without a worker: no `'wasm-unsafe-eval'`, and no `worker-src`, which then falls
 * back to `script-src` (no `blob:` there): a worker cannot be started at all.
 */
export const CSP_STRICT = [
  "default-src 'none'",
  "script-src 'unsafe-inline'",
  "style-src 'unsafe-inline'",
  'img-src data: blob:',
  "connect-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
].join('; ');

// The policy is the app's to give, so it is checked as a whole, not only for `connect-src 'none'`: a source listed next
// to 'none' makes the browser ignore the 'none', and a wildcard, a host or a scheme of the network lets the page load
// or reach what it names. What a policy may hold:

/** The directives that must be there, each with 'none' alone. */
const POLICY_NONE = ['default-src', 'connect-src', 'base-uri', 'form-action'];

/** The directives a policy may hold besides those; any other (`report-uri`, `report-to`, ...) is refused. */
const POLICY_DIRECTIVES = new Set([
  ...POLICY_NONE,
  'script-src',
  'script-src-elem',
  'script-src-attr',
  'style-src',
  'style-src-elem',
  'style-src-attr',
  'img-src',
  'font-src',
  'media-src',
  'worker-src',
  'child-src',
  'frame-src',
  'object-src',
  'manifest-src',
]);

/**
 * The two directives of Trusted Types. They only narrow what a page may do (every sink that takes a script or markup
 * as a string is refused unless a named policy made it), so they are allowed beside the others, with a fixed shape:
 * `require-trusted-types-for 'script'` and `trusted-types` naming policies, never `*` (any policy) and never
 * `'allow-duplicates'`.
 */
const POLICY_TRUSTED_TYPES = ['require-trusted-types-for', 'trusted-types'];

/** A policy name as the Trusted Types spec spells it (letters, digits and `-#=_/@.%`). */
const POLICY_NAME = /^[A-Za-z0-9\-#=_/@.%]+$/;

/** The sources a directive may name: what the page holds itself, never an address, a wildcard or eval. */
const POLICY_SOURCES = new Set(["'unsafe-inline'", "'wasm-unsafe-eval'", 'data:', 'blob:']);

/** What makes `csp` a policy that is not one of a page that loads nothing and reaches no network; empty when fine. */
export function policyProblems(csp) {
  if (typeof csp !== 'string' || csp.trim() === '') return ['the policy is empty'];
  const problems = [];
  const seen = new Map();
  for (const part of csp.split(';')) {
    const [name, ...sources] = part.trim().split(/\s+/).filter(Boolean);
    if (name === undefined) continue;
    const directive = name.toLowerCase();
    if (seen.has(directive)) {
      problems.push(`the directive ${directive} is given twice`);
      continue;
    }
    seen.set(directive, sources);
    if (directive === 'require-trusted-types-for') {
      if (sources.length !== 1 || sources[0] !== "'script'") problems.push(`${directive} is not 'script' alone: ${sources.join(' ')}`);
      continue;
    }
    if (directive === 'trusted-types') {
      if (sources.length === 0) problems.push(`${directive} names no policy (write 'none' for no policy at all)`);
      if (sources.includes("'none'") && sources.length > 1) problems.push(`${directive} names 'none' next to policies`);
      const odd = sources.filter((x) => x !== "'none'" && !POLICY_NAME.test(x));
      if (odd.length) problems.push(`${directive} names something that is not a policy name (no '*', no 'allow-duplicates'): ${odd.join(' ')}`);
      continue;
    }
    if (!POLICY_DIRECTIVES.has(directive)) {
      problems.push(`the directive ${directive} is not allowed`);
      continue;
    }
    if (POLICY_NONE.includes(directive)) {
      if (sources.length !== 1 || sources[0] !== "'none'") problems.push(`${directive} is not 'none' alone: ${sources.join(' ')}`);
      continue;
    }
    if (sources.includes("'none'") && sources.length > 1) problems.push(`${directive} names 'none' next to other sources`);
    const foreign = sources.filter((s) => s !== "'none'" && !POLICY_SOURCES.has(s.toLowerCase()));
    if (foreign.length) problems.push(`${directive} names a source that is not the page's own: ${foreign.join(' ')}`);
    if (directive.startsWith('script-src') && sources.some((s) => s.toLowerCase() === 'data:')) {
      problems.push(`${directive} allows a script from data:`);
    }
  }
  for (const directive of POLICY_NONE) {
    if (!seen.has(directive)) problems.push(`the directive ${directive} is missing`);
  }
  return problems;
}

/** A part's id: it becomes an attribute value and an element id, so it is a plain lowercase word. */
const PART_ID = /^[a-z][a-z0-9-]*$/;

/** The ids checked: plain words, no one twice, not `app` (the element the page mounts on). */
export function checkPartIds(partIds) {
  if (!Array.isArray(partIds)) throw new Error('pack: the part ids are not a list');
  const seen = new Set();
  for (const id of partIds) {
    if (typeof id !== 'string' || !PART_ID.test(id)) throw new Error(`pack: the part id ${JSON.stringify(id)} is not a plain word`);
    if (id === 'app') throw new Error('pack: the part id "app" is the element the page mounts on');
    if (seen.has(id)) throw new Error(`pack: the part id "${id}" is named twice`);
    seen.add(id);
  }
  return partIds;
}

/** Bytes as the text of a part: gzip at the highest level, base64, lines of 120. */
export const packPart = (data) => gzipSync(data, { level: 9 }).toString('base64').replace(/.{1,120}/g, '$&\n');

/** Text as the content of an inline script: `</script` and `<!--` would end it or start a comment. */
export function inlineScript(js) {
  // The case of the word is kept: a string "</SCRIPT>" must still be that string once the browser has read it.
  return js.replace(/<\/(script)/gi, (_, word) => `<\\/${word}`).replace(/<!--/g, '<\\!--');
}

/** Text as the content of an inline style; a `</style` cannot be written there, so it is refused. */
export function inlineStyle(css) {
  if (/<\/style/i.test(css)) throw new Error('pack: the style holds "</style"');
  return css;
}

/** Text that goes into the shell as it is (the title, the comment): it may not open or close markup. */
function plainText(text, what) {
  if (typeof text !== 'string' || /[<>&]|--/.test(text)) throw new Error(`pack: the ${what} is not plain text: ${JSON.stringify(text)}`);
  return text;
}

const isBytes = (value) => value instanceof Uint8Array || Buffer.isBuffer(value);

/**
 * The single file. `partIds` names the parts in the order they are written, `parts` maps each id to its bytes; `csp` is
 * the policy of the page (`CSP_WASM_WORKER`, `CSP_STRICT` or the app's own), `title` the title of the page and
 * `comment` the comment in the head (where the file comes from, its build).
 */
export function assemble({ css, js, partIds = [], parts = {}, csp, title, comment }) {
  checkPartIds(partIds);
  if (typeof csp !== 'string' || csp === '' || /"/.test(csp)) throw new Error('pack: the policy is missing or holds a quote');
  const loose = policyProblems(csp);
  if (loose.length) throw new Error('pack: the policy lets the page load or reach something: ' + loose.join('; '));
  const missing = partIds.filter((id) => !isBytes(parts[id]));
  if (missing.length) throw new Error('pack: parts missing: ' + missing.join(', '));
  const extra = Object.keys(parts).filter((id) => !partIds.includes(id));
  if (extra.length) throw new Error('pack: parts not named: ' + extra.join(', '));
  const blocks = partIds.map((id) => `<script id="${id}" type="application/octet-stream">${packPart(parts[id])}</script>`);
  return [
    '<!doctype html>',
    '<html lang="en">',
    '<head>',
    '<meta charset="utf-8">',
    `<meta http-equiv="Content-Security-Policy" content="${csp}">`,
    '<meta name="viewport" content="width=device-width, initial-scale=1">',
    '<meta name="referrer" content="no-referrer">',
    '<meta name="color-scheme" content="light dark">',
    `<title>${plainText(title, 'title')}</title>`,
    `<!-- ${plainText(comment, 'comment')} -->`,
    `<style>${inlineStyle(css)}</style>`,
    '</head>',
    '<body>',
    '<div id="app"></div>',
    ...blocks,
    `<script type="module">${inlineScript(js)}</script>`,
    '</body>',
    '</html>',
    '',
  ].join('\n');
}

// Every attribute that makes the browser fetch something, on any element (<a href>, <object data>, <base href>,
// <form action>, <video poster>, <img srcset>, ...): the page loads nothing, so none may be there.
const LOADING_ATTRIBUTE = /<[a-z][a-z0-9-]*\b[^>]*\s(src|href|data|action|formaction|poster|srcset|ping|background)\s*=/i;

/**
 * What keeps a page from being one that loads nothing and reaches no network; empty when it is fine. `allowUrls` are
 * the addresses the page may name as text (none by default).
 */
export function verify(html, { allowUrls = [] } = {}) {
  const problems = [];
  // Every policy meta is enforced, so each one is checked; without one the page may load anything.
  const policies = [...html.matchAll(/<meta\b[^>]*\bhttp-equiv=["']?Content-Security-Policy\b[^>]*>/gi)].map((m) => {
    const content = m[0].match(/\scontent="([^"]*)"/i) ?? m[0].match(/\scontent='([^']*)'/i);
    return content ? content[1] : '';
  });
  if (policies.length === 0) problems.push("the CSP meta with connect-src 'none' is missing");
  for (const csp of policies) {
    for (const problem of policyProblems(csp)) problems.push('the CSP meta is too loose: ' + problem);
  }
  const loading = html.match(LOADING_ATTRIBUTE);
  if (loading) problems.push(`an element loads a file (${loading[1]}=)`);
  if (/<meta[^>]*http-equiv=["']?refresh/i.test(html)) problems.push('a meta refresh');
  const urls = html.match(/\b(?:https?|wss?|ftp):\/\/[^\s"'<>]+/g) ?? [];
  const foreign = urls.filter((u) => !allowUrls.includes(u));
  if (foreign.length > 0) problems.push('a URL in the page: ' + foreign.join(', '));
  const placeholder = html.match(/__[A-Z][A-Z_]*__/);
  if (placeholder) problems.push('a placeholder is left: ' + placeholder[0]);
  return problems;
}

/**
 * What the page without the contents of its scripts and styles may not hold; empty when it is fine. `partIds` are the
 * parts the page must carry (the same list `assemble` was given); a script that is neither one of them nor the one
 * module is refused.
 */
export function verifyShell(html, { partIds = [] } = {}) {
  checkPartIds(partIds);
  const shell = html
    .replace(/(<script\b[^>]*>)[\s\S]*?(<\/script>)/gi, '$1$2')
    .replace(/(<style\b[^>]*>)[\s\S]*?(<\/style>)/gi, '$1$2');
  const problems = verify(shell, { allowUrls: [] });
  // Each script tag is compared as a whole with the tags `assemble` writes: a pattern that looks for an attribute in
  // the tag would also find it inside another one (`data-type="module"`).
  const partTag = (id) => `<script id="${id}" type="application/octet-stream">`;
  const tags = shell.match(/<script\b[^>]*>/gi) ?? [];
  for (const id of partIds) {
    const count = tags.filter((tag) => tag === partTag(id)).length;
    if (count === 0) problems.push(`the part "${id}" is missing`);
    if (count > 1) problems.push(`the part "${id}" is there ${count} times`);
  }
  const modules = tags.filter((tag) => tag === '<script type="module">').length;
  if (modules !== 1) problems.push(`expected one inline module script, found ${modules}`);
  const known = new Set(['<script type="module">', ...partIds.map(partTag)]);
  const plain = tags.find((tag) => !known.has(tag));
  if (plain) problems.push('a script that is neither a part nor the module: ' + plain);
  return problems;
}

/** The script and the style `vite build` made, found through its index (`index`, in `dist`). */
export function readDist(dist, index = 'index.html') {
  const page = readFileSync(join(dist, index), 'utf8');
  const scripts = [...page.matchAll(/<script\b[^>]*\btype="module"[^>]*\bsrc="([^"]+)"/g)].map((m) => m[1]);
  const styles = [...page.matchAll(/<link\b[^>]*\brel="stylesheet"[^>]*\bhref="([^"]+)"/g)].map((m) => m[1]);
  if (scripts.length !== 1) throw new Error(`pack: expected one script in ${dist}/${index}, found ${scripts.length} (is code splitting off?)`);
  if (styles.length !== 1) throw new Error(`pack: expected one style in ${dist}/${index}, found ${styles.length}`);
  return {
    js: readFileSync(resolve(dist, scripts[0]), 'utf8'),
    css: readFileSync(resolve(dist, styles[0]), 'utf8'),
  };
}

/** Write the page as `<out>/<name>` and its sum as `<out>/SHA256SUMS` (the format of `sha256sum`); the sum is returned. */
export function writePage(out, name, html) {
  if (!/^[\w.-]+\.html$/.test(name)) throw new Error(`pack: ${JSON.stringify(name)} is no plain file name of a page`);
  mkdirSync(out, { recursive: true });
  writeFileSync(join(out, name), html);
  const sha = createHash('sha256').update(html).digest('hex');
  writeFileSync(join(out, 'SHA256SUMS'), `${sha}  ${name}\n`);
  return sha;
}
