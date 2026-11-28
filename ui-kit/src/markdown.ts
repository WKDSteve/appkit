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

// The checks that need no parser live in `markdown-links.ts` (the root of the kit offers them from there); this module
// keeps offering them for the apps that import them from `appkit-ui/markdown`.
import { hostOf, MAX_SOURCE, safeAssetPath, safeHref } from './markdown-links';
export { ASSET_IMAGE_TYPES, hostOf, MAX_SOURCE, safeAssetPath, safeHref } from './markdown-links';

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

const md = new MarkdownIt({ html: false, linkify: false, typographer: false, breaks: false });

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

// Kept here for the apps that import it from this module; the function lives in `image-url.ts`, which does not load
// markdown-it.
export { drawableImageUrl } from './image-url';

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
