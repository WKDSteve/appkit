// The types of folder.js: the folder a page works in, the folders used before, IndexedDB for their handles.

import type { InputDocument } from './folder-input.js';

export type PermissionState = 'granted' | 'prompt' | 'denied';
export type AccessMode = 'read' | 'readwrite';

/** The part of a `FileSystemFileHandle` a reader uses. */
export interface FileHandleLike {
  kind: 'file';
  name: string;
  getFile(): Promise<{ size: number; lastModified?: number; arrayBuffer(): Promise<ArrayBuffer> }>;
}

/** The part of a `FileSystemDirectoryHandle` a reader uses. */
export interface DirHandleLike {
  kind: 'directory';
  name: string;
  values(): AsyncIterable<FileHandleLike | DirHandleLike>;
}

/** The part of a `FileSystemWritableFileStream` a writer uses. */
export interface WritableStreamLike {
  write(data: Uint8Array): Promise<void>;
  close(): Promise<void>;
  abort?(reason?: unknown): Promise<void>;
}

/** A `FileSystemFileHandle`, as far as a writer uses one. */
export interface WritableFileHandle extends FileHandleLike {
  createWritable(options?: { keepExistingData?: boolean }): Promise<WritableStreamLike>;
  /** Chrome's `FileSystemHandle.move`: into another folder under a name. Not on every handle. */
  move?(destination: WritableDirHandle, name: string): Promise<void>;
}

/** A `FileSystemDirectoryHandle`, as far as a writer uses one. */
export interface WritableDirHandle extends DirHandleLike {
  getFileHandle(name: string, options?: { create?: boolean }): Promise<WritableFileHandle>;
  getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<WritableDirHandle>;
  removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
}

/** A `FileSystemDirectoryHandle`, as far as this module uses one. */
export interface DirHandle extends WritableDirHandle {
  queryPermission(descriptor: { mode: AccessMode }): Promise<PermissionState>;
  requestPermission(descriptor: { mode: AccessMode }): Promise<PermissionState>;
  isSameEntry?(other: DirHandle): Promise<boolean>;
}

/** What the module needs of `window`. */
export interface PickerWindow {
  showDirectoryPicker?: (options: { id: string; mode: AccessMode }) => Promise<DirHandle>;
  isSecureContext?: boolean;
  /** Where a folder input is made, for the browsers without the picker (`folder-input.js`). */
  document?: InputDocument;
}

export const MAX_RECENT: number;

export type Support = 'ok' | 'missing' | 'insecure';

export type PickResult =
  | { kind: 'picked'; handle: DirHandle }
  | { kind: 'cancelled' }
  | { kind: 'blocked'; message: string }
  | { kind: 'failed'; message: string };

export function pickerSupport(win: PickerWindow): Support;
export function oneLine(error: unknown, max?: number): string;
export function pickFolder(win: PickerWindow, options: { id: string; mode: AccessMode }): Promise<PickResult>;
/** How long a look that should be quick (the permission of a stored folder) waits for the browser, in ms. */
export const LOOK_WAIT_MS: number;
/** `promise`, or `fallback` once `ms` have passed without an answer; a rejection in time still rejects. */
export function within<T>(promise: Promise<T>, ms: number, fallback: T): Promise<T>;
/** The permission of a stored handle; `prompt` when the browser does not answer within `waitMs` (default LOOK_WAIT_MS). */
export function permissionOf(handle: DirHandle, mode: AccessMode, waitMs?: number): Promise<PermissionState | 'gone'>;
export function grantAccess(handle: DirHandle, mode: AccessMode): Promise<PermissionState | 'gone'>;
export function displayName(name: unknown): string;

/** The part of a key-value store the list needs. */
export interface KeyValue {
  get(key: string): Promise<unknown>;
  put(key: string, value: unknown): Promise<void>;
}

export interface Recent {
  id: string;
  name: string;
  handle: DirHandle;
  lastUsed: number;
}

export class RecentFolders {
  constructor(kv: KeyValue, opts?: { max?: number; newId?: () => string });
  list(): Promise<Recent[]>;
  remember(handle: DirHandle, now: number): Promise<Recent>;
  forget(id: string): Promise<void>;
  private readonly kv;
  private readonly max;
  private readonly newId;
  private save;
}

export function idbKeyValue(factory: IDBFactory, name: string, store?: string): KeyValue;
