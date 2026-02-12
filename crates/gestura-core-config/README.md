# gestura-core-config

Configuration types, loading, validation, and file watching for Gestura.

## What belongs here

- `AppConfig` struct and all nested configuration types
- Environment variable loading (`config_env`)
- Configuration validation rules
- File-system watcher for live config reload
- Hook/plugin configuration types

Keep security-bridge methods and runtime state out of this crate; those remain in the `gestura-core` facade.

## Modules

- `types`        `AppConfig` and all nested config structs/enums
- `config_env`   Environment variable loader (`load_from_env`)
- `validation`   Configuration validation rules
- `watcher`      File-system watcher for live reload
- `hooks_types`  Hook and plugin configuration types

## Stable import paths

Most code should import through the facade:

- `gestura_core::config::*`
- `gestura_core::config_env::*`

The facade in `crates/gestura-core/src/config.rs` re-exports this crate plus a security extension trait (`AppConfigSecurityExt`).

## Development

```bash
cargo test -p gestura-core-config
cargo clippy -p gestura-core-config --all-targets --all-features -- -D warnings
```

