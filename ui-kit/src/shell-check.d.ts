/** What is read of an element. */
export declare const SHELL_PROPS: readonly [
  'color',
  'backgroundColor',
  'fontFamily',
  'fontSize',
  'fontWeight',
  'lineHeight',
  'borderTopColor',
  'borderTopStyle',
  'borderTopWidth',
  'borderRadius',
  'boxShadow',
  'paddingTop',
  'paddingLeft',
  'outlineStyle',
];
export type ShellProp = (typeof SHELL_PROPS)[number];
export type ShellStyle = Record<ShellProp, string>;
/** Probe name to what the browser works out for the first element that matches it. */
export type ShellSignature = Record<string, ShellStyle>;
/** The parts every app has, by the selector that finds each. The ones of an open menu or palette exist only then. */
export declare const SHELL_PROBES: Readonly<Record<string, string>>;
/** Runs in the page: the styles of the first element of each probe that is there now. */
export declare function collectShell(probes: Record<string, string>): Record<string, Record<string, string>>;
/** Per colour of a token of the kit (as the browser writes it, `rgb(r, g, b)`), the colours a palette gives that token. */
export type Recolor = Record<string, string[]>;
/**
 * What is different: one line per probe and property, and one for a probe that the page does not have. `recolor` (the
 * one theme of `paletteRecolor`) lets a colour of the reference be met by what the palette gives the same token.
 */
export declare function diffShell(label: string, actual: ShellSignature, golden: ShellSignature, recolor?: Recolor): string[];
/** Per theme, each colour of a token of the kit to the colours the palette gives the same token (texts of the CSS files). */
export declare function paletteRecolor(tokensCss: string, paletteCss: string): { light: Recolor; dark: Recolor };
