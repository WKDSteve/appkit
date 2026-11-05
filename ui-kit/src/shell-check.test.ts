// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import kitTokens from './tokens.css?raw';
import { collectShell, diffShell, paletteRecolor, SHELL_PROBES, SHELL_PROPS, type ShellSignature, type ShellStyle } from './shell-check';

const style = (over: Partial<ShellStyle> = {}): ShellStyle => {
  const base = Object.fromEntries(SHELL_PROPS.map((p) => [p, 'x'])) as ShellStyle;
  return { ...base, ...over };
};

describe('diffShell', () => {
  it('is empty for the same shell', () => {
    const sig: ShellSignature = { page: style(), trigger: style({ color: 'red' }) };
    expect(diffShell('light', sig, structuredClone(sig))).toEqual([]);
  });

  it('names the probe and the property that differ, with both values', () => {
    const golden: ShellSignature = { trigger: style({ borderRadius: '6px' }) };
    const actual: ShellSignature = { trigger: style({ borderRadius: '2px' }) };
    expect(diffShell('dark', actual, golden)).toEqual(['dark: trigger.borderRadius is "2px", the shell has "6px"']);
  });

  it('says so when a part is missing, and when a part is there that the reference does not know', () => {
    const lines = diffShell('light', { page: style(), extra: style() }, { page: style(), menubar: style() });
    expect(lines).toEqual(['light: menubar is not on the page', 'light: extra is on the page but not in the reference']);
  });
});

describe('paletteRecolor', () => {
  const tokens = `:root {
    /* a comment with --fake: #123456; */
    --bg: light-dark(#f7f7f5, #15171a);
    --surface: light-dark(#ffffff, #1d2024);
    --accent-text: light-dark(#ffffff, #0b1a33);
    --line: light-dark(#d4d4cf, #343942);
    --accent-soft: color-mix(in srgb, var(--accent) 14%, transparent);
    --radius: 6px;
  }`;
  const palette = `:root {
    --bg: light-dark(#f4f8f6, #101513);
    --surface: light-dark(#fff, #191e1c);
    --accent-text: light-dark(#000000, #061a13);
    --accent-50: #eaf9f3;
  }`;

  it('maps the colour of each token of the kit to the colour the palette gives it, per theme', () => {
    const r = paletteRecolor(tokens, palette);
    expect(r.light['rgb(247, 247, 245)']).toEqual(['rgb(244, 248, 246)']);
    expect(r.dark['rgb(21, 23, 26)']).toEqual(['rgb(16, 21, 19)']);
    expect(r.dark['rgb(29, 32, 36)']).toEqual(['rgb(25, 30, 28)']);
  });

  it('keeps both colours where two tokens of the kit share one that the palette parts', () => {
    // --surface and --accent-text are both white in the light kit; the palette keeps one white and makes one black.
    expect(paletteRecolor(tokens, palette).light['rgb(255, 255, 255)']).toEqual(['rgb(255, 255, 255)', 'rgb(0, 0, 0)']);
  });

  it('names no token the palette leaves alone, and reads no comment, mix or length', () => {
    const r = paletteRecolor(tokens, palette);
    expect(r.light['rgb(212, 212, 207)']).toBeUndefined();
    expect(r.light['rgb(18, 52, 86)']).toBeUndefined();
    // The light white of --surface and --accent-text is one colour of the kit.
    expect(Object.keys(r.light)).toHaveLength(2);
  });
});

describe('diffShell with a palette', () => {
  const recolor = { 'rgb(29, 32, 36)': ['rgb(25, 30, 28)'] };
  const golden: ShellSignature = { palette: style({ backgroundColor: 'rgb(29, 32, 36)', borderRadius: '10px' }) };

  it('takes the colour of the palette for the colour of a token', () => {
    const actual: ShellSignature = { palette: style({ backgroundColor: 'rgb(25, 30, 28)', borderRadius: '10px' }) };
    expect(diffShell('dark', actual, golden, recolor)).toEqual([]);
  });

  it('no longer takes the colour of the kit once the palette gives the token another one', () => {
    const actual: ShellSignature = { palette: style({ backgroundColor: 'rgb(29, 32, 36)', borderRadius: '10px' }) };
    expect(diffShell('dark', actual, golden, recolor)).toEqual([
      'dark: palette.backgroundColor is "rgb(29, 32, 36)", the shell has "rgb(25, 30, 28)" ("rgb(29, 32, 36)" recoloured by the palette)',
    ]);
  });

  it('compares everything that is no colour as before', () => {
    const actual: ShellSignature = { palette: style({ backgroundColor: 'rgb(25, 30, 28)', borderRadius: '2px' }) };
    expect(diffShell('dark', actual, golden, recolor)).toEqual(['dark: palette.borderRadius is "2px", the shell has "10px"']);
  });

  it('recolours only the properties that hold a colour', () => {
    const g: ShellSignature = { page: style({ fontFamily: 'rgb(29, 32, 36)' }) };
    const a: ShellSignature = { page: style({ fontFamily: 'rgb(25, 30, 28)' }) };
    expect(diffShell('dark', a, g, recolor)).toHaveLength(1);
  });

  it('works with the real tokens of the kit', () => {
    const r = paletteRecolor(kitTokens, ':root { --text: light-dark(#161c1a, #e5e9e7); }');
    expect(r.dark).toEqual({ 'rgb(232, 233, 234)': ['rgb(229, 233, 231)'] });
  });
});

describe('collectShell', () => {
  it('reads the first element of each probe that is there, and skips the others', () => {
    document.body.innerHTML = '<nav role="menubar" style="color: rgb(1, 2, 3); border-radius: 4px"></nav><nav role="menubar"></nav>';
    const got = collectShell({ menubar: '[role="menubar"]', dialog: 'dialog[open]' });
    expect(Object.keys(got)).toEqual(['menubar']);
    expect(got['menubar']?.['color']).toBe('rgb(1, 2, 3)');
    expect(got['menubar']?.['borderRadius']).toBe('4px');
    expect(Object.keys(got['menubar'] ?? {})).toEqual([...SHELL_PROPS]);
  });

  it('is self-contained: it works when it is sent to a browser as text', () => {
    // A function that names nothing outside itself survives `toString`, which is how Playwright sends it.
    const sent = new Function(`return (${collectShell.toString()})`)() as typeof collectShell;
    document.body.innerHTML = '<nav role="menubar" style="color: rgb(9, 9, 9)"></nav>';
    expect(sent({ menubar: '[role="menubar"]' })['menubar']?.['color']).toBe('rgb(9, 9, 9)');
  });
});

describe('the probe of a menu item', () => {
  it('is a plain action: a disabled one and a checked one are not the one that is read', () => {
    document.body.innerHTML = `<div role="menu">
      <button class="mb-item" disabled style="color: rgb(1, 1, 1)"></button>
      <button class="mb-item" role="menuitemradio" aria-checked="true" style="color: rgb(2, 2, 2)"></button>
      <button class="mb-item" role="menuitemcheckbox" aria-checked="false" style="color: rgb(3, 3, 3)"></button>
      <button class="mb-item" role="menuitem" style="color: rgb(4, 4, 4)"></button>
    </div>`;
    expect(collectShell(SHELL_PROBES)['menuitem']?.['color']).toBe('rgb(4, 4, 4)');
  });
});
