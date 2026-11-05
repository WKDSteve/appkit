// The tests read the source of a component or a stylesheet as text (`?raw`, a feature of Vite).
declare module '*.svelte?raw' {
  const source: string;
  export default source;
}

// The check of the shell reads the tokens as text (`vite.config.ts` lets the stylesheet through in the tests).
declare module '*.css?raw' {
  const source: string;
  export default source;
}
