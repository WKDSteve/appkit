// The check that the shell of every app looks like the same one. The parts of the shell the apps share (the page, the
// menu bar and an open menu, the palette) are read in a browser as the styles the browser works out for them (colour,
// type, line, shape, shadow), and compared with one reference kept with the kit. Styles, not pixels: a screenshot depends
// on the fonts of the machine, the styles do not, and a page that overrides a token or a rule of the kit shows as the
// property it changed.
//
// In a test (Playwright): `await page.evaluate(collectShell, SHELL_PROBES)` in each state (closed, a menu open, the
// palette open) and `diffShell(theme, collected, golden)`; an empty list is a pass.
//
// An app with a palette (the colours of its icon, loaded after tokens.css) differs from the reference in its colours by
// design. It passes `paletteRecolor(tokensCss, paletteCss)[theme]` as the fourth argument: a colour of the reference
// that is the value of a token of the kit is then met by the value the palette gives that token, and only by it. Type,
// line, shape and every colour that is no token are compared as before.

/** What is read of an element. */
export const SHELL_PROPS = [
  'color',
  'backgroundColor',
  'fontFamily',
  'fontSize',
  'fontWeight',
  'lineHeight',
  'borderTopColor',
  'borderTopStyle',
  'borderTopWidth',
  'borderRadius',
  'boxShadow',
  'paddingTop',
  'paddingLeft',
  'outlineStyle',
];

/** The parts every app has, by the selector that finds each. The ones of an open menu or palette exist only then. */
export const SHELL_PROBES = {
  page: 'body',
  menubar: '[role="menubar"]',
  trigger: '[role="menubar"] button.mb-trigger',
  // A plain action: not a checked or a disabled one, whose looks differ by design and would make the first item of a menu a moving target.
  menuitem: '[role="menu"] button.mb-item:not(:disabled):not([aria-checked])',
  palette: 'dialog[open]:has(input[role="combobox"])',
  paletteInput: 'dialog[open] input[role="combobox"]',
  paletteOption: 'dialog[open] [role="option"][aria-selected="true"]',
};

/**
 * Runs in the page: the styles of the first element of each probe that is there now. Self-contained (it is sent to the
 * browser as text), so it names nothing of this file.
 */
export function collectShell(probes) {
  const props = [
    'color',
    'backgroundColor',
    'fontFamily',
    'fontSize',
    'fontWeight',
    'lineHeight',
    'borderTopColor',
    'borderTopStyle',
    'borderTopWidth',
    'borderRadius',
    'boxShadow',
    'paddingTop',
    'paddingLeft',
    'outlineStyle',
  ];
  const out = {};
  for (const [name, selector] of Object.entries(probes)) {
    const el = document.querySelector(selector);
    if (!el) continue;
    const css = getComputedStyle(el);
    const style = {};
    for (const prop of props) style[prop] = css[prop] ?? '';
    out[name] = style;
  }
  return out;
}

/** The properties that hold one colour, and so may be recoloured by a palette. */
const COLOR_PROPS = new Set(['color', 'backgroundColor', 'borderTopColor']);

/**
 * What is different: one line per probe and property, and one for a probe that the page does not have. `recolor` (the
 * one theme of `paletteRecolor`) lets a colour of the reference be met by what the palette gives the same token.
 */
export function diffShell(label, actual, golden, recolor = {}) {
  const lines = [];
  for (const [name, want] of Object.entries(golden)) {
    const got = actual[name];
    if (!got) {
      lines.push(`${label}: ${name} is not on the page`);
      continue;
    }
    for (const prop of SHELL_PROPS) {
      const allowed = (COLOR_PROPS.has(prop) && recolor[want[prop]]) || [want[prop]];
      if (!allowed.includes(got[prop])) {
        const has = allowed.length === 1 ? JSON.stringify(allowed[0]) : allowed.map((v) => JSON.stringify(v)).join(' or ');
        const of = allowed[0] === want[prop] ? '' : ` (${JSON.stringify(want[prop])} recoloured by the palette)`;
        lines.push(`${label}: ${name}.${prop} is ${JSON.stringify(got[prop])}, the shell has ${has}${of}`);
      }
    }
  }
  for (const name of Object.keys(actual)) {
    if (!(name in golden)) lines.push(`${label}: ${name} is on the page but not in the reference`);
  }
  return lines;
}

/** `#rgb` or `#rrggbb` as the browser writes a computed colour (`rgb(r, g, b)`); anything else is no colour here. */
function computed(hex) {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join('') : m[1];
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}

/** The colour tokens of a stylesheet: name to [light, dark] (a plain colour is both, `light-dark(a, b)` is a and b). */
function colorTokens(css) {
  const out = new Map();
  const text = css.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const m of text.matchAll(/(--[\w-]+)\s*:\s*([^;{}]+);/g)) {
    const value = m[2].trim();
    const pair = /^light-dark\(\s*([^,()]+?)\s*,\s*([^,()]+?)\s*\)$/i.exec(value);
    const light = computed(pair ? pair[1] : value);
    const dark = computed(pair ? pair[2] : value);
    if (light && dark) out.set(m[1], [light, dark]);
  }
  return out;
}

/**
 * Per theme, each colour of a token of the kit (`tokens.css`) to the colours the palette gives the same token or
 * tokens (two tokens of the kit can share a colour that the palette parts). A token the palette leaves alone is not
 * named, so its colour is still the kit's.
 */
export function paletteRecolor(tokensCss, paletteCss) {
  const kit = colorTokens(tokensCss);
  const palette = colorTokens(paletteCss);
  const out = { light: {}, dark: {} };
  for (const [name, [pl, pd]] of palette) {
    const k = kit.get(name);
    if (!k) continue;
    for (const [theme, from, to] of [['light', k[0], pl], ['dark', k[1], pd]]) {
      const list = (out[theme][from] ??= []);
      if (!list.includes(to)) list.push(to);
    }
  }
  return out;
}
