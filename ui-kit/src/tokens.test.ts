// The colours of the areas (`--area-1` to `--area-8` in tokens.css), computed from the values in the file: each has
// a contrast of at least 3:1 on the surfaces of both themes, all eight are apart, and next ones stay apart for a
// person with red-green blindness (simulated, Machado et al. 2009, severity 1).
import { describe, expect, it } from 'vitest';
import css from './tokens.css?raw';

type Pair = { light: string; dark: string };

function token(name: string): Pair {
  const m = new RegExp(`--${name}:\\s*light-dark\\((#[0-9a-f]{6}),\\s*(#[0-9a-f]{6})\\)`, 'i').exec(css);
  if (!m?.[1] || !m[2]) throw new Error(`--${name} is not a light-dark() of two hex colours`);
  return { light: m[1], dark: m[2] };
}

type Rgb = [number, number, number];

function linear(hex: string): Rgb {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  return c.map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)) as Rgb;
}

function luminance(hex: string): number {
  const [r, g, b] = linear(hex);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

const DEUTAN: Rgb[] = [
  [0.367322, 0.860646, -0.227968],
  [0.280085, 0.672501, 0.047413],
  [-0.01182, 0.04294, 0.968881],
];
const PROTAN: Rgb[] = [
  [0.152286, 1.052583, -0.204868],
  [0.114503, 0.786281, 0.099216],
  [-0.003882, -0.048116, 1.051998],
];

/** The colour in OKLab, as a person with the given deficiency sees it (none: as it is). */
function oklab(hex: string, matrix?: Rgb[]): Rgb {
  let v = linear(hex);
  if (matrix) {
    v = matrix.map((row) => Math.min(1, Math.max(0, row[0] * v[0] + row[1] * v[1] + row[2] * v[2]))) as Rgb;
  }
  const [r, g, b] = v;
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function distance(a: string, b: string, matrix?: Rgb[]): number {
  const [x, y] = [oklab(a, matrix), oklab(b, matrix)];
  return Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]);
}

const areas = Array.from({ length: 8 }, (_, i) => token(`area-${i + 1}`));
const surfaces = [token('surface'), token('surface-2')];
const themes = ['light', 'dark'] as const;

describe('the colours of the areas', () => {
  it('have at least 3:1 on both surfaces in both themes', () => {
    for (const theme of themes) {
      areas.forEach((area, i) => {
        for (const surface of surfaces) {
          const ratio = contrast(area[theme], surface[theme]);
          expect(ratio, `--area-${i + 1} ${theme} on ${surface[theme]}`).toBeGreaterThanOrEqual(3);
        }
      });
    }
  });

  it('are all apart, and next ones stay apart for red-green blindness', () => {
    for (const theme of themes) {
      const cols: string[] = areas.map((a) => a[theme]);
      for (let i = 0; i < cols.length; i++) {
        for (let j = i + 1; j < cols.length; j++) {
          expect(distance(cols[i]!, cols[j]!), `--area-${i + 1} and -${j + 1} ${theme}`).toBeGreaterThan(0.05);
        }
      }
      for (let i = 0; i + 1 < cols.length; i++) {
        for (const [name, matrix] of [['deutan', DEUTAN], ['protan', PROTAN]] as const) {
          const d = distance(cols[i]!, cols[i + 1]!, matrix);
          expect(d, `--area-${i + 1} and -${i + 2} ${theme} ${name}`).toBeGreaterThan(0.1);
        }
      }
    }
  });

  it('are not the blue of the accent', () => {
    const accent = token('accent');
    for (const theme of themes) {
      areas.forEach((area, i) => {
        expect(distance(area[theme], accent[theme]), `--area-${i + 1} ${theme}`).toBeGreaterThan(0.06);
      });
    }
  });
});

// The colours of text, as they are used on the surfaces, in both themes: what axe measures in a browser, computed here
// from the file because the tests of the kit run in jsdom, which has no layout and no colours.
describe('the colours of text', () => {
  const pages = [token('bg'), ...surfaces];
  const texts: [string, number][] = [
    ['text', 7],
    ['muted', 4.5],
    ['accent', 4.5],
    ['ok', 4.5],
    ['warn', 4.5],
    ['bad', 4.5],
  ];
  for (const [name, least] of texts) {
    it(`--${name} has at least ${least}:1 on the page and on both surfaces in both themes`, () => {
      const colour = token(name);
      for (const theme of themes) {
        for (const page of pages) {
          expect(contrast(colour[theme], page[theme]), `--${name} ${theme} on ${page[theme]}`).toBeGreaterThanOrEqual(least);
        }
      }
    });
  }

  it('--accent-text has at least 4.5:1 on --accent in both themes', () => {
    const [on, accent] = [token('accent-text'), token('accent')];
    for (const theme of themes) expect(contrast(on[theme], accent[theme]), theme).toBeGreaterThanOrEqual(4.5);
  });
});
