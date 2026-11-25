// The check behind the `translate` test of the kit and of every app: commands, identifiers, paths and shortcuts are not
// for the translation of the browser (translated, `app open` stops being a command and a task id stops being an id).
// Every `<code>`, `<kbd>` and `<pre>`, and every element that shows an id (`class="id"`), says `translate="no"` (or
// `translate="yes"` where the text inside is prose, like the body of a task); the shortcut of a menu says `no`. A
// `<code>` straight inside a `<pre translate="no">` is covered by that `<pre>`.

/** The tags of one component that should carry a `translate` attribute and do not. */
export function unprotected(source: string): string[] {
  const bad: string[] = [];
  for (const m of source.matchAll(/<(code|pre|kbd)\b[^>]*>/g)) {
    const tag = m[0];
    if (/translate="(no|yes)"/.test(tag)) continue;
    const before = source.slice(Math.max(0, (m.index ?? 0) - 80), m.index);
    if (tag.startsWith('<code') && /<pre\b[^>]*translate="no"[^>]*>\s*$/.test(before)) continue;
    bad.push(tag);
  }
  for (const m of source.matchAll(/<[a-z]+\b[^>]*class="id"[^>]*>/g)) {
    if (!/translate="(no|yes)"/.test(m[0])) bad.push(m[0]);
  }
  for (const m of source.matchAll(/<span class="(?:cm|mb)-shortcut"[^>]*>/g)) {
    if (!/translate="no"/.test(m[0])) bad.push(m[0]);
  }
  return bad;
}
