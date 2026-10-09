import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { thirdPartyNotices } from './third-party-notices.ts';

const stylesheet = fileURLToPath(new URL('./src/ui/app.css', import.meta.url));
const lib = fileURLToPath(new URL('./src/ui/lib', import.meta.url));
const cacheDir = fileURLToPath(new URL('./node_modules/.vite', import.meta.url));

// The OFL-1.1 requires the licence text to travel with the bundled Inter font.
const interLicence = fileURLToPath(
  new URL('./node_modules/@fontsource-variable/inter/LICENSE', import.meta.url),
);
const fontLicence = {
  name: 'calliope-font-licence',
  generateBundle(this: { emitFile(f: { type: 'asset'; fileName: string; source: string }): void }) {
    this.emitFile({
      type: 'asset',
      fileName: 'licenses/Inter-OFL-1.1.txt',
      source: readFileSync(interLicence, 'utf8'),
    });
  },
};

export default defineConfig({
  root: 'src/ui',
  cacheDir,
  plugins: [
    tailwindcss(),
    svelte(),
    fontLicence,
    thirdPartyNotices({ crate: 'calliope-gui', stylesheet }),
  ],
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
    setupFiles: ['./test-setup.ts'],
    passWithNoTests: true,
  },
});
