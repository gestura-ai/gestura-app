# GitHub Actions Build Fixes

## Issues Identified and Resolved

### 1. **Code Coverage Job - Feature Flag Issue**
**Problem**: Coverage job was using `--all-features` which includes BLE features that fail on Ubuntu due to missing Linux-specific dependencies.

**Solution**: 
- Changed coverage to use `--no-default-features --features tauri-gui`
- Added Node.js setup and frontend build to coverage job
- Added proper system dependencies for GUI builds

**Before**:
```yaml
- name: Generate code coverage
  run: cargo tarpaulin --verbose --all-features --workspace --timeout 120 --out xml
```

**After**:
```yaml
- name: Setup Node.js
  uses: actions/setup-node@v4
  with:
    node-version: 'lts/*'
    cache: 'npm'
    cache-dependency-path: ui/package-lock.json

- name: Install frontend dependencies
  run: cd ui && npm ci

- name: Build frontend
  run: cd ui && npm run build

- name: Generate code coverage
  run: cargo tarpaulin --verbose --no-default-features --features tauri-gui --workspace --timeout 120 --out xml
```

### 2. **Clippy Job - Cross-Platform Feature Conflicts**
**Problem**: Clippy was trying to build GUI features on all platforms/Rust versions, but frontend is only built on stable Rust.

**Solution**: Split clippy into two conditional jobs:
- Beta Rust: CLI-only clippy checks
- Stable Rust: GUI clippy checks (with frontend built)

**Before**:
```yaml
- name: Run clippy
  run: cargo clippy --all-targets --no-default-features --features tauri-gui -- -D warnings
```

**After**:
```yaml
- name: Run clippy (CLI)
  if: matrix.rust == 'beta'
  run: cargo clippy --all-targets --no-default-features --features cli-only -- -D warnings

- name: Run clippy (GUI)
  if: matrix.rust == 'stable'
  run: cargo clippy --all-targets --no-default-features --features tauri-gui -- -D warnings
```

### 3. **Feature Flag Consistency**
**Problem**: Mixed usage of `--all-features` and specific feature flags across different jobs.

**Solution**: Standardized feature flag usage across all workflows:
- **CLI builds**: `--no-default-features --features cli-only`
- **GUI builds**: `--no-default-features --features tauri-gui`
- **Tests**: Split between no-features and GUI features
- **Coverage**: GUI features with proper frontend build

## Updated CI Matrix Strategy

### Build Matrix
```yaml
strategy:
  matrix:
    os: [ubuntu-22.04, windows-latest, macos-latest]
    rust: [stable, beta]
    exclude:
      - os: windows-latest
        rust: beta
      - os: macos-latest
        rust: beta
```

### Job Responsibilities

#### **All Platforms + Rust Versions**
- Code formatting check
- CLI builds and tests
- Basic clippy (CLI features only) on beta Rust

#### **Stable Rust Only**
- Frontend build (Node.js/npm)
- GUI builds and tests
- GUI clippy checks
- Coverage generation

#### **Ubuntu 22.04 Only**
- Code coverage with tarpaulin
- Enhanced system dependency installation

## Feature Flag Strategy

### Core Features
- `cli-only`: Cross-platform CLI functionality
- `tauri-gui`: GUI with Tauri frontend
- `ble`: Cross-platform BLE (btleplug) - optional
- `linux-ble`: Linux-specific BLE (bluer) - optional

### Build Combinations
```bash
# CLI builds (all platforms)
cargo build --no-default-features --features cli-only

# GUI builds (requires frontend)
cd ui && npm run build
cargo build --no-default-features --features tauri-gui

# BLE builds (Linux only)
cargo build --features linux-ble

# Full feature builds (Linux only)
cargo build --features tauri-gui,linux-ble
```

## System Dependencies

### Ubuntu 22.04
```bash
sudo apt-get update --fix-missing
sudo apt-get install -y --no-install-recommends \
  libwebkit2gtk-4.0-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libudev-dev \
  libdbus-1-dev \
  pkg-config \
  build-essential \
  curl \
  wget \
  file \
  libssl-dev
```

### macOS
```bash
brew install pkg-config || true
```

### Windows
- No additional system dependencies required
- Uses MSVC toolchain

## Workflow Execution Flow

### CI Workflow (`.github/workflows/ci.yml`)
1. **Setup**: Checkout, Rust toolchain, system dependencies
2. **Conditional Setup**: Node.js and frontend (stable Rust only)
3. **Code Quality**: Formatting, clippy (split by Rust version)
4. **Testing**: Unit tests, GUI tests (stable only)
5. **Building**: CLI (all), GUI (stable only)

### Coverage Workflow
1. **Setup**: Ubuntu 22.04, stable Rust, system dependencies
2. **Frontend**: Node.js setup, npm install, build
3. **Coverage**: Tarpaulin with GUI features
4. **Upload**: Codecov integration

### Release Workflow
1. **Trigger**: Main branch push or manual dispatch
2. **Multi-platform**: All OS/architecture combinations
3. **Features**: GUI builds with proper frontend compilation
4. **Artifacts**: Compressed executables, installers

## Expected Results

### ✅ Fixed Issues
- Code coverage builds successfully with GUI features
- Clippy runs without feature conflicts
- All platforms build correctly
- No more BLE dependency failures on non-Linux platforms

### ✅ Maintained Functionality
- Cross-platform CLI builds
- GUI builds with Tauri frontend
- Optional BLE features for Linux
- Comprehensive test coverage
- Multi-architecture release builds

### ✅ Performance Improvements
- Faster builds due to selective feature compilation
- Better caching with conditional Node.js setup
- Reduced dependency conflicts

The GitHub Actions workflows are now properly configured for reliable, cross-platform builds with appropriate feature flag usage and dependency management.
