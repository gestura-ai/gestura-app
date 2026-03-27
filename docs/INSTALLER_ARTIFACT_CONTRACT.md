## Installer artifact contract (Gestura.app)

This document defines the **canonical release asset names** and **selection rules** used by Gestura’s non-interactive installers.

### Versioning

- **Release tag** is the source of truth: `vX.Y.Z` (example: `v0.2.0`).
- All asset names below use `${TAG}` meaning the full tag string including the leading `v`.

### Modes

- **Full** (default): installs **GUI + CLI**.
- **CLI-only**: installs only the `gestura` CLI.

### Canonical assets (Full installers)

| Platform | Format | Asset name |
|---|---|---|
| macOS | PKG | `Gestura-${TAG}-universal.pkg` |
| Linux | DEB | `gestura-${TAG}-linux-x86_64.deb` |
| Linux | RPM | `gestura-${TAG}-linux-x86_64.rpm` |
| Windows | MSI | `Gestura-${TAG}-windows-x86_64.msi` |

Notes:

- The macOS PKG installer should install:
  - `/Applications/Gestura.app`
  - `/usr/local/bin/gestura` (CLI)
- Linux DEB/RPM should install:
  - GUI application
  - CLI to `/usr/bin/gestura` (preferred for system packages)
- Windows MSI should install:
  - GUI app
  - `gestura.exe` CLI into the install directory
  - Add the install directory to PATH (system if elevated, otherwise user)

### Canonical assets (CLI-only)

| Platform | Format | Asset name |
|---|---|---|
| macOS | tar.gz | `gestura-cli-${TAG}-macos-universal.tar.gz` |
| Linux | tar.gz | `gestura-cli-${TAG}-linux-x86_64.tar.gz` |
| Windows | zip | `gestura-cli-${TAG}-windows-x86_64.zip` |

Each archive contains a single executable named:

- macOS/Linux: `gestura`
- Windows: `gestura.exe`

Homebrew note:

- The macOS CLI archive (`gestura-cli-${TAG}-macos-universal.tar.gz`) is the
  canonical standalone release asset for Homebrew formula/tap submissions.
- Use the matching SHA-256 entry from `gestura-${TAG}-SHA256SUMS.txt`.

### Checksums

For each release tag, publish:

- `gestura-${TAG}-SHA256SUMS.txt`

This file contains SHA-256 checksums for all canonical assets above, in the standard format:

`<sha256>  <filename>`

### Backward compatibility

Installer scripts should:

1. Prefer canonical assets above.
2. If missing, fall back to legacy asset names (if present in older releases), including:
   - `gestura-cli-macos-universal`
   - `gestura-cli-linux-x86_64`
   - `gestura-cli-windows-x86_64.exe`
