# Frontend refactor acceptance checklist (Gestura GUI)

Use this checklist for every refactor slice PR that touches `crates/gestura-gui/frontend/src/**`.

## A. Must-not-break invariants

### App shell & navigation
- [ ] App boots to the main shell with header + sidebar visible.
- [ ] Default panel is **Voice** (`Voice Processing` heading visible).
- [ ] Sidebar buttons switch panels: Voice, Ring, Tools, Workflows, Simulator, Settings.
- [ ] Panel switch does not crash the UI or leave it in a blank state.

### Config load/save (Tauri IPC)
- [ ] On boot, `get_config` is invoked and the UI renders with a non-null config.
- [ ] Changing settings triggers the same backend commands as before (no contract drift):
  - `save_config` (payload `{ cfg }`)
  - `set_ui_prefs` (payload `{ ui }`)
- [ ] Failure to load config shows a clear error state (no infinite spinner).

### Help system & shortcuts
- [ ] Clicking the help button (`?`) opens Help.
- [ ] Pressing **F1** toggles Help open/closed.
- [ ] Pressing **Esc** closes Help when it is open.

### Onboarding
- [ ] If `localStorage.gestura_onboarding_completed` is missing/falsey, onboarding overlay appears.
- [ ] If the flag is set to `"true"`, onboarding overlay does not block navigation.

### Theme
- [ ] ThemeController applies the correct theme tokens based on config `ui.theme_mode`.
- [ ] Switching theme does not break layout or readability.

## B. Quick regression commands (run every slice)

From repo root:
- [ ] `cargo fmt`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-features`

Frontend (from `crates/gestura-gui/frontend`):
- [ ] `npm run lint`
- [ ] `npm run build`
- [ ] `npm run test:e2e`

## C. Notes to include in PR description
- [ ] What moved/changed (paths)
- [ ] Any public API / IPC contract changes (should be none for refactor slices)
- [ ] Commands run + results (including e2e)

