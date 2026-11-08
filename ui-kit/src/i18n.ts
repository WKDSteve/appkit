// The part of a dictionary that knows no words: filling the holes of a text, choosing a language, looking a key up,
// and the few functions that every page needs around a dictionary (how long ago, a number, the word of a value). The
// words themselves, and the typed keys, belong to the app; the words of the kit's own components are `kitTexts`.

/** Put the values into the `{name}` holes of a text; a hole that has no value stays as it is (it shows the mistake). */
export function fill(text: string, params: Record<string, string | number> = {}): string {
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const value = params[name];
    return value === undefined ? whole : String(value);
  });
}

/** The first language the browser asks for that the app has (by its base tag: `de-AT` is `de`), else the fallback. */
export function pickLang<L extends string>(preferred: readonly string[], known: readonly L[], fallback: L): L {
  for (const tag of preferred) {
    const base = tag.toLowerCase().split('-')[0] ?? '';
    const match = known.find((lang) => lang === base);
    if (match !== undefined) return match;
  }
  return fallback;
}

/** The words of one language: key to text. The keys are typed by the app; the kit only looks them up. */
export type Dictionary = Readonly<Record<string, string>>;

/** The text of a key with its holes filled; a key that is not there shows as itself rather than breaking the page. */
export function lookup(dictionary: Dictionary, key: string, params: Record<string, string | number> = {}): string {
  return fill(Object.hasOwn(dictionary, key) ? (dictionary[key] as string) : key, params);
}

/** The keys of the "how long ago" texts, in the order of `ago`. */
export type AgoKey = 'kit.ago.seconds' | 'kit.ago.minutes' | 'kit.ago.hours' | 'kit.ago.days';

/** "5 min", "2 h": how long ago, in the unit that fits. `text` looks a kit key up in the app's dictionary. */
export function ago(text: (key: AgoKey, params: { n: number }) => string, milliseconds: number): string {
  const seconds = Math.max(0, Math.round(milliseconds / 1000));
  if (seconds < 90) return text('kit.ago.seconds', { n: seconds });
  const minutes = Math.round(seconds / 60);
  if (minutes < 90) return text('kit.ago.minutes', { n: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 48) return text('kit.ago.hours', { n: hours });
  return text('kit.ago.days', { n: Math.round(hours / 24) });
}

/** A number the way the language writes it, with two decimals at most. */
export function formatNumber(lang: string, value: number): string {
  return new Intl.NumberFormat(lang, { maximumFractionDigits: 2 }).format(value);
}

/**
 * The short explanation of a value or a label (`hint.state.freed`), or '' when there is none: the hint is an
 * addition, and a word this version does not know has no explanation yet.
 */
export function hintOf(dictionary: Dictionary, prefix: string, value: string | undefined): string {
  if (value === undefined) return '';
  const key = `${prefix}.${value}`;
  return Object.hasOwn(dictionary, key) ? lookup(dictionary, key) : '';
}

/**
 * The word for a value the engine named (`status.doing`, `state.ready`). A value this version does not know shows as
 * it is: the contract may grow a new word, and the page must not break on it.
 */
export function labelOf(dictionary: Dictionary, prefix: string, value: string | undefined): string {
  if (value === undefined) return '?';
  const key = `${prefix}.${value}`;
  return Object.hasOwn(dictionary, key) ? lookup(dictionary, key) : value;
}

/**
 * The words of the kit's own components, under the prefix `kit.`, for an app to spread into its dictionaries
 * (`{ ...kitTexts.en, ...appTexts }`). An app that words a text differently puts its own after.
 */
export const kitTexts = {
  en: {
    'kit.ago.seconds': '{n} s',
    'kit.ago.minutes': '{n} min',
    'kit.ago.hours': '{n} h',
    'kit.ago.days': '{n} d',
    'kit.undo': 'Undo',
    'kit.dismiss': 'Close',
    'kit.palette.title': 'Command palette',
    'kit.palette.label': 'Find an action',
    'kit.palette.placeholder': 'Type an action ...',
    'kit.palette.none': 'Nothing matches',
    'kit.palette.action': 'Action',
    'kit.docs.title': 'Documentation',
    'kit.docs.label': 'Search the documentation',
    'kit.docs.placeholder': 'Search ...',
    'kit.docs.none': 'Nothing found',
    'kit.docs.list': 'Documents',
    'kit.help.docs': 'Documentation',
    'kit.help.docs_search': 'Search the documentation ...',
    'kit.help.project': 'Project',
    'kit.help.repo': 'GitHub repository',
    'kit.help.license': 'License: {name}',
    'kit.help.hint.docs_search': 'Searches all the documents at once',
    'kit.help.hint.doc': 'Opens this document',
    'kit.help.hint.repo': 'Opens the source and the issues on GitHub in the browser',
    'kit.help.hint.license': 'Opens the license text in the browser',
    'kit.select.clear': 'Remove the selection {label}',
  },
  de: {
    'kit.ago.seconds': '{n} s',
    'kit.ago.minutes': '{n} min',
    'kit.ago.hours': '{n} Std',
    'kit.ago.days': '{n} Tg.',
    'kit.undo': 'Rückgängig',
    'kit.dismiss': 'Schließen',
    'kit.palette.title': 'Befehlspalette',
    'kit.palette.label': 'Aktion finden',
    'kit.palette.placeholder': 'Aktion eintippen ...',
    'kit.palette.none': 'Nichts passt',
    'kit.palette.action': 'Aktion',
    'kit.docs.title': 'Dokumentation',
    'kit.docs.label': 'In der Dokumentation suchen',
    'kit.docs.placeholder': 'Suchen ...',
    'kit.docs.none': 'Nichts gefunden',
    'kit.docs.list': 'Dokumente',
    'kit.help.docs': 'Dokumentation',
    'kit.help.docs_search': 'In der Dokumentation suchen ...',
    'kit.help.project': 'Projekt',
    'kit.help.repo': 'GitHub-Repository',
    'kit.help.license': 'Lizenz: {name}',
    'kit.help.hint.docs_search': 'Durchsucht alle Dokumente auf einmal',
    'kit.help.hint.doc': 'Öffnet dieses Dokument',
    'kit.help.hint.repo': 'Öffnet den Quelltext und die Issues auf GitHub im Browser',
    'kit.help.hint.license': 'Öffnet den Lizenztext im Browser',
    'kit.select.clear': 'Auswahl {label} entfernen',
  },
} as const satisfies Record<string, Dictionary>;

/** The key of a kit text. */
export type KitKey = keyof (typeof kitTexts)['en'];
