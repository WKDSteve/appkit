// Svelte's compiler settings for the UI kit's tests: preprocessing (TypeScript in components) through Vite.

import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
};
