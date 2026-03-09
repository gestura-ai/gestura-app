import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'path';

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
    // Raise the per-chunk warning threshold to 700 kB.
    // The vendor-codemirror chunk (~675 kB) is CodeMirror 6 + 7 language packs — it
    // is already lazy-loaded on demand (only when the user opens the editor view)
    // and cached separately from the app code, so the default 500 kB threshold
    // produces a spurious warning rather than a real performance problem.
    chunkSizeWarningLimit: 700,
    rollupOptions: {
      input: {
        // Main settings/voice window
        main: resolve(__dirname, 'index.html'),
        // Agent window v2 (React/Vite). window_manager.rs now points here.
        // public/agent.html is dead code — delete it once v2 is confirmed stable.
        agent_v2: resolve(__dirname, 'agent_v2.html'),
      },
      output: {
        manualChunks(id) {
          // CodeMirror + Lezer parser runtime — the heaviest vendor group (~400 kB)
          if (id.includes('node_modules/@codemirror') || id.includes('node_modules/@lezer')) {
            return 'vendor-codemirror';
          }
          // React runtime — shared across both entry points
          if (
            id.includes('node_modules/react/') ||
            id.includes('node_modules/react-dom/') ||
            id.includes('node_modules/scheduler/')
          ) {
            return 'vendor-react';
          }
          // Tauri JS API
          if (id.includes('node_modules/@tauri-apps/')) {
            return 'vendor-tauri';
          }
        },
      },
    },
  },
});
