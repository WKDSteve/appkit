import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

// There is no build: the apps bundle the source. This file is for the tests only.
export default defineConfig({
  plugins: [svelte()],
  // The tests that mount a component in a DOM (`*.dom.test.ts`, jsdom) need the browser build of Svelte.
  resolve: { conditions: ['browser'] },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    // A stylesheet is empty in a test unless it is named here: the check of the shell reads the tokens as text.
    css: { include: [/tokens\.css/] },
  },
});
