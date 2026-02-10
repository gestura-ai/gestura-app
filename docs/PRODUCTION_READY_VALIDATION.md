# Production-ready validation (Gestura.app)

## Why this exists

CI is strict (formatting, linting, tests). This doc defines the **local** commands that best mirror the CI quality gates so failures are caught before pushing.

## Canonical commands

### Full (CI-like) validation

Run from repo root:

```bash
just validate
```

This executes:

```bash
./scripts/validate-production.sh --ci
```

In `--ci` mode the script runs the following (failing fast on the first error):

#### Rust workspace
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --all-features`

#### Frontend (cwd: `crates/gestura-gui/frontend`)
1. `npm ci`
2. `npm run lint`
3. `npm run build` (TypeScript + Vite)
4. Install Playwright Chromium
   - Linux: `npx playwright install --with-deps chromium`
   - Other OSes: `npx playwright install chromium`
5. `npm run test:e2e:smoke`

### Quick validation

For faster iteration:

```bash
just validate-quick
```

This is intentionally smaller than `just validate`.

## Tips

- Rust-only: `./scripts/validate-production.sh --ci --skip-frontend`
- Frontend-only: `./scripts/validate-production.sh --ci --skip-rust`
- Skip Playwright browser install: add `--no-playwright-install`
