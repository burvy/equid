import { defineConfig } from 'vite';

export default defineConfig({
  // data files live in ../data (shared with the engine tests); let the dev server read them
  server: { fs: { allow: ['..'] } },
  build: { target: 'es2022' },
});
