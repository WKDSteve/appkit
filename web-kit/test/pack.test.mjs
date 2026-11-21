// The packer of the single-file page: what the file holds, what it escapes, and what the check refuses, for a page with
// the four parts of an engine and for one without any part (no WASM, no worker). Synthetic script, style and parts.
//
//   node --test test/pack.test.mjs        (from web-kit/)

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { gunzipSync } from 'node:zlib';
import {
  assemble,
  checkPartIds,
  CSP_STRICT,
  CSP_WASM_WORKER,
  inlineScript,
  inlineStyle,
  policyProblems,
  readDist,
  verify,
  verifyShell,
  writePage,
} from '../src/pack.js';

const ENGINE = ['worker', 'wasm', 'bundle', 'shim'];
const engineParts = Object.fromEntries(ENGINE.map((id) => [id, Buffer.from(`part ${id} ☃`)]));
const base = { css: 'body{margin:0}', js: 'console.log(1)', title: 'app', comment: 'app web variant, build abc1234' };
const engine = (over = {}) => assemble({ ...base, partIds: ENGINE, parts: engineParts, csp: CSP_WASM_WORKER, ...over });
const plain = (over = {}) => assemble({ ...base, csp: CSP_STRICT, ...over });

test('the file holds the policy, the script and style inline, and the parts packed in their order', () => {
  const html = engine();
  assert.deepEqual(verifyShell(html, { partIds: ENGINE }), []);
  assert.ok(html.includes(`<meta http-equiv="Content-Security-Policy" content="${CSP_WASM_WORKER}">`));
  assert.match(html, /<title>app<\/title>/);
  assert.match(html, /<!-- app web variant, build abc1234 -->/);
  assert.match(html, /<style>body\{margin:0\}<\/style>/);
  assert.match(html, /<script type="module">console\.log\(1\)<\/script>/);
  const order = [...html.matchAll(/<script id="([a-z]+)"/g)].map((m) => m[1]);
  assert.deepEqual(order, ENGINE);
  for (const id of ENGINE) {
    const m = html.match(new RegExp(`<script id="${id}" type="application/octet-stream">([^<]*)</script>`));
    assert.ok(m, id);
    assert.equal(gunzipSync(Buffer.from(m[1].replace(/\s+/g, ''), 'base64')).toString('utf8'), `part ${id} ☃`, id);
  }
});

test('a page without WASM and worker: no part, the strict policy, and the check is content', () => {
  const html = plain();
  assert.deepEqual(verifyShell(html), []);
  assert.deepEqual(verifyShell(html, { partIds: [] }), []);
  assert.ok(!html.includes('application/octet-stream'), 'no part');
  assert.ok(html.includes(`content="${CSP_STRICT}"`));
  // The lines of the shell: no empty line where the parts would be.
  assert.ok(html.includes('<div id="app"></div>\n<script type="module">'));
  // The same page, asked to carry a part it does not have, is refused.
  assert.match(verifyShell(html, { partIds: ['data'] }).join('\n'), /part "data" is missing/);
  // One part of its own (data, no engine) is fine under the strict policy too.
  const withData = plain({ partIds: ['data'], parts: { data: Buffer.from('{}') } });
  assert.deepEqual(verifyShell(withData, { partIds: ['data'] }), []);
  assert.match(verifyShell(withData).join('\n'), /neither a part nor the module/, 'a part the check was not told of is a stray script');
});

test('the strict policy allows no WASM and no worker; both forbid every request and every outside source', () => {
  assert.ok(!CSP_STRICT.includes('wasm-unsafe-eval'));
  assert.ok(!CSP_STRICT.includes('worker-src'), 'a worker falls back to script-src, which allows no blob:');
  assert.ok(!/blob:/.test(CSP_STRICT.split('; ').find((d) => d.startsWith('script-src'))));
  assert.ok(CSP_WASM_WORKER.includes("'wasm-unsafe-eval'") && CSP_WASM_WORKER.includes('worker-src blob:'));
  for (const csp of [CSP_STRICT, CSP_WASM_WORKER]) {
    for (const directive of ["default-src 'none'", "connect-src 'none'", "base-uri 'none'", "form-action 'none'", "style-src 'unsafe-inline'"]) {
      assert.ok(csp.split('; ').includes(directive), `${directive} in ${csp}`);
    }
    assert.ok(!/https?:|\*/.test(csp), 'no address and no wildcard in the policy');
    assert.ok(!csp.includes("'unsafe-eval'"), 'no eval');
  }
});

test('the policy of the tasks page is the one it has shipped with', () => {
  assert.equal(
    CSP_WASM_WORKER,
    "default-src 'none'; script-src 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'unsafe-inline'; img-src data: blob:; worker-src blob:; connect-src 'none'; base-uri 'none'; form-action 'none'",
  );
});

test('a script cannot end its own element, a style cannot either', () => {
  const html = plain({ js: 'const a = "</script><img src=x>"; const b = "<!-- x";' });
  assert.deepEqual(verifyShell(html), [], 'the end tag in the script does not become markup');
  assert.equal(inlineScript('"</SCRIPT>"'), '"<\\/SCRIPT>"');
  assert.equal(inlineScript('"<!--"'), '"<\\!--"');
  assert.throws(() => inlineStyle('a{} </style><script>x</script>'), /<\/style/);
});

for (const [name, make, partIds] of [
  ['with the parts of an engine', engine, ENGINE],
  ['without WASM and worker', plain, []],
]) {
  test(`the check refuses a shell that loads something or lacks something (${name})`, () => {
    const good = make();
    const bad = (html, pattern, label) => assert.match(verifyShell(html, { partIds }).join('\n'), pattern, label);
    bad(good.replace('<script type="module">', '<script type="module" src="app.js">'), /loads a file \(src=\)/, 'a script with a source');
    bad(good.replace('</head>', '<link rel="stylesheet" href="style.css"></head>'), /loads a file \(href=\)/, 'a linked style');
    bad(good.replace('<div id="app">', '<img src="x.png"><div id="app">'), /src=/, 'an image');
    bad(good.replace('<div id="app">', '<a href="#x">x</a><div id="app">'), /href=/, 'a link');
    bad(good.replace('</head>', '<base href="/"></head>'), /href=/, 'a base');
    bad(good.replace('<div id="app">', '<object data="x.pdf"></object><div id="app">'), /data=/, 'an object');
    bad(good.replace('</head>', '<meta http-equiv="refresh" content="0;url=/x"></head>'), /refresh/, 'a refresh');
    bad(good.replace('<div id="app">', '<p>see https://example.com/x</p><div id="app">'), /URL in the page/, 'an address in the shell');
    bad(good.replace('<div id="app">', '<p>wss://example.com/x</p><div id="app">'), /URL in the page/, 'a socket address in the shell');
    bad(good.replace(/<meta http-equiv="Content-Security-Policy"[^>]*>/, ''), /CSP meta/, 'no policy');
    bad(good.replace(/connect-src 'none'/, "connect-src 'self'"), /CSP meta/, 'a policy that lets connections through');
    bad(good.replace('</body>', '<script>alert(1)</script></body>'), /neither a part nor the module/, 'a stray script');
    bad(good.replace('</body>', '<script id="worker">alert(1)</script></body>'), /neither a part nor the module/, 'a script that takes the name of a part');
    bad(good.replace('<script type="module">', '<script type="module">1</script><script type="module">'), /one inline module script, found 2/, 'two modules');
    bad(good.replace(/<script type="module">[\s\S]*?<\/script>/, ''), /one inline module script, found 0/, 'no module');
  });
}

test('a part that is missing from the file is refused', () => {
  assert.match(verifyShell(engine().replace('<script id="wasm"', '<script id="other"'), { partIds: ENGINE }).join('\n'), /part "wasm" is missing/);
});

test('what is inside the script and the style is not the shell: an address in the compiled code does not fail the check', () => {
  const html = plain({ js: 'const ns = "http://www.w3.org/2000/svg"; const t = `<a href="https://github.com/x">`;', css: '@namespace svg url(http://www.w3.org/2000/svg);' });
  assert.deepEqual(verifyShell(html), []);
});

test('parts must be bytes, each one named, each name a plain word', () => {
  assert.throws(() => engine({ parts: { worker: Buffer.from('x') } }), /parts missing: wasm, bundle, shim/);
  assert.throws(() => engine({ parts: { ...engineParts, wasm: 'text' } }), /parts missing: wasm/);
  assert.throws(() => plain({ parts: { stray: Buffer.from('x') } }), /parts not named: stray/);
  assert.throws(() => checkPartIds(['a', 'a']), /named twice/);
  assert.throws(() => checkPartIds(['app']), /mounts on/);
  for (const id of ['A', 'x"y', 'a b', '', '1a', 'a|b']) assert.throws(() => checkPartIds([id]), /plain word/, id);
  assert.throws(() => verifyShell(plain(), { partIds: ['x)|(.*'] }), /plain word/, 'an id never becomes a pattern');
  assert.deepEqual(checkPartIds(['worker', 'wasm-2']), ['worker', 'wasm-2']);
});

// Policies an app might give that let the page load or reach something; each one names the problem the check reports.
const LOOSE = [
  ["default-src 'none'; connect-src 'none' *; base-uri 'none'; form-action 'none'", /connect-src is not 'none' alone/],
  ["default-src *; script-src * 'unsafe-eval' 'unsafe-inline'; connect-src 'none'; base-uri 'none'; form-action 'none'", /default-src is not 'none' alone/],
  ["default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; connect-src 'none'; base-uri 'none'; form-action 'none'", /script-src names a source .*'unsafe-eval'/],
  ["default-src 'none'; img-src https://example.com; connect-src 'none'; base-uri 'none'; form-action 'none'", /img-src names a source .*example\.com/],
  ["default-src 'none'; style-src 'unsafe-inline' https:; connect-src 'none'; base-uri 'none'; form-action 'none'", /style-src names a source .*https:/],
  ["default-src 'none'; script-src 'self'; connect-src 'none'; base-uri 'none'; form-action 'none'", /script-src names a source .*'self'/],
  ["default-src 'none'; script-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'", /script from data:/],
  ["default-src 'none'; img-src 'none' data:; connect-src 'none'; base-uri 'none'; form-action 'none'", /'none' next to other sources/],
  ["default-src 'none'; connect-src 'none'; form-action 'none'", /base-uri is missing/],
  ["default-src 'none'; connect-src 'none'; base-uri 'none'", /form-action is missing/],
  ["script-src 'unsafe-inline'; connect-src 'none'; base-uri 'none'; form-action 'none'", /default-src is missing/],
  ["default-src 'none'; base-uri 'none'; form-action 'none'", /connect-src is missing/],
  ["default-src 'none'; connect-src 'none'; connect-src *; base-uri 'none'; form-action 'none'", /connect-src is given twice/],
  ["default-src 'none'; connect-src 'none'; base-uri 'none'; form-action 'none'; report-uri /r", /report-uri is not allowed/],
];

test('a policy that lets the page load or reach something is refused by assemble and by the check', () => {
  assert.deepEqual(policyProblems(CSP_STRICT), []);
  assert.deepEqual(policyProblems(CSP_WASM_WORKER), []);
  assert.deepEqual(policyProblems("default-src 'none'; connect-src 'none'; base-uri 'none'; form-action 'none'"), [], 'the least policy');
  for (const [csp, pattern] of LOOSE) {
    assert.match(policyProblems(csp).join('\n'), pattern, csp);
    assert.throws(() => plain({ csp }), /policy lets the page load or reach something/, csp);
    // A page whose policy was put in after assemble: the build check refuses it.
    const html = plain().replace(`content="${CSP_STRICT}"`, `content="${csp}"`);
    assert.ok(html.includes(csp), csp);
    assert.match(verifyShell(html).join('\n'), new RegExp('CSP meta is too loose: .*' + pattern.source), csp);
  }
  // A second policy meta is enforced too; a loose one is refused as well.
  const twice = plain().replace('</head>', `<meta http-equiv="Content-Security-Policy" content="${LOOSE[0][0]}"></head>`);
  assert.match(verifyShell(twice).join('\n'), /too loose: connect-src is not 'none' alone/);
  assert.match(policyProblems('').join('\n'), /empty/);
});

test('Trusted Types may be required, with named policies only: no wildcard, no duplicates, no other value', () => {
  const base = "default-src 'none'; connect-src 'none'; base-uri 'none'; form-action 'none'";
  for (const tt of [
    "require-trusted-types-for 'script'; trusted-types docmap",
    "require-trusted-types-for 'script'; trusted-types 'none'",
    "require-trusted-types-for 'script'; trusted-types app#1 other.policy",
    "require-trusted-types-for 'script'",
  ]) {
    assert.deepEqual(policyProblems(`${base}; ${tt}`), [], tt);
    // assemble takes it as well, and the check of the built page does too
    assert.doesNotThrow(() => plain({ csp: `${base}; ${tt}` }), tt);
  }
  for (const [tt, pattern] of [
    ["require-trusted-types-for 'style'", /require-trusted-types-for is not 'script' alone/],
    ["require-trusted-types-for", /require-trusted-types-for is not 'script' alone/],
    ["require-trusted-types-for 'script' 'script'", /require-trusted-types-for is not 'script' alone/],
    ["trusted-types", /trusted-types names no policy/],
    ["trusted-types *", /not a policy name/],
    ["trusted-types docmap 'allow-duplicates'", /not a policy name/],
    ["trusted-types 'none' docmap", /'none' next to policies/],
    ["trusted-types https://evil.example", /not a policy name/],
  ]) {
    assert.match(policyProblems(`${base}; ${tt}`).join('\n'), pattern, tt);
    assert.throws(() => plain({ csp: `${base}; ${tt}` }), /policy lets the page load or reach something/, tt);
  }
  // given twice, like every other directive
  assert.match(policyProblems(`${base}; trusted-types a; trusted-types b`).join('\n'), /trusted-types is given twice/);
});

test('a script tag is compared as a whole: an attribute inside another one does not make a script the module or a part', () => {
  for (const [name, make, partIds] of [
    ['with the parts of an engine', engine, ENGINE],
    ['without WASM and worker', plain, []],
  ]) {
    const good = make();
    const bad = (extra) => verifyShell(good.replace('<div id="app"></div>', `<div id="app"></div>${extra}`), { partIds }).join('\n');
    assert.match(bad('<script data-type="module">alert(1)</script>'), /neither a part nor the module: <script data-type="module">/, name);
    assert.match(bad('<script data-id="worker" type="application/octet-stream">x</script>'), /neither a part nor the module/, name);
    assert.match(bad('<script type="module" nomodule>x</script>'), /neither a part nor the module/, name);
    assert.match(bad('<SCRIPT type="module">x</SCRIPT>'), /neither a part nor the module/, name);
  }
  // A part that is there twice is refused: the page would read one of the two.
  const twice = engine().replace('<script type="module">', '<script id="wasm" type="application/octet-stream">x</script><script type="module">');
  assert.match(verifyShell(twice, { partIds: ENGINE }).join('\n'), /part "wasm" is there 2 times/);
});

test('the policy, the title and the comment are what the app gives, and plain', () => {
  assert.throws(() => plain({ csp: '' }), /policy/);
  assert.throws(() => plain({ csp: undefined }), /policy/);
  assert.throws(() => plain({ csp: 'a" onload="x' }), /policy/);
  assert.throws(() => plain({ title: '</title><script>' }), /title/);
  assert.throws(() => plain({ comment: 'x --> <script>' }), /comment/);
  assert.match(plain({ title: 'testing' }), /<title>testing<\/title>/);
});

test('verify: a page that names an address it is allowed to name', () => {
  const page = `<meta http-equiv="Content-Security-Policy" content="${CSP_STRICT}"><p>https://example.invalid/</p>`;
  assert.match(verify(page).join('\n'), /URL in the page/, 'none allowed by default');
  assert.deepEqual(verify(page, { allowUrls: ['https://example.invalid/'] }), []);
  assert.match(verify(page.replace('</p>', '__LEFT__</p>')).join('\n'), /placeholder/);
});

test('readDist finds the one script and the one style; writePage writes the file and its sum', () => {
  const dir = mkdtempSync(join(tmpdir(), 'appkit-web-pack-'));
  try {
    mkdirSync(join(dir, 'assets'));
    writeFileSync(join(dir, 'assets', 'a.js'), 'js!');
    writeFileSync(join(dir, 'assets', 'a.css'), 'css!');
    writeFileSync(join(dir, 'index.web.html'), '<script type="module" crossorigin src="./assets/a.js"></script><link rel="stylesheet" crossorigin href="./assets/a.css">');
    assert.deepEqual(readDist(dir, 'index.web.html'), { js: 'js!', css: 'css!' });
    writeFileSync(join(dir, 'index.html'), '<script type="module" src="./assets/a.js"></script><script type="module" src="./assets/a.js"></script>');
    assert.throws(() => readDist(dir), /expected one script/);
    const sha = writePage(join(dir, 'out'), 'page.html', 'hello');
    assert.equal(sha, '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824');
    assert.equal(readFileSync(join(dir, 'out', 'page.html'), 'utf8'), 'hello');
    assert.equal(readFileSync(join(dir, 'out', 'SHA256SUMS'), 'utf8'), `${sha}  page.html\n`);
    assert.throws(() => writePage(join(dir, 'out'), '../x.html', 'x'), /plain file name/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
