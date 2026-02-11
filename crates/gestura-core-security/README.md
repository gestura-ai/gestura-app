# gestura-core-security

Security domain crate: encryption, keychain storage, sandboxing, and GDPR compliance for Gestura.

## What belongs here

- Secure storage abstraction (keychain / credential manager)
- AES-256-GCM encryption for config values
- Sandbox policy evaluation and enforcement
- GDPR data handling and consent management
- Secret provider interfaces

Keep application-level security policy (e.g., config bridge methods) in the `gestura-core` facade.

## Modules

- `storage`      `SecureStorage` trait + `KeychainStorage` implementation
- `encryption`   `Encryptor` and `SecureConfigManager` (AES-256-GCM)
- `secrets`      `SecureStorageSecretProvider` for runtime secret access
- `sandbox`      Sandbox policy evaluation and enforcement
- `gdpr`         GDPR compliance helpers and data handling

## Stable import paths

Most code should import through the facade:

- `gestura_core::security::*`
- `gestura_core::sandbox::*`
- `gestura_core::gdpr::*`
- `gestura_core::secrets::*`

The facades in `crates/gestura-core/src/` re-export this crate.

## Development

```bash
cargo test -p gestura-core-security
cargo clippy -p gestura-core-security --all-targets --all-features -- -D warnings
```

