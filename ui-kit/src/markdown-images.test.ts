import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import Blocks from './Blocks.svelte';
import { drawableImageUrl, parseMarkdown, safeAssetPath, type Block, type ImageSource, type Inline } from './markdown';

/** Every image of a tree, in order. */
function images(blocks: Block[]): Extract<Inline, { t: 'image' }>[] {
  const out: Extract<Inline, { t: 'image' }>[] = [];
  const inline = (items: Inline[]): void => {
    for (const item of items) {
      if (item.t === 'image') out.push(item);
      else if (item.t === 'link' || item.t === 'em' || item.t === 'strong' || item.t === 'del') inline(item.children);
    }
  };
  const walk = (list: Block[]): void => {
    for (const block of list) {
      if (block.t === 'p' || block.t === 'h') inline(block.children);
      else if (block.t === 'quote') walk(block.children);
      else if (block.t === 'list') for (const item of block.items) walk(item);
      else if (block.t === 'table') {
        for (const cell of block.head) inline(cell);
        for (const row of block.rows) for (const cell of row) inline(cell);
      }
    }
  };
  walk(blocks);
  return out;
}

const source = (map: Record<string, string | null | undefined>, open?: ImageSource['open']): ImageSource => ({
  url: (src) => map[src],
  missing: 'Image missing',
  open,
});

const html = (text: string, images?: ImageSource, assets = true) =>
  render(Blocks, { props: { blocks: parseMarkdown(text, { assets }), images } }).body;

describe('safeAssetPath', () => {
  it('takes a file below assets/ of an allowed kind', () => {
    expect(safeAssetPath('assets/images/lua.svg')).toBe('assets/images/lua.svg');
    expect(safeAssetPath('assets/a.PNG')).toBe('assets/a.PNG');
    expect(safeAssetPath('./assets/a.webp')).toBe('assets/a.webp');
    expect(safeAssetPath('assets/a%20b.jpg')).toBe('assets/a b.jpg');
    for (const kind of ['png', 'jpg', 'jpeg', 'webp', 'gif', 'svg']) {
      expect(safeAssetPath(`assets/x.${kind}`)).toBe(`assets/x.${kind}`);
    }
  });

  it('refuses everything else', () => {
    for (const raw of [
      '',
      'https://tracker.example/pixel.png',
      '//tracker.example/assets/x.png',
      'data:image/svg+xml,<svg onload=alert(1)>',
      'data:image/png;base64,iVBORw0KGgo=',
      'javascript:alert(1)//assets/x.png',
      'JAVASCRIPT:alert(1)',
      'blob:https://example.org/1234',
      'file:///C:/x/assets/a.png',
      '/assets/a.png',
      'C:/assets/a.png',
      'C:\\assets\\a.png',
      'assets\\a.png',
      'assets/../secret.png',
      'assets/%2e%2e/secret.png',
      'assets/%2E%2E%2Fsecret.png',
      'assets/./a.png',
      'assets//a.png',
      'assets/',
      'assets',
      'other/a.png',
      'Assets/a.png',
      'images/assets/a.png',
      'assets/a.png?x=1',
      'assets/a.png#frag',
      'assets/a%00.png',
      'assets/a%zz.png',
      'assets/a.html',
      'assets/a.svgz',
      'assets/a.mp4',
      'assets/.png',
      'assets/noext',
      `assets/${'a'.repeat(600)}.png`,
    ]) {
      expect(safeAssetPath(raw), raw).toBeNull();
    }
  });
});

describe('parseMarkdown with assets', () => {
  it('keeps no address without the option (the other apps see no change)', () => {
    const blocks = parseMarkdown('![Diagram](assets/images/lua.svg)');
    expect(images(blocks)).toEqual([{ t: 'image', alt: 'Diagram' }]);
    expect(JSON.stringify(blocks)).not.toContain('assets/');
  });

  it('keeps the path of an asset with the option', () => {
    const blocks = parseMarkdown('![Diagram](assets/images/lua.svg)', { assets: true });
    expect(images(blocks)).toEqual([{ t: 'image', alt: 'Diagram', src: 'assets/images/lua.svg' }]);
  });

  it('never keeps a foreign address, data:, javascript:, .. or an absolute path', () => {
    const text = [
      '![a](https://tracker.example/pixel.png)',
      '![b](data:image/svg+xml,<svg onload=alert(1)>)',
      '![c](data:image/png;base64,iVBORw0KGgo=)',
      '![d](javascript:alert(1))',
      '![e](../assets/x.png)',
      '![f](assets/../x.png)',
      '![g](/etc/x.png)',
      '![h](C:/x/assets/a.png)',
      '![i][ref]',
      '',
      '[ref]: https://tracker.example/y.png',
    ].join('\n\n');
    const blocks = parseMarkdown(text, { assets: true });
    const found = images(blocks);
    // markdown-it itself refuses `data:image/svg+xml` and `javascript:` as an image (they stay text); the rest
    // are images without an address.
    expect(found.map((image) => image.alt)).toEqual(['a', 'c', 'e', 'f', 'g', 'h', 'i']);
    expect(found.every((image) => image.src === undefined)).toBe(true);
    expect(JSON.stringify(blocks)).not.toContain('"src"');
    expect(JSON.stringify(blocks)).not.toContain('tracker.example');
  });

  it('reads images in lists, quotes, tables, emphasis and links', () => {
    const text = '- [ ] ![a](assets/a.png)\n\n> ![b](assets/b.png)\n\n| x |\n| - |\n| ![c](assets/c.png) |\n\n*![d](assets/d.png)*\n\n[![e](assets/e.png)](https://example.org)';
    expect(images(parseMarkdown(text, { assets: true })).map((image) => image.src)).toEqual([
      'assets/a.png',
      'assets/b.png',
      'assets/c.png',
      'assets/d.png',
      'assets/e.png',
    ]);
  });
});

describe('drawableImageUrl', () => {
  it('lets only a blob: URL through', () => {
    expect(drawableImageUrl('blob:http://localhost/0c1d')).toBe('blob:http://localhost/0c1d');
    expect(drawableImageUrl(undefined)).toBeUndefined();
    expect(drawableImageUrl(null)).toBeNull();
    for (const url of ['https://tracker.example/x.png', 'data:image/png;base64,AA', 'javascript:alert(1)', 'assets/a.png', 'blob: x', '']) {
      expect(drawableImageUrl(url), url).toBeNull();
    }
  });
});

describe('Blocks with images', () => {
  const blob = 'blob:http://localhost/1f2e';

  it('draws an <img> with the address the app gives, and its description as alt', () => {
    const body = html('![Lua syntax](assets/lua.svg)', source({ 'assets/lua.svg': blob }));
    expect(body).toContain(`<img class="md-img`);
    expect(body).toContain(`src="${blob}"`);
    expect(body).toContain('alt="Lua syntax"');
    expect(body).not.toContain('<svg');
  });

  it('uses the file name as alt when the description is empty', () => {
    expect(html('![](assets/images/x.png)', source({ 'assets/images/x.png': blob }))).toContain('alt="x.png"');
  });

  it('stays the description without the option, or without the app source', () => {
    const map = source({ 'assets/lua.svg': blob });
    expect(html('![Lua](assets/lua.svg)', map, false)).not.toContain('<img');
    expect(html('![Lua](assets/lua.svg)', undefined, true)).not.toContain('<img');
    expect(html('![Lua](assets/lua.svg)', undefined, true)).toContain('Lua');
  });

  it('shows the description while it loads and the app word when the file is missing', () => {
    expect(html('![Lua](assets/lua.svg)', source({}))).not.toContain('<img');
    const missing = html('![Lua](assets/lua.svg)', source({ 'assets/lua.svg': null }));
    expect(missing).not.toContain('<img');
    expect(missing).toContain('Image missing: Lua');
  });

  it('never draws an address that is not a blob:, even when the app hands one over', () => {
    for (const url of ['https://tracker.example/x.png', 'data:image/svg+xml,<svg onload=alert(1)>', 'javascript:alert(1)']) {
      const body = html('![Lua](assets/lua.svg)', source({ 'assets/lua.svg': url }));
      expect(body, url).not.toContain('<img');
      expect(body, url).toContain('Image missing');
    }
  });

  it('makes the image a button to see it larger when the app can, but not inside a link', () => {
    const open = () => undefined;
    expect(html('![Lua](assets/lua.svg)', source({ 'assets/lua.svg': blob }, open))).toContain('<button type="button" class="img-open');
    expect(html('![Lua](assets/lua.svg)', source({ 'assets/lua.svg': blob }))).not.toContain('<button');
    const linked = html('[![Lua](assets/lua.svg)](https://example.org)', source({ 'assets/lua.svg': blob }, open));
    expect(linked).toContain('<img');
    expect(linked).not.toContain('<button');
  });
});
