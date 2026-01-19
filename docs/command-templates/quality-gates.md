# Quality gates (Rust/Tauri)

Run these from the repository root.

## Fast loop (recommended during iteration)

1. `just validate-quick`
2. If it fails:
   - Fix the underlying issue.
   - Re-run `just validate-quick` until green.

## Full loop (before merging/releasing)

1. `cargo fmt`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --all-features`

## “No TODOs in code” rule

We track remaining work in **`TODO.md`**. Do **not** leave `TODO`/`FIXME`/`XXX` markers (or Rust `todo!()` / `unimplemented!()`) in shipped code.

Suggested check (from repo root):

- `grep -RInw --exclude-dir node_modules --exclude-dir dist --exclude-dir target --exclude-dir .git --exclude=TODO.md -e TODO -e FIXME -e XXX crates crates/gestura-gui/frontend`
- `grep -RIn --exclude-dir node_modules --exclude-dir dist --exclude-dir target --exclude-dir .git -e "todo!" -e "unimplemented!" crates crates/gestura-gui/frontend`

## Notes

- Prefer the smallest scope that still gives signal (single test → file → crate → workspace), but don’t skip the full loop before release.
- Keep GUI/CLI thin; push shared behavior into `crates/gestura-core/`.
