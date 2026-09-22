import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  test: {
    environment: 'node',
    include: [
      'src/**/*.test.ts',
      'src-tauri/src/bridge/assets/**/*.test.js',
      'src-tauri/src/bridge/assets/**/*.test.ts',
    ],
  },
});
