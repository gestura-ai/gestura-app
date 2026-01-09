import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: [
        "**/src-tauri/**",
        "**/target/**",
        "**/dist/**",
        "**/scripts/**",
        "**/docs/**",
        "**/*.md",
        "**/*.log",
        "**/*.tmp",
        "**/node_modules/**",
        "**/.git/**",
      ],
    },
  },
  build: {
    // Use a dedicated frontend output directory so that macOS packaging
    // artifacts (dist/macos*) never conflict with Vite's cleanup logic.
    outDir: 'dist/frontend',
    target: process.env.TAURI_ENV_PLATFORM == 'windows' ? 'chrome105' : 'safari13',
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
