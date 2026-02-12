# Frontend duplication clusters + shared UI patterns (gestura-gui)

Last updated: 2026-02-10

Scope: `crates/gestura-gui/frontend/src/**`

This is the **Phase 1.2** artifact. It catalogs repeated patterns and concrete extraction candidates so later refactors can proceed slice-by-slice with minimal behavior change.

## 1) Async IPC loading pattern (loading + try/catch/finally)

Repeated pattern:
- local `loading` state
- `try { await invoke(...) } catch { console.error(...) } finally { setLoading(false) }`

Examples:
- `App.tsx`: config load + mutations (`get_config`, `save_config`, `set_ui_prefs`)
- `WorkflowsPanel.tsx`: `loadData()` (also uses `Promise.all`)
- `ToolsPanel.tsx`: tool list load
- `McpPanel.tsx`: servers/tools status load
- `SimulatorPanel.tsx`: simulator list load
- `OnboardingWizard.tsx`: permissions checks + follow-up actions

Why it matters:
- Error semantics diverge (some swallowed, some rendered, some only console-logged).
- Harder to mock consistently in Playwright without accidental "green" runs.

Extraction candidates:
- Phase 2: `src/services/tauri/invoke.ts` typed wrapper + normalized errors/logging.
- Phase 4: `useAsyncState()`-style hook (optional) to standardize `{loading, error, refresh}`.

## 2) Polling / periodic refresh (setInterval + cleanup)

Repeated pattern:
- call async `refresh()` immediately
- `setInterval(refresh, N)`
- cleanup with `clearInterval`

Examples:
- `StatusBar.tsx`: refresh every 5s; also contains a cancellation guard.
- `WorkflowsPanel.tsx`: refresh every 5s.

Extraction candidates:
- `src/shared/hooks/usePolling.ts` with:
  - stable callback handling
  - `enabled` toggle
  - built-in cancellation guard

## 3) Cancellation guards / race control for async effects

Pattern:
- `let cancelled = false;` and early-return checks to avoid state updates after unmount.

Example:
- `StatusBar.tsx`.

Notes:
- Some panels do not guard (often fine for user-triggered actions), but the inconsistency is a drift risk.

Extraction candidates:
- incorporate into `usePolling` / `useAsyncState`, or a small `useMountedRef()` helper.

## 4) LocalStorage onboarding gate

Pattern:
- `localStorage.getItem('gestura_onboarding_completed')` gating the app shell.
- `localStorage.setItem('gestura_onboarding_completed', 'true')` on completion.

Examples:
- `App.tsx`: checks the flag on mount.
- `OnboardingWizard.tsx`: sets the flag when onboarding completes.

Extraction candidates:
- `src/shared/storage/onboarding.ts`:
  - `getOnboardingCompleted(): boolean`
  - `setOnboardingCompleted(value: boolean): void`
  - optional `useOnboardingCompleted()` hook

## 5) Keyboard shortcut handling

Current implementation:
- `App.tsx` installs a `keydown` listener for:
  - `F1`: toggle Help
  - `Escape`: close Help when open

Docs/UI mismatch:
- `HelpSystem.tsx` documents many shortcuts (Ctrl+1/2/3, Ctrl+, etc.), but they are not currently implemented globally.

Extraction candidates:
- `src/shared/hooks/useKeyboardShortcuts.ts` (single listener + centralized mapping)

## 6) One-off timers for transitions / animation

Pattern:
- `window.setTimeout` stored in refs
- cleanup in an unmount effect

Example:
- `OnboardingWizard.tsx`: transition timers (animation/step unlock flows).

Notes:
- This is valid as-is; main risk is forgotten cleanup across future edits.

## 7) Duplicated domain types defined inline in components

Examples:
- `McpPanel.tsx` defines `McpServer`, `ServerStatus`, `McpClientTool`.
- `ToolsPanel.tsx` defines `McpServer` again.
- `WorkflowsPanel.tsx` defines `DelegatedTask`, `Agent`, `ListAgentsResponse`.
- `RingPanel.tsx` defines `RingStatus`.
- `SimulatorPanel.tsx` defines `SimulatorInfo`, `TestResults`.

Why it matters:
- Types drift across panels.
- Makes IPC contracts harder to type and mock consistently.

Extraction candidates:
- Phase 2/3: `src/types/ipc/*.ts` (or `src/features/*/types.ts`) aligned to Rust JSON.

## 8) Shared layout / styling primitives

Observed conventions:
- repeated `<div className="panel">` sections
- repeated `.form-group` patterns
- some inline layout styles (flex/grid) used ad hoc

Extraction candidates:
- Phase 4: shared UI primitives (`Panel`, `Section`, `FormRow`) **only where duplication is proven**.

## 9) Error visibility / user feedback

Current state:
- many errors are logged to console only
- a few panels show user-visible strings for results/errors (e.g. `VoicePanel.tsx`)

Extraction candidates:
- normalized `InvokeError` model from Phase 2 IPC wrapper
- minimal shared UI for inline errors (optional)

## 10) IPC payload casing & contract drift

High-risk area:
- Rust commands frequently expect `snake_case` payload keys.

Mitigations:
- `docs/IPC_CONTRACTS_GESTURA_GUI.md` remains the human-readable contract map.
- Phase 2 typed IPC boundary prevents drift by construction.
