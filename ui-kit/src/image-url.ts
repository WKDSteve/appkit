// The check of the address an `<img>` of an asset may carry, in a module of its own: a page that only draws images
// (it reads the text with its own, later loaded parser) does not pull in markdown-it for it. `markdown.ts` exports
// the same function, so the old import stays valid.

/**
 * The address an `<img>` of an asset may carry: only a `blob:` URL of this page. Anything else the app hands over
 * by mistake (a URL of another host, `data:`, `javascript:`) counts as a missing file and is never drawn.
 */
export function drawableImageUrl(url: string | null | undefined): string | null | undefined {
  if (url === undefined || url === null) return url;
  return /^blob:[^\s]+$/i.test(url) && url.length <= 2048 ? url : null;
}
