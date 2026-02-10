# RFC: Frontend module boundaries + target folder map (gestura-gui)

Last updated: 2026-02-10

Scope: `crates/gestura-gui/frontend/src/**`

## Goals

1. **Prevent IPC contract drift**: all Tauri IPC calls go through a typed boundary.
2. **Enable safe refactors**: move files without changing behavior, in small slices.
3. **Clarify ownership**: app shell vs features vs shared utilities.
4. **Keep presentation thin**: UI remains a thin layer over core business logic (core-first architecture).

Non-goals:
- No redesign of UI/UX.
- No Rust behavior changes.
- No new dependencies required for the boundary structure.

## Target folder map (end state)

This is a **directional** structure; migration occurs slice-by-slice.

```
src/
  app/
    AppShell.tsx            # high-level orchestration, routing/panel selection
    controllers/            # theme/help/onboarding shell glue
  features/
    voice/
      ui/VoicePanel.tsx
      services/voiceIpc.ts
      types.ts
    ring/
      ui/RingPanel.tsx
      services/ringIpc.ts
      types.ts
    tools/
    workflows/
    mcp/
    simulator/
    settings/
  services/
    tauri/
      invoke.ts             # typed invoke wrapper + error normalization
      contracts.ts          # central type map for IPC (command -> args/result)
  shared/
    hooks/
    storage/
    ui/
    util/
  types/
    config.ts               # stable cross-feature config types
```

Notes:
- Feature folders may keep `types.ts` if types are feature-local.
- Cross-feature / IPC-wide types live in `src/services/tauri/contracts.ts` (or `src/types/ipc.ts`).

## Boundary rules (allowed import directions)

**Rule of thumb:** dependencies point inward toward lower-level modules.

- `app/**` may import from `features/**`, `shared/**`, `services/**`, `types/**`.
- `features/**` may import from `shared/**`, `services/**`, `types/**`.
- `shared/**` **must not** import from `features/**` or `app/**`.
- `services/**` **must not** import from React UI modules.
- `types/**` contains pure types only; no side effects.

## Tauri IPC boundary (Phase 2)

### Requirements

- Only `services/tauri/*` should call `invoke()` from `@tauri-apps/api/core`.
- IPC functions should expose:
  - typed args
  - typed return value
  - normalized error type / message

### Why

- Prevents casing/shape drift (`snake_case` payload keys).
- Makes Playwright mocking simpler and more reliable.
- Enables gradual refactors while keeping behavior stable.

## Migration order (matches task list)

1. **Phase 2**: introduce `services/tauri` boundary; migrate `App.tsx` config calls first.
2. **Phase 3**: introduce `app/`, `features/`, `shared/` folders; move files slice-by-slice.
3. **Phase 4**: extract shared hooks/utilities where duplication is proven.

## Enforcement (lightweight)

Suggested (no new deps required):
- Prefer explicit imports (avoid wildcard exports that hide layering issues).
- Keep `docs/IPC_CONTRACTS_GESTURA_GUI.md` updated when commands change.

Optional later:
- ESLint rules for boundary enforcement (would require config work).
