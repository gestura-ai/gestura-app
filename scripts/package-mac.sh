#!/bin/bash
# macOS packaging script for Gestura.app

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
APP_NAME="Gestura"
BUNDLE_ID="com.gestura.app"
VERSION=$(grep -o '"version": "[^"]*"' crates/gestura-gui/frontend/package.json | cut -d'"' -f4)
# BUILD_DIR will be set after build completes (see resolve_build_dir function)
BUILD_DIR=""
DIST_DIR="dist/macos"
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
NOTARIZATION_PROFILE="${APPLE_NOTARIZATION_PROFILE:-}"

# Paths - Updated for new workspace structure (crates/gestura-gui)
GUI_DIR="crates/gestura-gui"

# Resolve the correct build directory after build completes
resolve_build_dir() {
    # Tauri builds universal macOS binaries to a different path
    local universal_path="${GUI_DIR}/target/universal-apple-darwin/release/bundle/macos"
    local regular_path="${GUI_DIR}/target/release/bundle/macos"

    if [ -d "${universal_path}/${APP_NAME}.app" ]; then
        BUILD_DIR="${universal_path}"
        echo -e "${GREEN}✅ Found app bundle at universal path${NC}"
    elif [ -d "${regular_path}/${APP_NAME}.app" ]; then
        BUILD_DIR="${regular_path}"
        echo -e "${GREEN}✅ Found app bundle at regular path${NC}"
    else
        echo -e "${RED}❌ App bundle not found in either:${NC}"
        echo -e "${RED}   - ${universal_path}/${APP_NAME}.app${NC}"
        echo -e "${RED}   - ${regular_path}/${APP_NAME}.app${NC}"
        exit 1
    fi
}

echo -e "${BLUE}📦 Starting macOS packaging for ${APP_NAME} v${VERSION}${NC}"

# Check prerequisites
check_prerequisites() {
    echo -e "${YELLOW}🔍 Checking prerequisites...${NC}"
    
    if ! command -v create-dmg &> /dev/null; then
        echo -e "${RED}❌ create-dmg not found. Installing...${NC}"
        brew install create-dmg
    fi
    
    if ! command -v pkgbuild &> /dev/null; then
        echo -e "${RED}❌ pkgbuild not found. Please install Xcode Command Line Tools.${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}✅ Prerequisites check complete${NC}"
}

# Build the application
build_app() {
    echo -e "${YELLOW}🔨 Building application...${NC}"

    # Check if this is a dry run
    if [ "${DRY_RUN:-false}" = "true" ]; then
        echo -e "${YELLOW}🔍 DRY RUN: Skipping actual build${NC}"
        # Create mock app bundle for testing in the universal path
        local mock_path="${GUI_DIR}/target/universal-apple-darwin/release/bundle/macos"
        mkdir -p "${mock_path}/${APP_NAME}.app/Contents/MacOS"
        echo "Mock app" > "${mock_path}/${APP_NAME}.app/Contents/MacOS/${APP_NAME}"
        chmod +x "${mock_path}/${APP_NAME}.app/Contents/MacOS/${APP_NAME}"
        echo -e "${GREEN}✅ Mock application created for testing${NC}"
        # Resolve the build directory
        resolve_build_dir
        return 0
    fi

    # Build with Tauri (from the GUI directory)
    cd "${GUI_DIR}/frontend" && npm install && npm run build && cd - >/dev/null
    cd "${GUI_DIR}" && cargo tauri build --features voice-local -- --target universal-apple-darwin && cd - >/dev/null

    # Resolve the build directory after build completes
    resolve_build_dir

    echo -e "${GREEN}✅ Application built successfully${NC}"
}

# Create distribution directory
setup_dist() {
    echo -e "${YELLOW}📁 Setting up distribution directory...${NC}"

	# If a previous dist directory exists, avoid modifying it directly.
	# In some environments a prior run may have been executed with sudo,
	# leaving root-owned files that the current user cannot delete. Rather
	# than failing on "rm -rf", we always fall back to a fresh,
	# timestamped dist directory when an existing one is detected.
	if [ -d "${DIST_DIR}" ]; then
	    echo -e "${YELLOW}⚠️ Existing dist directory (${DIST_DIR}) detected.${NC}"
	    echo -e "${YELLOW}   To avoid permission issues from previous runs, a new directory will be used.${NC}"
	    local timestamp
	    timestamp=$(date +"%Y%m%d-%H%M%S")
	    DIST_DIR="${DIST_DIR}-${timestamp}"
	    echo -e "${YELLOW}ℹ️ Using alternate distribution directory: ${DIST_DIR}${NC}"
	fi

	mkdir -p "${DIST_DIR}"

	# Copy app bundle
	cp -R "${BUILD_DIR}/${APP_NAME}.app" "${DIST_DIR}/"

	# Add privacy usage descriptions to Info.plist
	inject_privacy_descriptions

	echo -e "${GREEN}✅ Distribution directory ready at ${DIST_DIR}${NC}"
}

# Inject privacy usage descriptions into Info.plist
inject_privacy_descriptions() {
	local info_plist="${DIST_DIR}/${APP_NAME}.app/Contents/Info.plist"

	if [ ! -f "${info_plist}" ]; then
		echo -e "${YELLOW}⚠️ Info.plist not found, skipping privacy descriptions${NC}"
		return 0
	fi

	echo -e "${YELLOW}📝 Adding privacy usage descriptions to Info.plist...${NC}"

	# Add microphone usage description
	/usr/libexec/PlistBuddy -c "Add :NSMicrophoneUsageDescription string 'Gestura needs microphone access for voice commands and speech-to-text functionality.'" "${info_plist}" 2>/dev/null || \
	/usr/libexec/PlistBuddy -c "Set :NSMicrophoneUsageDescription 'Gestura needs microphone access for voice commands and speech-to-text functionality.'" "${info_plist}"

	# Add Bluetooth usage descriptions
	/usr/libexec/PlistBuddy -c "Add :NSBluetoothAlwaysUsageDescription string 'Gestura uses Bluetooth to connect to the Haptic Harmony Ring for gesture control.'" "${info_plist}" 2>/dev/null || \
	/usr/libexec/PlistBuddy -c "Set :NSBluetoothAlwaysUsageDescription 'Gestura uses Bluetooth to connect to the Haptic Harmony Ring for gesture control.'" "${info_plist}"

	/usr/libexec/PlistBuddy -c "Add :NSBluetoothPeripheralUsageDescription string 'Gestura uses Bluetooth to connect to the Haptic Harmony Ring for gesture control.'" "${info_plist}" 2>/dev/null || \
	/usr/libexec/PlistBuddy -c "Set :NSBluetoothPeripheralUsageDescription 'Gestura uses Bluetooth to connect to the Haptic Harmony Ring for gesture control.'" "${info_plist}"

	echo -e "${GREEN}✅ Privacy descriptions added${NC}"
}

# Code sign the application
code_sign() {
    if [ -z "${SIGNING_IDENTITY}" ]; then
        echo -e "${YELLOW}⚠️ No signing identity provided, skipping code signing${NC}"
        return 0
    fi

    echo -e "${YELLOW}🔐 Code signing application...${NC}"

    # Sign all binaries and frameworks
    find "${DIST_DIR}/${APP_NAME}.app" -type f \( -name "*.dylib" -o -name "*.so" \) -exec codesign --force --verify --verbose --sign "${SIGNING_IDENTITY}" {} \;

    # Sign frameworks
    find "${DIST_DIR}/${APP_NAME}.app/Contents/Frameworks" -type d -name "*.framework" -exec codesign --force --verify --verbose --sign "${SIGNING_IDENTITY}" {} \;

    # Sign the main executable
    codesign --force --verify --verbose --sign "${SIGNING_IDENTITY}" "${DIST_DIR}/${APP_NAME}.app/Contents/MacOS/${APP_NAME}"

    # Sign the app bundle
    codesign --force --verify --verbose --sign "${SIGNING_IDENTITY}" --entitlements "${GUI_DIR}/entitlements.plist" "${DIST_DIR}/${APP_NAME}.app"

    # Verify signature
    codesign --verify --deep --strict --verbose=2 "${DIST_DIR}/${APP_NAME}.app"
    spctl -a -t exec -vv "${DIST_DIR}/${APP_NAME}.app"

    echo -e "${GREEN}✅ Code signing complete${NC}"
}

# Create DMG
create_dmg() {
    echo -e "${YELLOW}💿 Creating DMG...${NC}"
    
    DMG_NAME="${APP_NAME}-${VERSION}-universal.dmg"
    DMG_PATH="${DIST_DIR}/${DMG_NAME}"
    
    # Remove existing DMG
    rm -f "${DMG_PATH}"
    
    # Create DMG with create-dmg
    DMG_ARGS=(
        --volname "${APP_NAME}"
        --window-pos 200 120
        --window-size 600 400
        --icon-size 100
        --icon "${APP_NAME}.app" 175 120
        --hide-extension "${APP_NAME}.app"
        --app-drop-link 425 120
    )

    # Add optional assets if they exist
    if [ -f "${GUI_DIR}/icons/icon.icns" ]; then
        DMG_ARGS+=(--volicon "${GUI_DIR}/icons/icon.icns")
    fi

    if [ -f "assets/dmg-background.png" ]; then
        DMG_ARGS+=(--background "assets/dmg-background.png")
    fi

    create-dmg "${DMG_ARGS[@]}" "${DMG_PATH}" "${DIST_DIR}/${APP_NAME}.app"
    
    if [ ! -f "${DMG_PATH}" ]; then
        echo -e "${RED}❌ DMG creation failed${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}✅ DMG created: ${DMG_NAME}${NC}"
}

# Create PKG installer
create_pkg() {
    echo -e "${YELLOW}📦 Creating PKG installer...${NC}"
    
    PKG_NAME="${APP_NAME}-${VERSION}-universal.pkg"
	PKG_PATH="${DIST_DIR}/${PKG_NAME}"
	PKGROOT_DIR="${DIST_DIR}/pkgroot"

	# Prepare a clean package root so that the PKG only installs the
	# application bundle into /Applications (and any optional CLI tools into
	# /usr/local/bin).
	rm -rf "${PKGROOT_DIR}"
	mkdir -p "${PKGROOT_DIR}/Applications"

	# App bundle goes under /Applications
	cp -R "${DIST_DIR}/${APP_NAME}.app" "${PKGROOT_DIR}/Applications/"

	# CLI tooling: The CLI binary is built separately via cargo build -p gestura-cli
	# and installed into /usr/local/bin. Check multiple possible locations.
	CLI_UNIVERSAL_PATH="target/universal-apple-darwin/release/gestura"
	CLI_RELEASE_PATH="target/release/gestura"
	CLI_BIN_PATH=""

	if [ -f "${CLI_UNIVERSAL_PATH}" ]; then
	    CLI_BIN_PATH="${CLI_UNIVERSAL_PATH}"
	    echo -e "${GREEN}✅ Found universal CLI binary${NC}"
	elif [ -f "${CLI_RELEASE_PATH}" ]; then
	    CLI_BIN_PATH="${CLI_RELEASE_PATH}"
	    echo -e "${GREEN}✅ Found release CLI binary${NC}"
	else
	    echo -e "${YELLOW}⚠️ CLI binary not found. Build with: cargo build --release -p gestura-cli${NC}"
	fi

	if [ -n "${CLI_BIN_PATH}" ]; then
	    mkdir -p "${PKGROOT_DIR}/usr/local/bin"
	    cp "${CLI_BIN_PATH}" "${PKGROOT_DIR}/usr/local/bin/gestura"
	    # Sign the CLI binary if signing identity is available
	    if [ -n "${SIGNING_IDENTITY}" ]; then
	        echo -e "${YELLOW}🔐 Signing CLI binary...${NC}"
	        codesign --force --options runtime --timestamp \
	            --sign "${SIGNING_IDENTITY}" \
	            "${PKGROOT_DIR}/usr/local/bin/gestura"
	    fi
	    echo -e "${GREEN}✅ Included CLI tool in /usr/local/bin/gestura${NC}"
	fi

	# Create component package. We use / as the install root so that the
	# payload paths inside PKGROOT_DIR (Applications/..., usr/local/bin/...)
	# map to /Applications and /usr/local/bin on the target system.
	UNSIGNED_PKG_PATH="${DIST_DIR}/${APP_NAME}-${VERSION}-unsigned.pkg"
	pkgbuild \
	    --root "${PKGROOT_DIR}" \
	    --identifier "${BUNDLE_ID}" \
	    --version "${VERSION}" \
	    --install-location "/" \
	    "${UNSIGNED_PKG_PATH}"

	# Sign the PKG if installer identity is available
	INSTALLER_IDENTITY="${APPLE_INSTALLER_IDENTITY:-}"
	if [ -n "${INSTALLER_IDENTITY}" ]; then
	    echo -e "${YELLOW}🔐 Signing PKG installer...${NC}"
	    productsign --sign "${INSTALLER_IDENTITY}" "${UNSIGNED_PKG_PATH}" "${PKG_PATH}"
	    rm -f "${UNSIGNED_PKG_PATH}"
	    echo -e "${GREEN}✅ PKG signed with: ${INSTALLER_IDENTITY}${NC}"
	else
	    # No installer identity, just rename the unsigned pkg
	    mv "${UNSIGNED_PKG_PATH}" "${PKG_PATH}"
	    echo -e "${YELLOW}⚠️ PKG not signed (set APPLE_INSTALLER_IDENTITY for signing)${NC}"
	fi
    
    if [ ! -f "${PKG_PATH}" ]; then
        echo -e "${RED}❌ PKG creation failed${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}✅ PKG created: ${PKG_NAME}${NC}"
}

# Notarize the application
notarize() {
    if [ -z "${NOTARIZATION_PROFILE}" ]; then
        echo -e "${YELLOW}⚠️ No notarization profile provided, skipping notarization${NC}"
        return 0
    fi
    
    echo -e "${YELLOW}📋 Notarizing application...${NC}"
    
    DMG_NAME="${APP_NAME}-${VERSION}-universal.dmg"
    DMG_PATH="${DIST_DIR}/${DMG_NAME}"
    
    # Submit for notarization
    xcrun notarytool submit "${DMG_PATH}" \
        --keychain-profile "${NOTARIZATION_PROFILE}" \
        --wait
    
    # Staple the notarization
    xcrun stapler staple "${DMG_PATH}"
    
    echo -e "${GREEN}✅ Notarization complete${NC}"
}

# Generate checksums
generate_checksums() {
    echo -e "${YELLOW}🔢 Generating checksums...${NC}"
    
    cd "${DIST_DIR}"
    
    # Generate SHA256 checksums
    for file in *.dmg *.pkg; do
        if [ -f "$file" ]; then
            shasum -a 256 "$file" > "$file.sha256"
            echo -e "${GREEN}✅ Checksum for $file${NC}"
        fi
    done
    
    cd - > /dev/null
}

# Create release notes
create_release_info() {
    echo -e "${YELLOW}📝 Creating release information...${NC}"
    
    cat > "${DIST_DIR}/RELEASE_INFO.txt" << EOF
Gestura.app v${VERSION} - macOS Release

Build Information:
- Version: ${VERSION}
- Platform: macOS Universal (Intel + Apple Silicon)
- Build Date: $(date -u +"%Y-%m-%d %H:%M:%S UTC")
- Build Host: $(hostname)

Files:
- ${APP_NAME}-${VERSION}-universal.dmg (Disk Image)
- ${APP_NAME}-${VERSION}-universal.pkg (Installer Package)

Installation:
1. Download the DMG file
2. Open the DMG and drag ${APP_NAME}.app to Applications
3. Or use the PKG installer for automated installation

System Requirements:
- macOS 10.15 (Catalina) or later
- 4GB RAM minimum, 8GB recommended
- 500MB free disk space
- Bluetooth 5.0+ for Haptic Harmony Ring

Support:
- Website: https://gestura.app
- Support: support@gestura.app
- Documentation: https://docs.gestura.app
EOF
    
    echo -e "${GREEN}✅ Release information created${NC}"
}

# Main execution
main() {
    echo -e "${BLUE}🚀 Starting macOS packaging process${NC}"
    
    check_prerequisites
    build_app
    setup_dist
    code_sign
    create_dmg
    create_pkg
    notarize
    generate_checksums
    create_release_info
    
    echo -e "${GREEN}🎉 macOS packaging complete!${NC}"
    echo -e "${BLUE}📁 Files created in: ${DIST_DIR}${NC}"
    
    # List created files
    echo -e "${YELLOW}📋 Created files:${NC}"
    ls -la "${DIST_DIR}"
    
    # Show file sizes
    echo -e "${YELLOW}📊 File sizes:${NC}"
    du -h "${DIST_DIR}"/*
}

# Error handling
trap 'echo -e "${RED}❌ Packaging failed at line $LINENO${NC}"; exit 1' ERR

# Run main function
main "$@"
