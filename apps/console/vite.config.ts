import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';
import { paraglideVitePlugin } from '@inlang/paraglide-js';
import { realpathSync } from 'node:fs';
import { dirname } from 'node:path';
import { createRequire } from 'node:module';

// Bun links packages from its global cache; Vite's dev fs guard blocks those
// real paths, so fontsource .woff2 files 403 in dev. Allow just those dirs.
const require = createRequire(import.meta.url);
const fontDirs = ['geist', 'geist-mono', 'doto'].map((f) =>
  dirname(realpathSync(require.resolve(`@fontsource-variable/${f}/package.json`))),
);

export default defineConfig({
  plugins: [
    tailwindcss(),
    sveltekit(),
    paraglideVitePlugin({
      project: './project.inlang',
      outdir: './src/lib/paraglide',
      strategy: ['cookie', 'baseLocale'],
    }),
  ],
  server: {
    fs: { allow: ['.', ...fontDirs] },
    // Opt-in: `VKDG_ADMIN_PROXY=https://host VITE_ADMIN_URL= bun run dev`
    // serves /admin/* from a running gateway (same-origin, so cookies work).
    proxy: process.env.VKDG_ADMIN_PROXY
      ? { '/admin': { target: process.env.VKDG_ADMIN_PROXY, changeOrigin: true, cookieDomainRewrite: 'localhost' } }
      : undefined,
  },
});
