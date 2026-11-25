/// <reference types="vite/client" />
import { describe, expect, it } from 'vitest';
import { drawableImageUrl } from './image-url';
import source from './image-url.ts?raw';
import { drawableImageUrl as fromMarkdown } from './markdown';

describe('image-url', () => {
  it('is the same function the old import gives', () => {
    expect(fromMarkdown).toBe(drawableImageUrl);
  });

  it('lets only a blob: URL through', () => {
    expect(drawableImageUrl('blob:http://localhost/0c1d')).toBe('blob:http://localhost/0c1d');
    expect(drawableImageUrl('https://x.example/a.png')).toBeNull();
  });

  it('imports nothing, so that markdown-it stays out of a page that only draws images', () => {
    expect(source).toContain('drawableImageUrl');
    expect(source).not.toMatch(/^\s*import\s/m);
  });
});
