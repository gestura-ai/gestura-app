## Installation (Full GUI+CLI vs CLI-only)

Gestura supports two install paths:

- **Full** (default): installs **GUI + CLI** via native installers (**PKG/DEB/RPM/MSI**).
- **CLI-only**: installs only the `gestura` CLI.

All official assets are published on GitHub Releases:

- https://github.com/gestura-ai/gestura-app/releases

### Option A: Full install (GUI + CLI)

1. Download the correct installer for your OS from the Release you want.
2. Install it using your OS’s normal installer flow.

Expected result:

- **macOS (PKG)**: `/Applications/Gestura.app` + CLI at `/usr/local/bin/gestura`
- **Linux (DEB/RPM)**: GUI app + CLI at `/usr/bin/gestura`
- **Windows (MSI)**: GUI app + `gestura.exe` on PATH

Canonical installer names are defined in `docs/INSTALLER_ARTIFACT_CONTRACT.md`.

### Option B: Non-interactive bootstrap installer

This is best for automation (CI machines, dev boxes, dotfiles, etc.).

#### macOS / Linux (bash)

- Install latest **Full** (GUI+CLI):
  - `curl -fsSL https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.sh | bash`
- Install latest **CLI-only**:
  - `curl -fsSL https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.sh | bash -s -- --mode cli`

Common flags:

- Pin a release tag: `--tag vX.Y.Z`
- Skip checksum verification (not recommended): `--no-verify`
- Fail if checksum file is missing: `--require-verify`
- Print actions without installing: `--dry-run`

#### Windows (PowerShell)

- Load the installer function and run (defaults to **Full**):
  - `iwr -useb https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.ps1 | iex; Install-Gestura`
- CLI-only:
  - `iwr -useb https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.ps1 | iex; Install-Gestura -Mode cli`

Common flags:

- Pin a release tag: `-Tag vX.Y.Z`
- Skip checksum verification (not recommended): `-NoVerify`
- Fail if checksum file is missing: `-RequireVerify`
- Print actions without installing: `-DryRun`

### Checksum verification

By default, both installers verify downloads against the release manifest:

- `gestura-${TAG}-SHA256SUMS.txt`

The exact canonical asset names that are checksummed are documented in:

- `docs/INSTALLER_ARTIFACT_CONTRACT.md`

