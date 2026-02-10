import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Frontend package root: crates/gestura-gui/frontend
const frontendRoot = path.resolve(__dirname, '..');

const cliPath = path.join(frontendRoot, 'node_modules', '@playwright', 'test', 'cli.js');

// Ensure Playwright config/tests that live outside the frontend package (e.g. repo-root tests/)
// can still resolve @playwright/test by adding the frontend's node_modules to NODE_PATH.
const env = {
  ...process.env,
  NODE_PATH: path.join(frontendRoot, 'node_modules'),
};

const args = process.argv.slice(2);
const child = spawn(process.execPath, [cliPath, ...args], {
  cwd: frontendRoot,
  env,
  stdio: 'inherit',
});

child.on('exit', (code, signal) => {
  if (typeof code === 'number') process.exit(code);
  if (signal) process.exit(1);
  process.exit(1);
});

