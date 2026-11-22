// The submenu of the menu bar in a real browser (Chromium): the example page `menubar/` is built with Vite, served
// with a strict CSP, and checked in light and dark: beside the menu on the right, flipped to the left at the last
// menu, the pointer's corridor, folded open in a narrow window without a sideways scroll, and axe without a finding.
// A second round takes the Popover API away (the fallback where a browser lacks it): the submenu is then a fixed box
// of the page. jsdom has no layout, so `MenuBar.submenu.dom.test.ts` cannot show this. Not part of `pnpm test`: it
// needs Playwright and axe, which the kit does not install.
//
//   node ui-kit/browser/menubar.browser.cjs        (from the root of appkit, after `pnpm install` in ui-kit)
//
// `require('playwright')` (or `@playwright/test`) and `require('@axe-core/playwright')` must resolve (a global
// install, or NODE_PATH, such as the node_modules of an app's ui: NODE_PATH=../tasks-desktop/ui/node_modules).
'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { AxeBuilder } = require('@axe-core/playwright');

// `playwright`, or the `@playwright/test` that an app's ui has for its own browser tests.
function loadChromium() {
  try {
    return require('playwright').chromium;
  } catch {
    return require('@playwright/test').chromium;
  }
}
const chromium = loadChromium();

const KIT = path.join(__dirname, '..');
const CSP = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:";
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' };

async function buildPage() {
  const vite = await import(pathToFileURL(path.join(KIT, 'node_modules/vite/dist/node/index.js')).href);
  const plugin = await import(
    pathToFileURL(path.join(KIT, 'node_modules/@sveltejs/vite-plugin-svelte/src/index.js')).href
  );
  const out = fs.mkdtempSync(path.join(os.tmpdir(), 'appkit-menubar-'));
  await vite.build({
    configFile: false,
    root: path.join(__dirname, 'menubar'),
    base: './',
    logLevel: 'warn',
    plugins: [plugin.svelte()],
    resolve: { conditions: ['browser'] },
    build: { outDir: out, emptyOutDir: true },
  });
  return out;
}

function serve(dir) {
  const server = http.createServer((req, res) => {
    const name = decodeURIComponent(new URL(req.url, 'http://x').pathname).replace(/^\/+/, '') || 'index.html';
    const file = path.resolve(dir, name);
    if (!file.startsWith(dir) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, {
      'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream',
      'content-security-policy': CSP,
    });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server)));
}

const box = (page, selector) =>
  page.locator(selector).first().evaluate((el) => {
    const r = el.getBoundingClientRect();
    return { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
  });
const noSideScroll = (page) =>
  page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);
const activeText = (page) => page.evaluate(() => document.activeElement?.querySelector('.mb-text')?.textContent);

async function axe(page, what) {
  const result = await new AxeBuilder({ page }).analyze();
  assert.deepEqual(
    result.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).join(', ')}`),
    [],
    `${what}: axe`,
  );
}

/** The menu View open, the focus on its item Lernpläne. */
async function toPlans(page) {
  await page.locator('button.mb-trigger[data-for="view"]').focus();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('End');
  await page.keyboard.press('ArrowUp');
  assert.equal(await activeText(page), 'Lernpläne');
}

async function round(browser, base, scheme, popover) {
  const what = `${scheme}${popover ? '' : ', no popover'}`;
  // A context of its own: axe opens frames through it.
  const context = await browser.newContext({ viewport: { width: 1000, height: 700 }, colorScheme: scheme });
  const page = await context.newPage();
  const violations = [];
  page.on('console', (m) => {
    if (m.type() === 'error') violations.push(m.text());
  });
  await page.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (e) => console.error(`csp ${e.violatedDirective}`));
  });
  if (!popover) {
    await page.addInitScript(() => {
      delete HTMLElement.prototype.showPopover;
      delete HTMLElement.prototype.hidePopover;
    });
  }
  await page.goto(base);

  // Beside the menu, on the right; its name says which item inside is checked.
  await toPlans(page);
  const parent = page.getByRole('menuitem', { name: 'Lernpläne, aktuell: Kompetenzkarte' });
  assert.equal(await parent.getAttribute('aria-expanded'), 'false');
  await page.keyboard.press('ArrowRight');
  assert.equal(await activeText(page), 'Kompetenzkarte');
  assert.equal(await parent.getAttribute('aria-expanded'), 'true');
  const sub = page.getByRole('menu', { name: 'Lernpläne' });
  assert.equal(await sub.getAttribute('data-side'), 'right');
  const p = await box(page, '[data-sub-for]:is([aria-expanded="true"])');
  const s = await box(page, '[data-sub]');
  assert.ok(s.left >= p.right - 2, `${what}: beside its item (${s.left} / ${p.right})`);
  assert.ok(Math.abs(s.top - p.top) < 12, `${what}: level with its item (${s.top} / ${p.top})`);
  assert.ok(await sub.isVisible(), `${what}: the submenu is seen`);
  assert.ok(await noSideScroll(page), `${what}: no sideways scroll`);
  await axe(page, `${what}, submenu beside`);
  await page.keyboard.press('ArrowDown');
  assert.equal(await activeText(page), 'Lernpfade');
  await page.keyboard.press('Escape');
  assert.equal(await activeText(page), 'Lernpläne');
  assert.equal(await page.locator('[data-sub]').count(), 0);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('[data-menu]').count(), 0);

  // The last menu: no room on the right, so on the left.
  await page.setViewportSize({ width: 700, height: 700 });
  await page.locator('#bar').evaluate((el) => el.classList.add('end'));
  await page.locator('button.mb-trigger[data-for="options"]').focus();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('End');
  await page.keyboard.press('ArrowRight');
  assert.equal(await activeText(page), 'Deutsch');
  const left = page.getByRole('menu', { name: 'Sprache' });
  assert.equal(await left.getAttribute('data-side'), 'left');
  const lp = await box(page, '[data-sub-for]:is([aria-expanded="true"])');
  const ls = await box(page, '[data-sub]');
  assert.ok(ls.right <= lp.left + 2, `${what}: left of its item (${ls.right} / ${lp.left})`);
  assert.ok(ls.left >= 0, `${what}: inside the window`);
  assert.ok(await noSideScroll(page), `${what}: no sideways scroll on the left`);
  // Not clipped by the menu it belongs to: its first item is what the window shows at its place.
  const seen = await page.evaluate(() => {
    const first = document.querySelector('[data-sub] .mb-item');
    const r = first.getBoundingClientRect();
    return document.elementFromPoint(r.left + 6, r.top + r.height / 2)?.closest('.mb-item') === first;
  });
  assert.ok(seen, `${what}: the submenu on the left is not clipped`);
  await axe(page, `${what}, submenu on the left`);
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await page.locator('#bar').evaluate((el) => el.classList.remove('end'));

  // The pointer: a rest opens; on the way there over a neighbour it stays; straight down to a neighbour it closes.
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.locator('button.mb-trigger[data-for="view"]').click();
  const pb = await box(page, '[data-sub-for]');
  const cx = (pb.left + pb.right) / 2;
  const cy = (pb.top + pb.bottom) / 2;
  await page.mouse.move(cx, cy - 4);
  await page.mouse.move(cx, cy);
  await page.waitForTimeout(400);
  assert.equal(await page.locator('[data-sub]').count(), 1, `${what}: a rest opens it`);
  const sb = await box(page, '[data-sub]');
  const nb = await box(page, '.mb-item >> text=Als Markdown');
  const below = Math.min(nb.top + 2, (pb.bottom + nb.bottom) / 2);
  // Toward the bottom of the submenu, over the gap and the next item.
  const steps = 12;
  const tx = sb.left + 4;
  const ty = Math.max(sb.bottom - 4, below + 4);
  for (let i = 1; i <= steps; i++) {
    await page.mouse.move(pb.right - 20 + ((tx - pb.right + 20) * i) / steps, cy + ((ty - cy) * i) / steps);
  }
  await page.mouse.move(sb.left + 20, sb.top + 10);
  await page.waitForTimeout(500);
  assert.equal(await page.locator('[data-sub]').count(), 1, `${what}: on its way it stays`);
  await page.mouse.move(cx, cy);
  await page.mouse.move(cx - 30, nb.top + 4, { steps: 4 });
  await page.waitForTimeout(50);
  assert.equal(await page.locator('[data-sub]').count(), 0, `${what}: straight to a neighbour it closes`);
  await page.keyboard.press('Escape');

  // A narrow window: folded open inside the menu, no sideways scroll, in the window.
  await page.setViewportSize({ width: 380, height: 700 });
  await toPlans(page);
  await page.keyboard.press('ArrowRight');
  assert.equal(await activeText(page), 'Kompetenzkarte');
  const fold = page.locator('[data-menu="view"] [data-sub]');
  assert.equal(await fold.count(), 1, `${what}: folded inside the menu`);
  assert.equal(await page.locator('[data-sub-for] .mb-arrow').first().textContent(), '▾');
  assert.ok(await noSideScroll(page), `${what}: no sideways scroll when narrow`);
  const inside = await page.locator('[data-menu="view"]').evaluate((el) => el.scrollWidth <= el.clientWidth);
  assert.ok(inside, `${what}: the menu does not scroll sideways`);
  const fb = await box(page, '[data-menu="view"] [data-sub]');
  assert.ok(fb.right <= 380 && fb.left >= 0, `${what}: the folded list is in the window`);
  await axe(page, `${what}, folded`);
  await page.keyboard.press('ArrowLeft');
  assert.equal(await activeText(page), 'Lernpläne');

  assert.deepEqual(violations, [], `${what}: console errors or CSP violations`);
  await context.close();
  console.log(`ok  ${what}`);
}

/**
 * A menu taller than the window scrolls inside its popup. The scrolling box must be reachable by keyboard (axe
 * `scrollable-region-focusable`), and the arrow keys must scroll the item with the focus into view, down to the last.
 */
async function longMenu(browser, base, scheme, popover) {
  const what = `${scheme}${popover ? '' : ', no popover'}, long menu`;
  const context = await browser.newContext({ viewport: { width: 1000, height: 300 }, colorScheme: scheme });
  const page = await context.newPage();
  if (!popover) {
    await page.addInitScript(() => {
      delete HTMLElement.prototype.showPopover;
      delete HTMLElement.prototype.hidePopover;
    });
  }
  await page.goto(base);
  // Opened by a click, as in an app: the focus stays on the title, no item has it yet.
  await page.locator('button.mb-trigger[data-for="long"]').click();
  const popup = page.locator('[data-menu="long"]');
  const scrolls = await popup.evaluate((el) => el.scrollHeight > el.clientHeight);
  assert.ok(scrolls, `${what}: the popup scrolls in a low window`);
  await axe(page, `${what}, open`);
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('End');
  assert.equal(await activeText(page), 'Eintrag 30');
  const inView = await page.evaluate(() => {
    const item = document.activeElement.getBoundingClientRect();
    const box = document.activeElement.closest('[data-menu]').getBoundingClientRect();
    return item.top >= box.top - 1 && item.bottom <= box.bottom + 1;
  });
  assert.ok(inView, `${what}: the last item is scrolled into view`);
  await axe(page, `${what}, at the end`);
  await context.close();
  console.log(`ok  ${what}`);
}

(async () => {
  const dir = await buildPage();
  const server = await serve(dir);
  const base = `http://127.0.0.1:${server.address().port}/`;
  const browser = await chromium.launch();
  try {
    for (const popover of [true, false]) {
      for (const scheme of ['light', 'dark']) {
        await round(browser, base, scheme, popover);
        await longMenu(browser, base, scheme, popover);
      }
    }
  } finally {
    await browser.close();
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
