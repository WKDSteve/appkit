// The types of code-ref.js: the reader's check of a code-ref/1, a port of tools/check-code-ref.py.

export const MAX_BYTES: number;
export const MAX_EXCERPT_BYTES: number;
export const SCHEMA: string;
export const FENCE_LANG: string;
export const MAX_PATH: number;
export const MAX_LINE: number;

/** Does `text` hold one word that leaves the repository: an absolute path, a home, a `..` segment? */
export function leavesRepo(text: string): boolean;

/** Every reason to refuse `raw` (the bytes of the document, or its text); empty when the page may use the reference. Never throws. */
export function problems(raw: Uint8Array | string): string[];

/** The text of every closed fenced block of language `code-ref` in the body of a task. */
export function blocks(markdown: string): string[];

/** `[index, problems]` for every `code-ref` block of a task; empty problems means usable. */
export function taskProblems(markdown: string): Array<[number, string[]]>;
