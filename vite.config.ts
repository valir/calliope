import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

const lib = fileURLToPath(new URL('./src/ui/lib', import.meta.url));
const cacheDir = fileURLToPath(new URL('./node_modules/.vite', import.meta.url));

export default defineConfig({
  root: 'src/ui',
  cacheDir,
  plugins: [tailwindcss(), svelte()],
  resolve: {
    alias: { $lib: lib },
    ...(process.env.VITEST ? { conditions: ['browser'] } : {}),
  },
  server: { port: 5173, strictPort: true, host: 'localhost' },
  build: {
    outDir: '../../dist',
    emptyOutDir: true,
    assetsInlineLimit: 0,
    modulePreload: { polyfill: false },
  },
  test: {
    environment: 'jsdom',
    include: ['**/*.test.ts'],
    passWithNoTests: true,
  },
});
