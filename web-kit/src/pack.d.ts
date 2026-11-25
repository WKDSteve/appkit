// The types of pack.js (plain JavaScript, so that Node runs it from node_modules).

/** The policy of a page that runs an engine: inline scripts, WASM from embedded bytes, a worker from a blob. */
export const CSP_WASM_WORKER: string;
/** The policy of a page without WASM and without a worker (no `'wasm-unsafe-eval'`, no `worker-src`). */
export const CSP_STRICT: string;

/** Bytes, as Node and the browser give them. */
export type Bytes = Uint8Array;

export interface PageInput {
  css: string;
  js: string;
  /** The parts the page carries, in the order they are written; none by default. Plain lowercase words, not `app`. */
  partIds?: readonly string[];
  /** Each id of `partIds` to its bytes; an id that is not named there is refused. */
  parts?: Readonly<Record<string, Bytes>>;
  /**
   * The Content-Security-Policy of the page: `default-src`, `connect-src`, `base-uri` and `form-action` 'none' alone,
   * no other source than `'unsafe-inline'`, `'wasm-unsafe-eval'`, `data:` and `blob:` (see `policyProblems`). Trusted Types may be
   * required: `require-trusted-types-for 'script'` and `trusted-types <policy names>` (or `'none'`), never `*`.
   */
  csp: string;
  /** The title of the page (plain text). */
  title: string;
  /** The language of the text of the page, a language tag such as `de` or `pt-BR`; `en` by default. */
  lang?: string;
  /** The comment in the head: where the file comes from, its build (plain text). */
  comment: string;
}

/** What makes a policy one that lets the page load or reach something (empty when it is fine); `assemble` throws on it. */
export function policyProblems(csp: string): string[];
export function checkPartIds(partIds: readonly string[]): readonly string[];
export function packPart(data: Bytes): string;
export function inlineScript(js: string): string;
export function inlineStyle(css: string): string;
export function assemble(input: PageInput): string;
export function verify(html: string, options?: { allowUrls?: readonly string[] }): string[];
export function verifyShell(html: string, options?: { partIds?: readonly string[] }): string[];
export function readDist(dist: string, index?: string): { js: string; css: string };
export function writePage(out: string, name: string, html: string): string;
