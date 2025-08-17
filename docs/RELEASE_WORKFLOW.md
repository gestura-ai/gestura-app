# Release Workflow - Main Branch Triggered

## Overview

The release system has been updated to trigger **only on merges to the main branch**, automatically creating releases with compressed executable bundles.

## Workflow Triggers

### Release Workflow (`.github/workflows/release.yml`)
**Triggers:**
- ✅ Push to `main` branch (merges)
- ✅ Manual dispatch (workflow_dispatch)
- ❌ ~~Git tags~~ (removed)

### Package Manager Publishing (`.github/workflows/package-managers.yml`)
**Triggers:**
- ✅ After successful Release workflow completion
- ✅ Manual dispatch (workflow_dispatch)

## Release Process

### 1. **Automatic Version Detection**
- Extracts version from `Cargo.toml`
- Creates release tag automatically (e.g., `v0.1.0`)
- Checks if release already exists to avoid duplicates

### 2. **Multi-Platform Builds**
Builds for all supported platforms:

#### macOS
- **Intel (x64)**: `x86_64-apple-darwin`
- **Apple Silicon (ARM64)**: `aarch64-apple-darwin`

#### Linux
- **Intel (x64)**: `x86_64-unknown-linux-gnu`
- **ARM64**: `aarch64-unknown-linux-gnu`

#### Windows
- **Intel (x64)**: `x86_64-pc-windows-msvc`
- **ARM64**: `aarch64-pc-windows-msvc`

### 3. **Compressed Executable Bundles**
All binaries are automatically compressed:

#### Archive Formats
- **Linux/macOS**: `.tar.gz` archives
- **Windows**: `.zip` archives

#### Naming Convention
```
haptic-harmony-simulator-{os}-{arch}.{ext}
```

Examples:
- `haptic-harmony-simulator-macos-x64.tar.gz`
- `haptic-harmony-simulator-macos-arm64.tar.gz`
- `haptic-harmony-simulator-linux-x64.tar.gz`
- `haptic-harmony-simulator-linux-arm64.tar.gz`
- `haptic-harmony-simulator-windows-x64.zip`
- `haptic-harmony-simulator-windows-arm64.zip`

### 4. **Release Creation**
- Creates GitHub release automatically
- Uploads all compressed bundles
- Generates release notes with download links
- Includes both CLI and GUI versions

### 5. **Package Manager Publishing**
After successful release creation:
- Updates Homebrew formula
- Publishes to Chocolatey
- Submits to Winget
- Publishes Snap package
- Creates AppImage for Linux

## How to Release

### Method 1: Merge to Main (Recommended)
```bash
# 1. Update version in Cargo.toml
vim Cargo.toml  # Update version = "1.0.0"

# 2. Commit and push to feature branch
git add Cargo.toml
git commit -m "Bump version to 1.0.0"
git push origin feature/version-bump

# 3. Create PR and merge to main
# This automatically triggers the release workflow
```

### Method 2: Direct Push to Main
```bash
# 1. Update version in Cargo.toml
vim Cargo.toml  # Update version = "1.0.0"

# 2. Commit and push directly to main
git add Cargo.toml
git commit -m "Release v1.0.0"
git push origin main

# This automatically triggers the release workflow
```

### Method 3: Manual Dispatch
```bash
# Use GitHub CLI or web interface
gh workflow run release.yml -f version=v1.0.0
```

## What Happens Automatically

### On Main Branch Merge:
1. **Version Detection**: Reads version from `Cargo.toml`
2. **Release Creation**: Creates GitHub release with tag
3. **Multi-Platform Build**: Builds for all 6 platform/arch combinations
4. **Compression**: Creates compressed archives for all binaries
5. **Upload**: Uploads all archives to the GitHub release
6. **Package Publishing**: Triggers package manager publishing workflow

### Release Assets Created:
- `haptic-harmony-simulator-macos-x64.tar.gz`
- `haptic-harmony-simulator-macos-arm64.tar.gz`
- `haptic-harmony-simulator-linux-x64.tar.gz`
- `haptic-harmony-simulator-linux-arm64.tar.gz`
- `haptic-harmony-simulator-windows-x64.zip`
- `haptic-harmony-simulator-windows-arm64.zip`
- `haptic-harmony-simulator-x86_64.AppImage` (Linux portable)
- Tauri-generated platform-specific installers (DMG, MSI, DEB, etc.)

## Monitoring Releases

### GitHub Actions
- Monitor workflow runs in the Actions tab
- Check for build failures or upload issues
- Review release creation and asset uploads

### Release Page
- Verify all expected assets are uploaded
- Check release notes are generated correctly
- Confirm download links work

### Package Managers
- Homebrew: Check formula updates
- Chocolatey: Verify package publication
- Winget: Confirm submission success
- Snap: Check store publication

## Troubleshooting

### Build Failures
- Check GitHub Actions logs
- Verify Cargo.toml version format
- Ensure all dependencies are available

### Missing Assets
- Check if compression step completed
- Verify upload permissions
- Review GitHub token permissions

### Package Manager Issues
- Check API keys in GitHub secrets
- Verify package configurations
- Review submission logs

## Version Management

### Semantic Versioning
Follow semantic versioning (semver):
- `MAJOR.MINOR.PATCH` (e.g., `1.2.3`)
- Breaking changes: increment MAJOR
- New features: increment MINOR
- Bug fixes: increment PATCH

### Pre-release Versions
For pre-releases, use:
- `1.0.0-alpha.1`
- `1.0.0-beta.1`
- `1.0.0-rc.1`

### Version in Cargo.toml
```toml
[package]
name = "haptic-harmony-simulation"
version = "1.0.0"  # Update this to trigger release
```

## Security Considerations

### Required Secrets
Ensure these secrets are set in GitHub repository settings:
- `GITHUB_TOKEN` (automatically provided)
- `HOMEBREW_TAP_TOKEN` (for Homebrew updates)
- `CHOCOLATEY_API_KEY` (for Chocolatey publishing)
- `WINGET_TOKEN` (for Winget submissions)
- `SNAPCRAFT_TOKEN` (for Snap store)

### Permissions
- Workflows have write access to releases
- Package manager tokens have appropriate scopes
- Cross-compilation tools are securely installed

## Benefits of This Approach

### Automated
- No manual tag creation required
- Automatic version detection
- Compressed bundles without manual steps

### Consistent
- Every main branch merge creates a release
- Standardized naming conventions
- Reliable compression and upload

### Comprehensive
- All platforms and architectures
- Multiple package managers
- Both CLI and GUI versions

### Traceable
- Clear workflow logs
- Version tied to commits
- Automated release notes

This workflow ensures that every merge to main creates a complete, professional release with compressed executables ready for distribution across all supported platforms and package managers.
