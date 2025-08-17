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
VERSION=$(grep -o '"version": "[^"]*"' package.json | cut -d'"' -f4)
BUILD_DIR="src-tauri/target/release/bundle/macos"
DIST_DIR="dist/macos"
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
NOTARIZATION_PROFILE="${APPLE_NOTARIZATION_PROFILE:-}"

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
        # Create mock app bundle for testing
        mkdir -p "${BUILD_DIR}"
        mkdir -p "${BUILD_DIR}/${APP_NAME}.app/Contents/MacOS"
        echo "Mock app" > "${BUILD_DIR}/${APP_NAME}.app/Contents/MacOS/${APP_NAME}"
        chmod +x "${BUILD_DIR}/${APP_NAME}.app/Contents/MacOS/${APP_NAME}"
        echo -e "${GREEN}✅ Mock application created for testing${NC}"
        return 0
    fi

    # Build with Tauri
    npm run tauri build -- --target universal-apple-darwin

    if [ ! -d "${BUILD_DIR}/${APP_NAME}.app" ]; then
        echo -e "${RED}❌ Build failed - app bundle not found${NC}"
        exit 1
    fi

    echo -e "${GREEN}✅ Application built successfully${NC}"
}

# Create distribution directory
setup_dist() {
    echo -e "${YELLOW}📁 Setting up distribution directory...${NC}"
    
    rm -rf "${DIST_DIR}"
    mkdir -p "${DIST_DIR}"
    
    # Copy app bundle
    cp -R "${BUILD_DIR}/${APP_NAME}.app" "${DIST_DIR}/"
    
    echo -e "${GREEN}✅ Distribution directory ready${NC}"
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
    codesign --force --verify --verbose --sign "${SIGNING_IDENTITY}" --entitlements src-tauri/entitlements.plist "${DIST_DIR}/${APP_NAME}.app"
    
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
    if [ -f "src-tauri/icons/icon.icns" ]; then
        DMG_ARGS+=(--volicon "src-tauri/icons/icon.icns")
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
    
    # Create component package
    pkgbuild \
        --root "${DIST_DIR}" \
        --identifier "${BUNDLE_ID}" \
        --version "${VERSION}" \
        --install-location "/Applications" \
        "${PKG_PATH}"
    
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
