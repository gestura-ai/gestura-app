# WebKit/JavaScriptCore Dependencies Fix

## Issue Identified

The GitHub Actions build was failing with this error:
```
error: failed to run custom build command for `javascriptcore-rs-sys v1.1.1`
The system library `javascriptcoregtk-4.1` required by crate `javascriptcore-rs-sys` was not found.
The file `javascriptcoregtk-4.1.pc` needs to be installed and the PKG_CONFIG_PATH environment variable must contain its parent directory.
```

## Root Cause

Tauri applications require WebKit and JavaScriptCore development libraries on Linux systems. The Ubuntu runners in GitHub Actions were missing these essential dependencies:

- `libjavascriptcoregtk-4.0-dev` - JavaScriptCore GTK 4.0 development files
- `libjavascriptcoregtk-4.1-dev` - JavaScriptCore GTK 4.1 development files  
- `libwebkit2gtk-4.1-dev` - WebKit2 GTK 4.1 development files
- `libsoup2.4-dev` and `libsoup-3.0-dev` - HTTP client/server library

## Solution Applied

### Updated System Dependencies

**Before** (incomplete):
```yaml
sudo apt-get install -y --no-install-recommends \
  libwebkit2gtk-4.0-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libudev-dev \
  libdbus-1-dev \
  pkg-config \
  build-essential \
  libssl-dev
```

**After** (complete):
```yaml
sudo apt-get install -y --no-install-recommends \
  libwebkit2gtk-4.0-dev \
  libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.0-dev \
  libjavascriptcoregtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libudev-dev \
  libdbus-1-dev \
  pkg-config \
  build-essential \
  libssl-dev \
  libsoup2.4-dev \
  libsoup-3.0-dev
```

### Files Updated

1. **`.github/workflows/ci.yml`**
   - Main CI workflow Ubuntu dependencies
   - Coverage job Ubuntu dependencies

2. **`.github/workflows/release.yml`**
   - Release workflow Ubuntu dependencies

3. **`.github/workflows/package-managers.yml`**
   - AppImage build Ubuntu dependencies

## Dependencies Explained

### Core WebKit Dependencies
- **`libwebkit2gtk-4.0-dev`** - WebKit2 GTK 4.0 development headers
- **`libwebkit2gtk-4.1-dev`** - WebKit2 GTK 4.1 development headers (newer version)

### JavaScriptCore Dependencies  
- **`libjavascriptcoregtk-4.0-dev`** - JavaScriptCore GTK 4.0 development headers
- **`libjavascriptcoregtk-4.1-dev`** - JavaScriptCore GTK 4.1 development headers (newer version)

### HTTP/Network Dependencies
- **`libsoup2.4-dev`** - HTTP client/server library (legacy version)
- **`libsoup-3.0-dev`** - HTTP client/server library (modern version)

### Why Both Versions?

Tauri and its dependencies may require different versions of WebKit/JavaScriptCore:
- **4.0 versions**: Legacy compatibility for older systems
- **4.1 versions**: Modern features and better performance
- **Both soup versions**: Different WebKit versions may depend on different libsoup versions

## Expected Results

### ✅ Fixed Build Issues
- JavaScriptCore compilation errors resolved
- WebKit2GTK dependencies satisfied
- pkg-config can find all required `.pc` files

### ✅ Supported Tauri Features
- Full WebView functionality
- JavaScript engine integration
- HTTP/HTTPS networking
- Modern web standards support

### ✅ Cross-Platform Compatibility
- Ubuntu 22.04 builds work correctly
- All Tauri GUI features available
- Consistent behavior across development and CI environments

## Verification Commands

To verify dependencies are installed correctly:

```bash
# Check if JavaScriptCore is available
pkg-config --exists javascriptcoregtk-4.1 && echo "JavaScriptCore 4.1 found"
pkg-config --exists javascriptcoregtk-4.0 && echo "JavaScriptCore 4.0 found"

# Check if WebKit2GTK is available  
pkg-config --exists webkit2gtk-4.1 && echo "WebKit2GTK 4.1 found"
pkg-config --exists webkit2gtk-4.0 && echo "WebKit2GTK 4.0 found"

# Check if libsoup is available
pkg-config --exists libsoup-3.0 && echo "libsoup 3.0 found"
pkg-config --exists libsoup-2.4 && echo "libsoup 2.4 found"
```

## Development Environment Setup

For local development on Ubuntu/Debian systems:

```bash
# Install all required dependencies
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.0-dev \
  libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.0-dev \
  libjavascriptcoregtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libudev-dev \
  libdbus-1-dev \
  pkg-config \
  build-essential \
  libssl-dev \
  libsoup2.4-dev \
  libsoup-3.0-dev

# Verify installation
pkg-config --list-all | grep -E "(webkit|javascript|soup)"
```

The GitHub Actions workflows should now build successfully with full Tauri WebView support on Ubuntu systems.
