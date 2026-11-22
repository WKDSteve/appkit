// The example page of the menu bar with submenus, built and checked by `../menubar.browser.cjs`: the menu View with
// the submenu of the learning plans (as tasks-desktop will have it), and the last menu with a submenu that has no
// room on the right.

import '../../src/tokens.css';
import '../../src/base.css';
import './example.css';
import { mount } from 'svelte';
import MenuBar from '../../src/MenuBar.svelte';
import type { Menu, MenuItem } from '../../src/menu';

const picked: string[] = [];
(window as unknown as { picked: string[] }).picked = picked;

const radio = (label: string, checked: boolean, href?: string): MenuItem => ({
  kind: 'radio',
  label,
  checked,
  href,
  onselect: () => picked.push(label),
  onopennew: href ? () => picked.push(`new:${label}`) : undefined,
});
const action = (label: string, shortcut?: string): MenuItem => ({
  kind: 'action',
  label,
  shortcut,
  onselect: () => picked.push(label),
});

const check = (label: string, checked: boolean): MenuItem => ({
  kind: 'check',
  label,
  checked,
  onselect: () => picked.push(label),
});

const menus: Menu[] = [
  { id: 'tasks', label: 'Aufgaben', mnemonic: 'a', items: [action('Neu', 'Strg+N'), action('Suchen', 'Strg+F')] },
  {
    id: 'view',
    label: 'Ansicht',
    mnemonic: 'n',
    items: [
      { kind: 'section', label: 'Ansichten' },
      radio('Heute', false, '#view=today'),
      radio('Liste', false, '#view=list'),
      radio('Board', false, '#view=board'),
      radio('Repos', false, '#view=repos'),
      {
        kind: 'submenu',
        label: 'Lernpläne',
        hint: 'Kompetenzkarte und Lernpfade',
        items: [radio('Kompetenzkarte', true, '#view=pathmap'), radio('Lernpfade', false, '#view=paths')],
      },
      { kind: 'section', label: 'Export' },
      action('Als Markdown …'),
    ],
  },
  {
    // Taller than a low window: the popup scrolls (`longMenu` in `../menubar.browser.cjs`).
    id: 'long',
    label: 'Lang',
    mnemonic: 'l',
    items: Array.from({ length: 30 }, (_, i) => check(`Eintrag ${i + 1}`, i % 2 === 0)),
  },
  {
    id: 'options',
    label: 'Optionen',
    mnemonic: 'o',
    items: [
      action('Einstellungen …'),
      {
        kind: 'submenu',
        label: 'Sprache',
        items: [radio('Deutsch', true), radio('English', false), radio('Français', false)],
      },
    ],
  },
];

mount(MenuBar, {
  target: document.getElementById('bar')!,
  props: { menus, label: 'Menü', labels: { submenuCurrent: '{label}, aktuell: {current}' } },
});
