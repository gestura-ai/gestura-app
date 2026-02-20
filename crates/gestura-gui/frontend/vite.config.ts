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
    rollupOptions: {
      input: {
        // Main settings/voice window
        main: resolve(__dirname, 'index.html'),
        // Agent window v2 (React/Vite). window_manager.rs now points here.
        // public/agent.html is dead code — delete it once v2 is confirmed stable.
        agent_v2: resolve(__dirname, 'agent_v2.html'),
      },
    },
  },
});
