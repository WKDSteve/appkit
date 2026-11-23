// The tests of tools/test_check_code_ref.py, ported, against the goldens of schemas/code-ref/1/examples.
//
//   node --test test/code-ref.test.mjs        (from web-kit/)

import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { MAX_BYTES, MAX_EXCERPT_BYTES, blocks, leavesRepo, problems, taskProblems } from '../src/code-ref.js';

const EXAMPLES = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'schemas', 'code-ref', '1', 'examples');
const golden = (name) => JSON.parse(readFileSync(join(EXAMPLES, `${name}.json`), 'utf8'));
const mutated = (name, change) => {
  const doc = golden(name);
  change(doc);
  return doc;
};
const check = (doc) => problems(JSON.stringify(doc));
const refused = (doc, needle) => {
  const found = check(doc);
  assert.ok(found.some((line) => line.includes(needle)), `${needle} not in ${JSON.stringify(found)} for ${JSON.stringify(doc)}`);
};
const accepted = (doc) => assert.deepEqual(check(doc), [], JSON.stringify(doc));

test('each golden passes', () => {
  assert.deepEqual(readdirSync(EXAMPLES).sort(), ['function.json', 'marking.json', 'minimal.json', 'task-body.md']);
  for (const name of ['marking', 'function', 'minimal']) {
    assert.deepEqual(problems(new Uint8Array(readFileSync(join(EXAMPLES, `${name}.json`)))), [], name);
  }
});

test('the golden task body yields its two blocks and both pass', () => {
  const found = taskProblems(readFileSync(join(EXAMPLES, 'task-body.md'), 'utf8'));
  assert.deepEqual(found, [[0, []], [1, []]]);
});

test('size is measured before parsing, in bytes', () => {
  assert.deepEqual(problems(' '.repeat(MAX_BYTES + 1)), [`larger than ${MAX_BYTES} bytes`]);
  assert.deepEqual(problems(new Uint8Array(MAX_BYTES + 1)), [`larger than ${MAX_BYTES} bytes`]);
  assert.deepEqual(problems(' '.repeat(MAX_BYTES)), ['not JSON in UTF-8']);
  // 21846 euro signs are 65538 bytes: short by characters, too long by bytes.
  assert.deepEqual(problems('\u20ac'.repeat(21_846)), [`larger than ${MAX_BYTES} bytes`]);
});

test('not a document', () => {
  for (const raw of ['', '[1]', 'null', '['.repeat(100_000), new Uint8Array([0xff, 0xfe]), '\ufeff{}']) {
    assert.ok(problems(raw).length > 0, String(raw).slice(0, 8));
  }
  assert.deepEqual(problems('[1]'), ['not a JSON object']);
});

test('a higher or foreign version', () => {
  for (const schema of ['code-ref/2', 'code-ref/0', 'wkd-intent/1', null, ['code-ref/1']]) {
    refused(mutated('function', (d) => { d.schema = schema; }), 'is not code-ref/1');
  }
  refused(mutated('function', (d) => { delete d.schema; }), 'is not code-ref/1');
});

test('required fields', () => {
  for (const key of ['repo', 'file', 'lines']) refused(mutated('minimal', (d) => { delete d[key]; }), key);
});

test('repo is a name, not a path', () => {
  for (const text of ['E:\\repos\\x', '/home/x', 'a/b', '..', '', '.x', 'x'.repeat(101), 1]) {
    refused(mutated('minimal', (d) => { d.repo = text; }), 'repo');
  }
  for (const text of ['docmap.nvim', 'a', 'A_b-c.d', 'x'.repeat(100)]) accepted(mutated('minimal', (d) => { d.repo = text; }));
});

test('no way out of the repository', () => {
  const escapes = ['/etc/passwd', '\\windows', 'C:\\x', 'c:/x', '\\\\host\\x', '~/x', '$HOME/x', '${HOME}/x', '%USERPROFILE%\\x', '../x', 'a/../../x', 'a\\..\\x', '..'];
  for (const text of escapes) {
    refused(mutated('minimal', (d) => { d.file = text; }), 'file leaves');
    refused(mutated('function', (d) => { d.docmap.node = text; }), 'docmap.node leaves');
  }
});

test('the escape words in the middle of a text', () => {
  for (const text of ['a /etc', 'x=/etc', "'~/x", '"C:\\x', 'a ../x', 'a/..', 'a/../b', 'x\u00a0%HOME%', 'v $HOME']) assert.ok(leavesRepo(text), text);
  const fine = ['a..b', 'TESTS/x..y/z.lua', 'a~/b', 'a/b/c', 'a$HOMEX', '${HOMEX}', '100%', '%%', 'ui-kit/src/a b.css', '..a', 'a/..b'];
  for (const text of fine) assert.ok(!leavesRepo(text), text);
});

test('a file is one canonical form', () => {
  for (const text of ['a\\b.lua', 'a/b.lua:5', './a.lua', 'a//b.lua', 'a/./b.lua', 'a/', 'x\0y']) {
    refused(mutated('minimal', (d) => { d.file = text; }), 'file is not');
  }
  for (const text of ['a/b.lua', 'TESTS/x..y/z.lua', 'a..b', 'ui-kit/src/a b.css', '.github/workflows/ci.yml']) {
    accepted(mutated('minimal', (d) => { d.file = text; }));
  }
  refused(mutated('minimal', (d) => { d.file = 'a/'.repeat(151); }), 'file is longer');
  refused(mutated('minimal', (d) => { d.file = ''; }), 'file is missing');
});

test('lines', () => {
  const bad = [[5, 4], [0, 3], [-1, 2], [1], [1, 2, 3], ['1', 2], [1.5, 2], [true, true], '1-2', null, [1, 10_000_001]];
  for (const lines of bad) refused(mutated('minimal', (d) => { d.lines = lines; }), 'lines');
  for (const lines of [[10_000_000, 10_000_000], [1, 1], [3, 9]]) accepted(mutated('minimal', (d) => { d.lines = lines; }));
});

test('sha', () => {
  for (const text of ['abc', '0123456789ABCDEF0123456789ABCDEF01234567', 'g'.repeat(40), 1]) {
    refused(mutated('function', (d) => { d.sha = text; }), 'sha');
  }
  for (const ok of [null, 'a'.repeat(40), 'b'.repeat(64)]) accepted(mutated('function', (d) => { d.sha = ok; }));
});

test('only released classes', () => {
  for (const cls of ['private', 'unclassified', null, 1]) refused(mutated('function', (d) => { d.class = cls; }), 'class');
  assert.ok(!('class' in golden('minimal')));
  for (const cls of ['public', 'internal']) accepted(mutated('function', (d) => { d.class = cls; }));
});

test('the excerpt is counted in bytes', () => {
  accepted(mutated('function', (d) => { d.excerpt = 'x'.repeat(MAX_EXCERPT_BYTES); }));
  refused(mutated('function', (d) => { d.excerpt = 'x'.repeat(MAX_EXCERPT_BYTES + 1); }), 'excerpt is longer');
  // 1366 characters, 4098 bytes: short by characters, too long by bytes.
  refused(mutated('function', (d) => { d.excerpt = '\u20ac'.repeat(1366); }), 'excerpt is longer');
  accepted(mutated('function', (d) => { d.excerpt = '\u20ac'.repeat(1365); }));
  refused(mutated('function', (d) => { d.excerpt = 5; }), 'excerpt is not a string');
});

test('the excerpt is code, not a path', () => {
  accepted(mutated('function', (d) => { d.excerpt = 'local p = "/etc/passwd" .. "../x" -- ~/y'; }));
});

test('lengths and types of the small fields', () => {
  const cases = [['symbol', null, 200], ['docmap', 'node', 300], ['docmap', 'fn', 200], ['telemetry', 'namespace', 100], ['telemetry', 'key', 300]];
  for (const [key, sub, max] of cases) {
    const name = sub === null ? key : `${key}.${sub}`;
    const set = (d, len) => { if (sub === null) d[key] = 'x'.repeat(len); else d[key][sub] = 'x'.repeat(len); };
    refused(mutated('function', (d) => set(d, max + 1)), name);
    accepted(mutated('function', (d) => set(d, max)));
  }
  refused(mutated('function', (d) => { d.docmap = { fn: 'node' }; }), 'docmap.node');
  accepted(mutated('function', (d) => { d.docmap = { node: 'a.b' }; }));
  refused(mutated('function', (d) => { d.telemetry = { namespace: 'x' }; }), 'telemetry.key');
  refused(mutated('function', (d) => { d.telemetry = 'x'; }), 'telemetry is not an object');
  refused(mutated('function', (d) => { d.dirty = 'no'; }), 'dirty');
  refused(mutated('function', (d) => { d.truncated = 1; }), 'truncated');
  refused(mutated('function', (d) => { d.lang = 'Lua!'; }), 'lang');
  // 200 characters are not too long for a symbol even when they are 600 bytes.
  accepted(mutated('function', (d) => { d.symbol = '\u20ac'.repeat(200); }));
});

test('control characters', () => {
  for (const key of ['symbol', 'excerpt', 'repo']) refused(mutated('function', (d) => { d[key] = 'evil\x1b[31m'; }), 'control character');
  const doc = mutated('function', (d) => { d.excerpt = 'line one\r\nline two\ttab'; });
  accepted(doc);
  refused({ ...doc, later: 'a\0b' }, 'control character');
  refused(mutated('function', (d) => { d.later = 'a\u0085b'; }), 'control character');
  refused(mutated('function', (d) => { d['a\x07b'] = 1; }), 'control character');
});

test('deep nesting never throws', () => {
  for (const depth of [500, 900, 990, 1100, 20_000]) {
    const raw = `{"schema":"code-ref/1","later":${'['.repeat(depth)}${']'.repeat(depth)}}`;
    assert.ok(Array.isArray(problems(raw)), String(depth));
  }
  assert.ok(problems(`{"schema":"code-ref/1","later":${'['.repeat(5000)}"x"${']'.repeat(5000)}}`).some((l) => l.includes('repo')));
});

test('unknown fields are ignored', () => {
  accepted(mutated('function', (d) => { d.later = { x: 1 }; d.docmap.later = 1; }));
});

test('only the language code-ref counts', () => {
  assert.deepEqual(blocks('```json\n{"schema": "code-ref/1"}\n```\n```code-ref-x\n{}\n```\n'), []);
});

test('a reference inside a longer fence is shown, not read', () => {
  assert.deepEqual(blocks('````md\n```code-ref\n{}\n```\n````\n'), []);
  assert.deepEqual(blocks('```\n```code-ref\n{}\n```\n'), []);
});

test('frontmatter is not body and an unclosed block is dropped', () => {
  assert.deepEqual(blocks('---\nnote: |\n  ```code-ref\n---\nbody\n```code-ref\n{}\n'), []);
  assert.deepEqual(blocks('---\nn: |\n```code-ref\nx\n```\n---\n```code-ref\n{}\n```\n'), ['{}']);
});

test('a bad block is reported with its number', () => {
  const text = `\`\`\`code-ref\n${JSON.stringify(golden('minimal'))}\n\`\`\`\n\`\`\`code-ref\n{"schema": "code-ref/2"}\n\`\`\`\n`;
  const found = taskProblems(text);
  assert.deepEqual(found[0], [0, []]);
  assert.ok(found[1][1].length > 0);
});

test('CRLF, indented fences and info strings', () => {
  assert.deepEqual(blocks('  ```code-ref\r\n{"a": 1}\r\n  ```\r\n'), ['{"a": 1}']);
  assert.deepEqual(blocks('    ```code-ref\n{}\n```\n'), []);
  assert.deepEqual(blocks('````code-ref\n```\n{}\n````\n'), ['```\n{}']);
  assert.deepEqual(blocks('``` code-ref \n{}\n```\n'), ['{}']);
  assert.deepEqual(blocks('```code-ref`\n{}\n```\n'), []);
});

test('each block has its own size limit', () => {
  assert.deepEqual(taskProblems(`\`\`\`code-ref\n${' '.repeat(MAX_BYTES + 1)}\n\`\`\`\n`), [[0, [`larger than ${MAX_BYTES} bytes`]]]);
});
