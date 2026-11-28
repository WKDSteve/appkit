// The checks of the markdown reader that need no parser: which link and which image address a text may keep. They live
// apart from `markdown.ts`, which loads markdown-it (about 100 kB), so that the root of the kit can offer them without
// pulling the parser into every bundle (`rootImport.bundle.test.ts`). `parseMarkdown` itself is `appkit-ui/markdown`.

/** Longer than this is not read as Markdown: it is shown as it is. */
export const MAX_SOURCE = 256 * 1024;

/** The kinds of image file an asset may be: what an `<img>` draws and nothing that runs (E12: SVG only as `<img>`). */
export const ASSET_IMAGE_TYPES: readonly string[] = ['png', 'jpg', 'jpeg', 'webp', 'gif', 'svg'];
const ASSET_MAX = 512;

/**
 * The path below `assets/` an image address names, or null. Relative only: no scheme, no leading slash, no `.` or
 * `..` segment, no backslash, no query or fragment, no control character, and the file kind is on the allow-list.
 * markdown-it hands the address percent-encoded; it is decoded once and checked as decoded, so `%2e%2e` is `..`.
 */
export function safeAssetPath(raw: string): string | null {
  let text = raw.trim();
  if (text === '' || text.length > ASSET_MAX) return null;
  try {
    text = decodeURIComponent(text);
  } catch {
    return null;
  }
  if (text.startsWith('./')) text = text.slice(2);
  if (/[\\:?#%\u0000-\u001f\u007f]/.test(text)) return null;
  const parts = text.split('/');
  if (parts[0] !== 'assets' || parts.length < 2) return null;
  if (parts.some((part) => part === '' || part === '.' || part === '..')) return null;
  const name = parts[parts.length - 1] ?? '';
  const dot = name.lastIndexOf('.');
  if (dot <= 0) return null;
  return ASSET_IMAGE_TYPES.includes(name.slice(dot + 1).toLowerCase()) ? text : null;
}

// Only these ever become a link. markdown-it already refuses `javascript:`
// and the like; this is the second wall, and it is an allow-list.
const LINK_PROTOCOLS = new Set(['http:', 'https:', 'mailto:']);

/** The host a link goes to, for the person who reads the link text. */
export function hostOf(href: string): string | null {
  try {
    const url = new URL(href);
    return url.protocol === 'mailto:' ? url.pathname.split('@').pop() || null : url.host || null;
  } catch {
    return null;
  }
}

export function safeHref(raw: string): string | null {
  const text = raw.trim();
  if (text === '' || text.length > 2048) return null;
  try {
    const url = new URL(text);
    return LINK_PROTOCOLS.has(url.protocol) ? url.href : null;
  } catch {
    // Relative, or not a URL at all: shown as text, not made a link.
    return null;
  }
}
