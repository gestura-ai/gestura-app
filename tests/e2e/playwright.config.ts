import { defineConfig, devices } from '@playwright/test';
import path from 'node:path';

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: 'html',
  use: {
    baseURL: 'http://localhost:1420',
    trace: 'on-first-retry',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'firefox',
      use: { ...devices['Desktop Firefox'] },
    },
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
    },
  ],

  webServer: {
    // NOTE: Playwright runs in a regular browser context.
    // `cargo tauri dev` starts the native app, but the served page at :1420
    // does not have the Tauri IPC bridge when opened by Playwright.
    // We therefore run the Vite dev server and mock IPC in the tests.
    // Use `--force` so Vite re-optimizes deps on each Playwright run.
    // This avoids flaky 504 "Outdated Optimize Dep" responses after dependency changes.
    command: 'npm run dev -- --force',
    // This repo's only Node project lives under crates/gestura-gui/frontend.
    // Without cwd, Playwright would try to run this from tests/e2e/ and fail.
    cwd: path.resolve(__dirname, '../../crates/gestura-gui/frontend'),
    url: 'http://localhost:1420',
    // Always start a fresh dev server for Playwright.
    // Reusing an existing server can keep a stale optimize-deps cache and cause 504s.
    reuseExistingServer: false,
    timeout: 120 * 1000,
  },
});
