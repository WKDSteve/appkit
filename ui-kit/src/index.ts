// The kit as one module. The Svelte components are imported by their file (`appkit-ui/MenuBar.svelte`), so that an
// app takes only what it uses; the styles are `appkit-ui/tokens.css` and `appkit-ui/base.css`.

export {
  Preferences,
  StreamSession,
  type Connection,
  type PreferenceKeys,
  type Profile,
  type StreamDeps,
  type Theme,
} from './session.svelte';
export {
  cacheableClass,
  indexedDbStore,
  type CacheStore,
  type DataClass,
  type IndexedDbOptions,
} from './cache';
export {
  ApiError,
  etagOf,
  failure,
  HubClient,
  PLAIN,
  textOf,
  type Hello,
  type SnapshotResult,
} from './client';
export {
  ago,
  fill,
  formatNumber,
  hintOf,
  kitTexts,
  labelOf,
  lookup,
  pickLang,
  type AgoKey,
  type Dictionary,
  type KitKey,
} from './i18n';
export {
  defaultKeymap,
  matches,
  resolveKey,
  withChord,
  type ChordState,
  type Command,
  type KeyPress,
  type KeyResult,
  type Keymap,
} from './keymap';
export {
  ariaShortcut,
  letterOfCode,
  menuForMnemonic,
  splitMnemonic,
  toBlocks,
  TYPE_AHEAD_MS,
  typeAheadIndex,
  type Menu,
  type MenuBlock,
  type MenuItem,
} from './menu';
export {
  CONFLICT_CODES,
  CONFLICTS_MAX,
  DEFAULT_CONFLICT_MODE,
  formatValue,
  mergeConflicts,
  parseConflictMode,
  type ConflictAction,
  type ConflictItem,
  type ConflictMode,
  type ConflictRow,
} from './conflicts';
export {
  ACTION_PREFIX,
  bumped,
  frecencyBonus,
  FRECENCY_MAX,
  matchScore,
  PALETTE_LIMIT,
  paletteItems,
  parseFrecency,
  stepIndex,
  wordScore,
  type Frecency,
  type PaletteAction,
  type PaletteEntry,
  type PaletteItem,
  type PaletteOptions,
} from './palette';
export type { ToastKind, ToastMessage } from './toast';
export { hostOf, MAX_SOURCE, parseMarkdown, safeHref, type Block, type Inline } from './markdown';
export {
  installNav,
  placeOf,
  type Nav,
  type NavAddress,
  type NavHistory,
  type NavOptions,
  type NavWindow,
} from './nav';
export { hasPairing, MAX_VALUE, readRoute, routeHash, routeHref, type Route } from './route';
export { ASK_MS, askForSession, shareSession, type AskOptions, type ChannelLike, type ShareOptions } from './handoff';
export { takePairingCode, type HistoryLike, type LocationLike } from './pair';
export {
  backoffSeconds,
  IDLE_MS,
  MAX_BUFFER,
  readStream,
  SseParser,
  StreamRefused,
  type SseMessage,
  type StreamHandlers,
} from './sse';
export {
  helpItems,
  isHttps,
  searchDocs,
  SNIPPET,
  type DocEntry,
  type DocHit,
  type HelpOptions,
} from './docs';
export {
  buildContextMenu,
  contextTarget,
  ContextRegistry,
  isEditable,
  placeMenu,
  targetChain,
  type ContextEntry,
  type ContextEnv,
  type ContextItem,
  type ContextProvider,
  type ContextTarget,
} from './contextmenu';
export { closeOnBackdrop, isOutside, type BackdropOptions, type Rect } from './backdrop';
export { hasHidden, markHidden } from './text';
export {
  HIDE_DELAY_MS,
  hintAttributes,
  hintDelay,
  installTooltips,
  MAX_DELAY_MS,
  placeTip,
  SHOW_DELAY_MS,
  TIP_ID,
  tooltip,
  withoutToken,
  withToken,
  type Box,
  type TooltipOptions,
  type TooltipParams,
} from './tooltip';
export {
  ipcFetch,
  isWay,
  KEPT_AT,
  LinkError,
  nextStep,
  WAY_ABILITIES,
  type Bridge,
  type BridgeRequest,
  type BridgeResponse,
  type Connection as MobileConnection,
  type LinkFailure,
  type Way,
  type WayAbilities,
} from './mobile';
export {
  WriteQueue,
  type FlushReport,
  type Pending,
  type PendingState,
  type QueueDeps,
  type QueueStore,
  type SendOutcome,
} from './queue';
