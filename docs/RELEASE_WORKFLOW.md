# Release Workflow

## Overview

Gestura’s canonical release pipeline lives in `.github/workflows/release.yml` and is now **definition-driven**, **tag-and-version validated**, **signed on macOS and Windows**, and **configurable for higher-capacity runners** when GitHub-hosted resources are not enough.

The canonical release contract is stored in `release/release-definition.json`. It defines:

- the supported platform release matrix
- the feature set shipped per platform
- the canonical release artifacts expected for each platform
- the downstream channels that each release must support

## Triggers

### Automatic publish

- `push` to a tag matching `v*`
- The tag **must** match the in-repo version (`v<workspace version>`)

### Manual dispatch

- `workflow_dispatch`
- Inputs:
  - `ref`: branch, tag, or SHA to build
  - `publish`: whether to create/update the GitHub release after the build

Manual dispatch is useful for dry runs on a release candidate commit before pushing the final tag.

## Version and tag rules

The workflow reads and compares these files before any platform build starts:

- `Cargo.toml` → `workspace.package.version`
- `crates/gestura-gui/tauri.conf.json` → `version`
- `crates/gestura-gui/frontend/package.json` → `version`

The workflow fails immediately if:

- any version differs
- the version is not valid semver
- a tag-triggered run is not for `v<version>`

## Release definition and feature matrix

The tag workflow validates `release/release-definition.json` before any platform build starts.

Current canonical public release matrix:

- macOS: universal GUI + CLI with `voice-local,macos-permissions`
- Linux: x86_64 GUI + CLI with `voice-local,linux-permissions`
- Windows: x86_64 GUI + CLI with `voice-local,windows-permissions`

If the release definition and the generated artifact set diverge, the publish job fails before a GitHub release is created or updated.

## Release job layout

1. **Prepare**
   - validates versions and tags
   - resolves the exact release commit SHA
   - checks whether macOS and Windows signing secrets are present
2. **Build macOS**
   - builds a universal CLI with separate target dirs
   - stages per-arch sidecars for Tauri universal bundling
   - signs the CLI, app, and PKG
   - publishes both the full PKG and the standalone CLI tarball used for Homebrew submission
   - verifies notarization and stapled tickets
3. **Build Linux**
   - builds `.deb` / `.rpm` installers and the standalone CLI tarball
   - validates packaged CLI placement and emitted DEB/RPM runtime dependency metadata
4. **Build Windows**
   - imports the PFX certificate
   - injects the certificate thumbprint into `tauri.conf.json` during the build
   - signs the MSI and standalone CLI executable
   - publishes both the MSI and standalone CLI zip
   - verifies signatures with `Get-AuthenticodeSignature`
5. **Publish release**
   - downloads all artifacts
   - generates a unified SHA256 manifest
   - validates the full artifact set against `release/release-definition.json`
   - generates a machine-readable release manifest asset
   - creates or updates the GitHub release and uploads assets

## Canonical release assets

The workflow publishes these OS release packages and companion CLI archives:

- macOS
  - `Gestura-vX.Y.Z-universal.pkg`
  - `gestura-cli-vX.Y.Z-macos-universal.tar.gz`
- Linux
  - `gestura-vX.Y.Z-linux-x86_64.deb`
  - `gestura-vX.Y.Z-linux-x86_64.rpm`
  - `gestura-cli-vX.Y.Z-linux-x86_64.tar.gz`
- Windows
  - `Gestura-vX.Y.Z-windows-x86_64.msi`
  - `gestura-cli-vX.Y.Z-windows-x86_64.zip`
- Checksums
  - `gestura-vX.Y.Z-SHA256SUMS.txt`
- Release manifest
  - `gestura-vX.Y.Z-release-manifest.json`

The macOS standalone archive (`gestura-cli-vX.Y.Z-macos-universal.tar.gz`) is the
canonical release asset to reference from a Homebrew formula or tap update.

## Channel completeness

The release definition currently treats these channels as part of release readiness:

- `github-release` — published directly by the tag workflow
- `homebrew-cli` — documented downstream update using the macOS CLI tarball + checksums + release manifest
- `winget` — documented downstream update using the Windows MSI + checksums + release manifest

The publish job validates that the assets required by those channels are present before it publishes the GitHub release.

## Required secrets for signed releases

### macOS

- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_INSTALLER_CERTIFICATE`
- `APPLE_INSTALLER_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_INSTALLER_IDENTITY`
- `APPLE_TEAM_ID`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `KEYCHAIN_PASSWORD`

### Windows

- `WINDOWS_CERTIFICATE`
- `WINDOWS_CERTIFICATE_PASSWORD`

If `publish=true` (or the workflow is tag-triggered), the workflow **fails early** when these signing secrets are incomplete.

## Runner overrides for resource pressure

The release workflow supports repository/org variables so Windows and macOS can move to larger or self-hosted runners without changing YAML again.

Supported variables:

- `RELEASE_MACOS_RUNNER`
- `RELEASE_WINDOWS_RUNNER`
- `RELEASE_LINUX_RUNNER`

Each variable must be valid JSON for `runs-on`.

Examples:

- GitHub-hosted default: `["macos-14"]`
- macOS self-hosted: `["self-hosted", "macOS", "arm64", "gestura-release"]`
- Windows self-hosted: `["self-hosted", "Windows", "x64", "gestura-release"]`

## Recommended release process

1. Update the version consistently:
   - `just set-version X.Y.Z`
2. Run the canonical local validation workflow:
   - `just validate`
   - `just show-version`
3. Merge the release commit
4. Push the release tag:
   - `git tag vX.Y.Z`
   - `git push origin vX.Y.Z`
5. Monitor the GitHub Actions run and verify uploaded assets

## Manual dry-run example

```bash
gh workflow run release.yml -f ref=main -f publish=false
```

## Notes

- macOS and Windows signing are enforced for published releases.
- Linux packages are built and checksummed but do not use platform code signing.
- Release publication is intentionally separate from Homebrew/Winget submission so unrelated downstream jobs cannot block installer creation.