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
        "**/gestura-gui/src/**",
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
    // Output directory relative to frontend folder
    // Tauri will read from this location
    outDir: 'dist',
    target: process.env.TAURI_ENV_PLATFORM == 'windows' ? 'chrome105' : 'safari13',
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
