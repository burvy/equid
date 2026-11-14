import { defineConfig } from 'vite';

// The trip agent (cargo run -p server) on its default port; the page calls /agent on its own origin.
// Point elsewhere with EQUID_AGENT=http://host:port npm run dev, or ?agent=<full URL> in the page.
const agent = { '/agent': { target: process.env.EQUID_AGENT ?? 'http://127.0.0.1:8787', changeOrigin: true } };

export default defineConfig({
  // data files live in ../data (shared with the engine tests); let the dev server read them
  server: { fs: { allow: ['..'] }, proxy: agent },
  preview: { proxy: agent },
  build: { target: 'es2022' },
  worker: { format: 'es' },
});
