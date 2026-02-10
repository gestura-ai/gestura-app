# Baseline validation runs

This document records known-good baseline runs for quick regression checking during refactors.

## 2026-02-10 (local macOS)

### Frontend (from `crates/gestura-gui/frontend`)

- `npm run lint`
  - Result: ✅ pass
  - Wall time: ~6.2s

- `npm run build`
  - Result: ✅ pass
  - Wall time: ~8.4s

- `npm run test:e2e:smoke`
  - Result: ✅ pass (5 tests, Chromium only)
  - Wall time: ~5.4s

- `npm run test:e2e`
  - Result: ✅ pass (54 tests total across Chromium/Firefox/WebKit; 52 passed, 2 skipped)
    - Note: the `@smoke` “F1 shortcut” check is intentionally skipped outside Chromium (Playwright limitation).
  - Wall time: ~21.5s (with `npm run test:e2e -- --reporter=line`)

