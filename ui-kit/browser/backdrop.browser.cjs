// The backdrop of a modal dialog in a real browser (Chromium), both ways of `closeOnBackdrop`: the browser's own
// `closedby="any"` and, with `closedBy` taken off the prototype as in a browser that does not know it (Safari, the
// Chrome 86 of tasks-desktop), the listeners. jsdom has no layout and no light dismiss, so `backdrop.dom.test.ts`
// cannot show this. Not part of `pnpm test`: it needs a Chromium with Playwright, which the kit does not install.
//
//   node ui-kit/browser/backdrop.browser.cjs        (from the root of appkit, after `pnpm install` in ui-kit)
//
// `require('playwright')` must resolve (a global install, or NODE_PATH); a Chromium older than 134 runs only the
// listeners and says so.
'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright');
const ts = require(require.resolve('typescript', { paths: [path.join(__dirname, '..')] }));

const source = fs.readFileSync(path.join(__dirname, '..', 'src', 'backdrop.ts'), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
});

// The dialog is 200 x 200 at 200, 200 (plus padding): (50, 50) is the backdrop, the text lies at about (240, 235).
const PAGE = `<!doctype html><meta charset="utf-8">
<style>dialog { position: fixed; inset: auto; left: 200px; top: 200px; margin: 0; width: 200px; height: 200px; }</style>
<dialog><p id="text">Some text to select out of the dialog</p></dialog>`;

async function run(browser, way) {
  const page = await browser.newPage({ viewport: { width: 800, height: 800 } });
  await page.setContent(PAGE);
  if (way === 'listeners') await page.evaluate(() => delete HTMLDialogElement.prototype.closedBy);
  await page.addScriptTag({ content: `(function (exports) {\n${outputText}\n})(window.backdrop = {});` });

  // Opens the dialog afresh with the action and a log of its events; `prevent` keeps it on cancel.
  const open = (opts = {}) =>
    page.evaluate(async ({ prevent, closeable }) => {
      const dialog = document.querySelector('dialog');
      if (window.action) window.action.destroy();
      if (dialog.open) dialog.close();
      await new Promise((r) => setTimeout(r, 30)); // the close event of the previous round is queued
      window.seen = [];
      dialog.oncancel = (e) => {
        window.seen.push(`cancel${e.cancelable ? '' : ' (not cancelable)'}`);
        if (prevent) e.preventDefault();
      };
      dialog.onclose = () => window.seen.push('close');
      const options = closeable === undefined ? {} : { closeable: () => closeable };
      window.action = window.backdrop.closeOnBackdrop(dialog, options);
      dialog.showModal();
      return dialog.getAttribute('closedby');
    }, opts);
  const state = async () => {
    await page.waitForTimeout(30);
    return page.evaluate(() => ({ open: document.querySelector('dialog').open, seen: window.seen }));
  };

  const results = [];
  const check = async (name, fn) => {
    await fn();
    results.push(`${way}: ${name}`);
  };

  await check('the action takes the way of this browser', async () => {
    assert.equal(await open(), way === 'closedby' ? 'any' : null);
  });
  await check('a click beside the dialog closes it, cancel first', async () => {
    await open();
    await page.mouse.click(50, 50);
    assert.deepEqual(await state(), { open: false, seen: ['cancel', 'close'] });
  });
  await check('a text selection out of the dialog onto the backdrop keeps it', async () => {
    await open();
    const box = await page.locator('#text').boundingBox();
    await page.mouse.move(box.x + 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(50, 50, { steps: 8 });
    await page.mouse.up();
    assert.notEqual(await page.evaluate(() => String(window.getSelection())), '', 'text was selected');
    assert.deepEqual(await state(), { open: true, seen: [] });
  });
  await check('a press on the backdrop released in the dialog keeps it', async () => {
    await open();
    await page.mouse.move(50, 50);
    await page.mouse.down();
    await page.mouse.move(300, 300, { steps: 8 });
    await page.mouse.up();
    assert.deepEqual(await state(), { open: true, seen: [] });
  });
  await check('a click in the dialog keeps it', async () => {
    await open();
    await page.mouse.click(300, 300);
    assert.deepEqual(await state(), { open: true, seen: [] });
  });
  await check('a dialog that prevents cancel stays open, also on the second click', async () => {
    await open({ prevent: true });
    await page.mouse.click(50, 50);
    await page.mouse.click(60, 60);
    assert.deepEqual(await state(), { open: true, seen: ['cancel', 'cancel'] });
  });
  await check('Escape still closes it', async () => {
    await open();
    await page.keyboard.press('Escape');
    assert.deepEqual(await state(), { open: false, seen: ['cancel', 'close'] });
  });
  await check('closeable false keeps it (listeners in every browser), true lets it go', async () => {
    assert.equal(await open({ closeable: false }), null);
    await page.mouse.click(50, 50);
    assert.deepEqual(await state(), { open: true, seen: [] });
    await open({ closeable: true });
    await page.mouse.click(50, 50);
    assert.deepEqual(await state(), { open: false, seen: ['cancel', 'close'] });
  });
  await page.close();
  return results;
}

(async () => {
  const browser = await chromium.launch();
  try {
    const probe = await browser.newPage();
    const native = await probe.evaluate(() => 'closedBy' in HTMLDialogElement.prototype);
    await probe.close();
    console.log(`Chromium ${browser.version()}, closedby ${native ? 'known' : 'not known'}`);
    const ways = native ? ['closedby', 'listeners'] : ['listeners'];
    for (const way of ways) for (const line of await run(browser, way)) console.log(`ok - ${line}`);
  } finally {
    await browser.close();
  }
})().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
