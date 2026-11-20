// Markdown of an untrusted text, read into a small tree and drawn by components.
//
// The body is untrusted. It never becomes an HTML string: markdown-it (with
// `html: false`) gives tokens, the tokens become this tree, and Svelte makes
// elements from the tree. There is no sink an attacker's text could reach,
// so there is nothing for a sanitizer to miss (and Trusted Types can stay
// strict: the page creates no HTML of its own).

import MarkdownIt from 'markdown-it';

export type Inline =
  | { t: 'text'; text: string }
  | { t: 'code'; text: string }
  | { t: 'br' }
  | { t: 'em' | 'strong' | 'del'; children: Inline[] }
  | { t: 'link'; href: string | null; host: string | null; children: Inline[] }
  // `src` is there only when the reader was asked for the images of a container (`assets`) and the address is a
  // file below its `assets/` folder; it is a path, never a URL: the app turns it into the address it draws.
  | { t: 'image'; alt: string; src?: string };

/** How a text is read. */
export interface ParseOptions {
  /**
   * Keep the address of an image that names a file below `assets/` of the text's own folder (`assets/x.svg`), so
   * the page can draw it from the app. Every other address (a URL, `data:`, an absolute path, `..`) stays what it
   * is without this option: the description, as text.
   */
  assets?: boolean;
}

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

export type Block =
  | { t: 'p'; children: Inline[] }
  | { t: 'h'; level: number; children: Inline[] }
  | { t: 'list'; ordered: boolean; start: number; items: Block[][] }
  | { t: 'quote'; children: Block[] }
  | { t: 'pre'; text: string; lang: string }
  | { t: 'hr' }
  | { t: 'table'; head: Inline[][]; rows: Inline[][][] };

/** Deeper than this is drawn flat: a body that nests a thousand quotes. */
const MAX_DEPTH = 12;
/** Longer than this is not read as Markdown: it is shown as it is. */
export const MAX_SOURCE = 256 * 1024;

const md = new MarkdownIt({ html: false, linkify: false, typographer: false, breaks: false });

// Only these ever become a link. markdown-it already refuses `javascript:`
// and the like; this is the second wall, and it is an allow-list.
const LINK_PROTOCOLS = new Set(['http:', 'https:', 'mailto:']);

/**
 * The app's way to the images of a text read with `assets`. `url` gives the address to draw for a path below
 * `assets/`: a `blob:` URL the app made from the file (it fetched it itself, through its own checked route),
 * `undefined` while it still loads (the description shows), `null` when there is no such file. `missing` is the
 * app's word for that ("Image missing"): the kit has no words. With `open` the image is a button that shows it
 * larger.
 */
export interface ImageSource {
  url(src: string): string | null | undefined;
  missing: string;
  open?(src: string, url: string, alt: string): void;
}

/**
 * The address an `<img>` of an asset may carry: only a `blob:` URL of this page. Anything else the app hands over
 * by mistake (a URL of another host, `data:`, `javascript:`) counts as a missing file and is never drawn.
 */
export function drawableImageUrl(url: string | null | undefined): string | null | undefined {
  if (url === undefined || url === null) return url;
  return /^blob:[^\s]+$/i.test(url) && url.length <= 2048 ? url : null;
}

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

interface Tok {
  type: string;
  tag: string;
  content: string;
  info: string;
  children: Tok[] | null;
  attrs: [string, string][] | null;
  nesting: number;
}

/** Read Markdown into blocks. Text over the limit is one preformatted block. */
export function parseMarkdown(source: string, options: ParseOptions = {}): Block[] {
  if (source.length > MAX_SOURCE) return [{ t: 'pre', text: source, lang: '' }];
  const tokens = md.parse(source, {}) as unknown as Tok[];
  const cursor = { i: 0 };
  return readBlocks(tokens, cursor, 0, null, options.assets === true);
}

function readBlocks(tokens: Tok[], c: { i: number }, depth: number, close: string | null, assets: boolean): Block[] {
  const out: Block[] = [];
  while (c.i < tokens.length) {
    const tok = tokens[c.i];
    if (!tok) break;
    if (close !== null && tok.type === close) {
      c.i += 1;
      return out;
    }
    c.i += 1;
    switch (tok.type) {
      case 'paragraph_open': {
        const inline = tokens[c.i];
        out.push({ t: 'p', children: inlineOf(inline, assets) });
        c.i += 2; // inline, paragraph_close
        break;
      }
      case 'heading_open': {
        const inline = tokens[c.i];
        const level = Math.min(6, Math.max(1, Number(tok.tag.slice(1)) || 1));
        out.push({ t: 'h', level, children: inlineOf(inline, assets) });
        c.i += 2;
        break;
      }
      case 'bullet_list_open':
      case 'ordered_list_open': {
        const ordered = tok.type === 'ordered_list_open';
        const start = Number(tok.attrs?.find(([k]) => k === 'start')?.[1] ?? 1) || 1;
        const items: Block[][] = [];
        const closeList = ordered ? 'ordered_list_close' : 'bullet_list_close';
        while (c.i < tokens.length && tokens[c.i]?.type !== closeList) {
          if (tokens[c.i]?.type === 'list_item_open') {
            c.i += 1;
            items.push(deeper(tokens, c, depth, 'list_item_close', assets));
          } else {
            c.i += 1;
          }
        }
        c.i += 1;
        out.push({ t: 'list', ordered, start, items });
        break;
      }
      case 'blockquote_open':
        out.push({ t: 'quote', children: deeper(tokens, c, depth, 'blockquote_close', assets) });
        break;
      case 'fence':
      case 'code_block':
        out.push({ t: 'pre', text: tok.content, lang: tok.info.trim().split(/\s+/)[0] ?? '' });
        break;
      case 'hr':
        out.push({ t: 'hr' });
        break;
      case 'table_open':
        out.push(readTable(tokens, c, assets));
        break;
      default:
        // A token this reader does not know draws nothing.
        break;
    }
  }
  return out;
}

/** The children of a container; past the depth limit they are skipped whole. */
function deeper(tokens: Tok[], c: { i: number }, depth: number, close: string, assets: boolean): Block[] {
  if (depth >= MAX_DEPTH) {
    // Skip to the matching close without drawing: every token says whether
    // it opens (1), closes (-1) or stands alone (0).
    let level = 1;
    while (c.i < tokens.length && level > 0) {
      level += tokens[c.i]?.nesting ?? 0;
      c.i += 1;
    }
    return [];
  }
  return readBlocks(tokens, c, depth + 1, close, assets);
}

function readTable(tokens: Tok[], c: { i: number }, assets: boolean): Block {
  const head: Inline[][] = [];
  const rows: Inline[][][] = [];
  let row: Inline[][] | null = null;
  let inHead = false;
  while (c.i < tokens.length) {
    const tok = tokens[c.i];
    c.i += 1;
    if (!tok) break;
    if (tok.type === 'table_close') break;
    if (tok.type === 'thead_open') inHead = true;
    else if (tok.type === 'thead_close') inHead = false;
    else if (tok.type === 'tr_open') row = [];
    else if (tok.type === 'tr_close') {
      if (row) {
        if (inHead) head.push(...row);
        else rows.push(row);
      }
      row = null;
    } else if (tok.type === 'th_open' || tok.type === 'td_open') {
      row?.push(inlineOf(tokens[c.i], assets));
      c.i += 2; // inline, close
    }
  }
  return { t: 'table', head, rows };
}

function inlineOf(token: Tok | undefined, assets: boolean): Inline[] {
  if (!token || token.type !== 'inline') return [];
  return readInline(token.children ?? [], { i: 0 }, null, assets);
}

function readInline(tokens: Tok[], c: { i: number }, close: string | null, assets: boolean): Inline[] {
  const out: Inline[] = [];
  while (c.i < tokens.length) {
    const tok = tokens[c.i];
    if (!tok) break;
    c.i += 1;
    if (close !== null && tok.type === close) return out;
    switch (tok.type) {
      case 'text':
      case 'text_special':
        out.push({ t: 'text', text: tok.content });
        break;
      case 'softbreak':
        out.push({ t: 'text', text: ' ' });
        break;
      case 'hardbreak':
        out.push({ t: 'br' });
        break;
      case 'code_inline':
        out.push({ t: 'code', text: tok.content });
        break;
      case 'em_open':
        out.push({ t: 'em', children: readInline(tokens, c, 'em_close', assets) });
        break;
      case 'strong_open':
        out.push({ t: 'strong', children: readInline(tokens, c, 'strong_close', assets) });
        break;
      case 's_open':
        out.push({ t: 'del', children: readInline(tokens, c, 's_close', assets) });
        break;
      case 'link_open': {
        const href = tok.attrs?.find(([k]) => k === 'href')?.[1] ?? '';
        const safe = safeHref(href);
        out.push({
          t: 'link',
          href: safe,
          host: safe === null ? null : hostOf(safe),
          children: readInline(tokens, c, 'link_close', assets),
        });
        break;
      }
      case 'image': {
        // Without `assets` no image is fetched: it is its description, as text. With it, only a file below
        // `assets/` keeps its path; the page asks the app for it and never fetches the address itself.
        const src = assets ? safeAssetPath(tok.attrs?.find(([k]) => k === 'src')?.[1] ?? '') : null;
        out.push(src === null ? { t: 'image', alt: tok.content } : { t: 'image', alt: tok.content, src });
        break;
      }
      default:
        break;
    }
  }
  return out;
}
