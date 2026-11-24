// The types of e2e-node.js.

// Written without `@types/node`, so that the kit checks without it: `ChildLike` is the part of a `ChildProcess` used.

/** The part of a Node `ChildProcess` that `stopChild` uses; a real one is one. */
export interface ChildLike {
  exitCode: number | null;
  signalCode: string | null;
  once(event: 'exit', listener: () => void): unknown;
  kill(): unknown;
}

export interface RemoveOptions {
  tries?: number;
  pauseMs?: number;
  /** The removal of one try; `fs.rmSync` by default (a test hands in one that fails). */
  rm?: (path: string) => void;
}

export function removeTree(path: string, options?: RemoveOptions): Promise<void>;
export function stopChild(child: ChildLike, timeoutMs?: number): Promise<void>;
export function freePort(): Promise<number>;

/** What differs between the apps: the binary of the hub, the environment it runs with, and the origin it listens on. */
export interface HubTarget {
  exe: string;
  env: (config: string) => Record<string, string | undefined>;
  /** `http://127.0.0.1:<port>`, without a trailing slash. */
  origin: string;
}

export function pairingLink(config: string, hub: HubTarget): string;
export function sessionToken(config: string, hub: HubTarget): Promise<string>;
