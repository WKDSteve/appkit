// The reader's check of a code-ref/1 (schemas/code-ref/1/README.md): a JavaScript port of tools/check-code-ref.py.
//
// A code-ref is untrusted input, wherever it came from (a task file, a clipboard, a job). `problems` never throws on
// one: it takes the bytes (or the text) and names every reason to refuse them, and an empty list means the page may
// use the reference. `blocks` finds the fenced `code-ref` blocks of a task body, `taskProblems` checks each of them.
//
// Same rules, same order and the same words as the reference. Two things JSON.parse cannot tell: `1.0` is the number 1
// (the reference refuses it as no integer), and a number beyond 2**53 is a double (refused as out of range either
// way). The path rule is the one of docmap-desktop's open-at.js `relativePath`, in one place.

export const MAX_BYTES = 64 * 1024;
export const MAX_EXCERPT_BYTES = 4096;
export const SCHEMA = 'code-ref/1';
export const FENCE_LANG = 'code-ref';
export const MAX_PATH = 300;
export const MAX_LINE = 10_000_000;

const CLASSES = ['public', 'internal'];
const LIMITS = { symbol: 200, 'docmap.node': 300, 'docmap.fn': 200, 'telemetry.namespace': 100, 'telemetry.key': 300 };

const REPO = /^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/;
const SHA = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/;
const LANG = /^[a-z0-9][a-z0-9+#._-]{0,31}$/;
// Control characters other than tab, line break and carriage return, and the C1 block.
const CONTROL = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/;
// One word that leaves the repository: an absolute path, a home, a `..` segment. `\s` is Unicode white space in both
// languages; Python's also takes \x1c-\x1f, so they are named here.
const SPACE = '\\s\\x1c-\\x1f';
const ESCAPE = new RegExp(
  `(?:^|[${SPACE}"'=])(?:[/\\\\]|[A-Za-z]:[/\\\\]|~[/\\\\]|\\$\\{?HOME(?![\\p{L}\\p{N}_])|%[A-Za-z]+%)` +
    `|(?:^|[${SPACE}"'=/\\\\])\\.\\.(?:[/\\\\]|[${SPACE}]|$)`,
  'u',
);

const encoder = new TextEncoder();
const isObject = (value) => typeof value === 'object' && value !== null && !Array.isArray(value);
const length = (text) => [...text].length;
const isPlain = (value, max) => typeof value === 'string' && value.length > 0 && length(value) <= max;

export function leavesRepo(text) {
  return ESCAPE.test(text);
}

/** Every string of the document, keys included. A loop, not recursion, so the depth cannot overflow the stack. */
function strings(value) {
  const found = [];
  const stack = [value];
  while (stack.length > 0) {
    const item = stack.pop();
    if (typeof item === 'string') found.push(item);
    else if (Array.isArray(item)) stack.push(...item);
    else if (isObject(item)) {
      for (const key of Object.keys(item)) {
        found.push(key);
        stack.push(item[key]);
      }
    }
  }
  return found;
}

function checkFile(file, out) {
  if (typeof file !== 'string' || file === '') out.push('file is missing');
  else if (length(file) > MAX_PATH) out.push(`file is longer than ${MAX_PATH}`);
  else if (leavesRepo(file)) out.push('file leaves the repository');
  else if (/[\\:\0]/.test(file)) out.push('file is not a plain forward-slash path');
  else if (file.split('/').some((part) => part === '' || part === '.')) out.push('file is not canonical (empty or . segment)');
}

/** `docmap` and `telemetry`: an object whose fields are plain non-empty strings; the first `required` are required. */
function checkPair(doc, key, fields, required, out) {
  const value = doc[key];
  if (value === undefined || value === null) return;
  if (!isObject(value)) {
    out.push(`${key} is not an object`);
    return;
  }
  fields.forEach((field, index) => {
    const text = value[field];
    const name = `${key}.${field}`;
    if ((text === undefined || text === null) && index >= required) return;
    if (!isPlain(text, LIMITS[name])) out.push(`${name} is missing or longer than ${LIMITS[name]}`);
  });
  if (key === 'docmap' && typeof value.node === 'string' && leavesRepo(value.node)) out.push('docmap.node leaves the repository');
}

/** Every reason to refuse `raw` (bytes or text), empty when the page may use the reference. */
export function problems(raw) {
  const bytes = typeof raw === 'string' ? encoder.encode(raw) : raw;
  if (bytes.length > MAX_BYTES) return [`larger than ${MAX_BYTES} bytes`];
  let doc;
  try {
    // ignoreBOM keeps a byte order mark in the text, where JSON.parse refuses it, as the reference does.
    doc = JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes));
  } catch {
    return ['not JSON in UTF-8'];
  }
  if (!isObject(doc)) return ['not a JSON object'];
  if (doc.schema !== SCHEMA) return [`schema ${JSON.stringify(doc.schema ?? null)} is not ${SCHEMA}`];
  const out = [];
  if (!(typeof doc.repo === 'string' && REPO.test(doc.repo))) out.push('repo is missing or not a plain name');
  checkFile(doc.file, out);
  const { lines } = doc;
  if (!(Array.isArray(lines) && lines.length === 2 && lines.every((n) => Number.isInteger(n)))) out.push('lines is not two integers');
  else if (!(lines[0] >= 1 && lines[0] <= lines[1] && lines[1] <= MAX_LINE)) out.push(`lines is not 1 <= first <= last <= ${MAX_LINE}`);
  if (doc.sha !== undefined && doc.sha !== null && !(typeof doc.sha === 'string' && SHA.test(doc.sha))) {
    out.push('sha is not 40 or 64 lowercase hex characters');
  }
  for (const key of ['dirty', 'truncated']) {
    if (key in doc && typeof doc[key] !== 'boolean') out.push(`${key} is not a boolean`);
  }
  if (doc.lang !== undefined && doc.lang !== null && !(typeof doc.lang === 'string' && LANG.test(doc.lang))) out.push('lang is not a plain name');
  if (doc.symbol !== undefined && doc.symbol !== null && !isPlain(doc.symbol, LIMITS.symbol)) {
    out.push(`symbol is empty or longer than ${LIMITS.symbol}`);
  }
  if (doc.excerpt !== undefined && doc.excerpt !== null) {
    if (typeof doc.excerpt !== 'string') out.push('excerpt is not a string');
    else if (encoder.encode(doc.excerpt).length > MAX_EXCERPT_BYTES) out.push(`excerpt is longer than ${MAX_EXCERPT_BYTES} bytes`);
  }
  if ('class' in doc && !CLASSES.includes(doc.class)) out.push('class is not public or internal');
  checkPair(doc, 'docmap', ['node', 'fn'], 1, out);
  checkPair(doc, 'telemetry', ['namespace', 'key'], 2, out);
  if (strings(doc).some((text) => CONTROL.test(text))) out.push('a field holds a control character');
  return out;
}

const OPEN = /^ {0,3}(`{3,})([^`]*)$/;

/**
 * The text of every closed fenced block of language `code-ref` in the body of a task.
 *
 * A fence opens on a line of three or more backticks and closes on a line of at least as many; a block of any other
 * language, and anything inside it, is skipped, so a `code-ref` shown inside a longer fence is not a reference. The
 * frontmatter is not body. An unclosed block is dropped.
 */
export function blocks(markdown) {
  const lines = markdown.replaceAll('\r\n', '\n').split('\n');
  let start = 0;
  if (lines.length > 0 && lines[0].trimEnd() === '---') {
    for (let i = 1; i < lines.length; i += 1) {
      if (lines[i].trimEnd() === '---') {
        start = i + 1;
        break;
      }
    }
  }
  const found = [];
  let fence = null;
  let mine = false;
  let body = [];
  for (const line of lines.slice(start)) {
    if (fence === null) {
      const match = OPEN.exec(line);
      if (match) [fence, mine, body] = [match[1].length, match[2].trim() === FENCE_LANG, []];
    } else if (new RegExp(`^ {0,3}\`{${fence},}\\s*$`).test(line)) {
      if (mine) found.push(body.join('\n'));
      fence = null;
    } else if (mine) body.push(line);
  }
  return found;
}

/** `[index, problems]` for every `code-ref` block of a task; empty problems means usable. */
export function taskProblems(markdown) {
  return blocks(markdown).map((text, index) => [index, problems(text)]);
}
