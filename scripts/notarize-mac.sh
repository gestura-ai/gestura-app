#!/bin/bash
# macOS Notarization Script for Gestura.app
# This script handles code signing and notarization for local development builds.
#
# Prerequisites:
#   1. Apple Developer ID Application certificate installed in Keychain
#   2. Apple Developer ID Installer certificate (for PKG signing)
#   3. App-specific password stored in Keychain (for notarization)
#
# Required Environment Variables:
#   APPLE_SIGNING_IDENTITY - Developer ID Application certificate name
#                            e.g., "Developer ID Application: Your Name (TEAMID)"
#   APPLE_TEAM_ID          - Your Apple Developer Team ID (10-char alphanumeric)
#   APPLE_ID               - Your Apple ID email for notarization
#   APPLE_PASSWORD         - App-specific password for notarization
#                            (or use @keychain:notarytool-password)
#
# Optional Environment Variables:
#   APPLE_INSTALLER_IDENTITY - Developer ID Installer certificate for PKG signing
#   NOTARIZATION_WAIT        - Set to "false" to submit without waiting (default: true)

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
APP_NAME="Gestura"
VERSION=$(grep -o '"version": "[^"]*"' package.json | cut -d'"' -f4)
BUNDLE_ID="ai.gestura.desktop"

# Paths
UNIVERSAL_APP_PATH="src-tauri/target/universal-apple-darwin/release/bundle/macos/${APP_NAME}.app"
RELEASE_APP_PATH="src-tauri/target/release/bundle/macos/${APP_NAME}.app"
DIST_DIR="dist/macos"

echo -e "${BLUE}🔐 Gestura macOS Notarization Script${NC}"
echo -e "${BLUE}Version: ${VERSION}${NC}"
echo ""

# Validate required environment variables
validate_env() {
    echo -e "${YELLOW}🔍 Validating environment...${NC}"
    
    local missing=0
    
    if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
        echo -e "${RED}❌ APPLE_SIGNING_IDENTITY not set${NC}"
        echo "   Set to your Developer ID Application certificate name"
        echo "   Example: export APPLE_SIGNING_IDENTITY=\"Developer ID Application: Your Name (TEAMID)\""
        missing=1
    else
        echo -e "${GREEN}✅ APPLE_SIGNING_IDENTITY: ${APPLE_SIGNING_IDENTITY}${NC}"
    fi
    
    if [ -z "${APPLE_TEAM_ID:-}" ]; then
        echo -e "${RED}❌ APPLE_TEAM_ID not set${NC}"
        echo "   Set to your 10-character Apple Developer Team ID"
        missing=1
    else
        echo -e "${GREEN}✅ APPLE_TEAM_ID: ${APPLE_TEAM_ID}${NC}"
    fi
    
    if [ -z "${APPLE_ID:-}" ]; then
        echo -e "${RED}❌ APPLE_ID not set${NC}"
        echo "   Set to your Apple ID email for notarization"
        missing=1
    else
        echo -e "${GREEN}✅ APPLE_ID: ${APPLE_ID}${NC}"
    fi
    
    if [ -z "${APPLE_PASSWORD:-}" ]; then
        echo -e "${RED}❌ APPLE_PASSWORD not set${NC}"
        echo "   Set to your app-specific password or @keychain:profile-name"
        echo "   Create at: https://appleid.apple.com/account/manage"
        missing=1
    else
        echo -e "${GREEN}✅ APPLE_PASSWORD: [set]${NC}"
    fi
    
    if [ $missing -eq 1 ]; then
        echo ""
        echo -e "${YELLOW}💡 Tip: Add these to your shell profile (~/.zshrc or ~/.bashrc):${NC}"
        echo "   export APPLE_SIGNING_IDENTITY=\"Developer ID Application: ...\""
        echo "   export APPLE_TEAM_ID=\"XXXXXXXXXX\""
        echo "   export APPLE_ID=\"your@email.com\""
        echo "   export APPLE_PASSWORD=\"@keychain:notarytool-password\""
        echo ""
        echo -e "${YELLOW}💡 To store password in Keychain:${NC}"
        echo "   xcrun notarytool store-credentials notarytool-password \\"
        echo "     --apple-id your@email.com \\"
        echo "     --team-id XXXXXXXXXX \\"
        echo "     --password <app-specific-password>"
        exit 1
    fi
    
    # Verify certificate exists in keychain
    echo -e "${YELLOW}🔍 Checking keychain for signing certificate...${NC}"
    if ! security find-identity -v -p codesigning | grep -q "${APPLE_SIGNING_IDENTITY}"; then
        echo -e "${RED}❌ Certificate not found in keychain: ${APPLE_SIGNING_IDENTITY}${NC}"
        echo "   Available certificates:"
        security find-identity -v -p codesigning | grep "Developer ID Application" || echo "   (none found)"
        exit 1
    fi
    echo -e "${GREEN}✅ Certificate found in keychain${NC}"
}

# Find the app bundle
find_app_bundle() {
    if [ -d "${UNIVERSAL_APP_PATH}" ]; then
        APP_PATH="${UNIVERSAL_APP_PATH}"
        echo -e "${GREEN}✅ Found universal app bundle${NC}"
    elif [ -d "${RELEASE_APP_PATH}" ]; then
        APP_PATH="${RELEASE_APP_PATH}"
        echo -e "${GREEN}✅ Found release app bundle${NC}"
    else
        echo -e "${RED}❌ App bundle not found. Run 'just build-macos' first.${NC}"
        exit 1
    fi
}

# Sign the application
sign_app() {
    echo -e "${YELLOW}🔐 Signing application...${NC}"
    
    # Sign all nested binaries and frameworks first
    echo "   Signing nested components..."
    find "${APP_PATH}" -type f \( -name "*.dylib" -o -name "*.so" \) -exec \
        codesign --force --options runtime --timestamp \
        --sign "${APPLE_SIGNING_IDENTITY}" {} \; 2>/dev/null || true
    
    # Sign frameworks
    if [ -d "${APP_PATH}/Contents/Frameworks" ]; then
        find "${APP_PATH}/Contents/Frameworks" -type d -name "*.framework" -exec \
            codesign --force --options runtime --timestamp \
            --sign "${APPLE_SIGNING_IDENTITY}" {} \; 2>/dev/null || true
    fi
    
    # Sign the main executable
    echo "   Signing main executable..."
    codesign --force --options runtime --timestamp \
        --entitlements src-tauri/entitlements.plist \
        --sign "${APPLE_SIGNING_IDENTITY}" \
        "${APP_PATH}/Contents/MacOS/${APP_NAME}"
    
    # Sign the app bundle
    echo "   Signing app bundle..."
    codesign --force --options runtime --timestamp \
        --entitlements src-tauri/entitlements.plist \
        --sign "${APPLE_SIGNING_IDENTITY}" \
        "${APP_PATH}"
    
    echo -e "${GREEN}✅ Application signed${NC}"
}

# Verify signature
verify_signature() {
    echo -e "${YELLOW}🔍 Verifying signature...${NC}"

    codesign --verify --deep --strict --verbose=2 "${APP_PATH}"

    echo -e "${GREEN}✅ Signature verified${NC}"
}

# Create ZIP for notarization
create_zip() {
    echo -e "${YELLOW}📦 Creating ZIP for notarization...${NC}"

    mkdir -p "${DIST_DIR}"
    ZIP_PATH="${DIST_DIR}/${APP_NAME}-${VERSION}-notarize.zip"

    rm -f "${ZIP_PATH}"
    ditto -c -k --keepParent "${APP_PATH}" "${ZIP_PATH}"

    echo -e "${GREEN}✅ ZIP created: ${ZIP_PATH}${NC}"
}

# Submit for notarization
notarize_app() {
    echo -e "${YELLOW}📋 Submitting for notarization...${NC}"

    ZIP_PATH="${DIST_DIR}/${APP_NAME}-${VERSION}-notarize.zip"

    # Check if using keychain profile
    if [[ "${APPLE_PASSWORD}" == @keychain:* ]]; then
        PROFILE_NAME="${APPLE_PASSWORD#@keychain:}"
        echo "   Using keychain profile: ${PROFILE_NAME}"

        if [ "${NOTARIZATION_WAIT:-true}" = "true" ]; then
            xcrun notarytool submit "${ZIP_PATH}" \
                --keychain-profile "${PROFILE_NAME}" \
                --wait
        else
            xcrun notarytool submit "${ZIP_PATH}" \
                --keychain-profile "${PROFILE_NAME}"
        fi
    else
        if [ "${NOTARIZATION_WAIT:-true}" = "true" ]; then
            xcrun notarytool submit "${ZIP_PATH}" \
                --apple-id "${APPLE_ID}" \
                --team-id "${APPLE_TEAM_ID}" \
                --password "${APPLE_PASSWORD}" \
                --wait
        else
            xcrun notarytool submit "${ZIP_PATH}" \
                --apple-id "${APPLE_ID}" \
                --team-id "${APPLE_TEAM_ID}" \
                --password "${APPLE_PASSWORD}"
        fi
    fi

    echo -e "${GREEN}✅ Notarization submitted${NC}"
}

# Staple the notarization ticket
staple_app() {
    echo -e "${YELLOW}📎 Stapling notarization ticket...${NC}"

    xcrun stapler staple "${APP_PATH}"

    echo -e "${GREEN}✅ Notarization ticket stapled${NC}"
}

# Verify notarization
verify_notarization() {
    echo -e "${YELLOW}🔍 Verifying notarization...${NC}"

    spctl -a -t exec -vv "${APP_PATH}"

    echo -e "${GREEN}✅ Notarization verified - app is ready for distribution${NC}"
}

# Main execution
main() {
    validate_env
    find_app_bundle
    sign_app
    verify_signature
    create_zip
    notarize_app
    staple_app
    verify_notarization

    echo ""
    echo -e "${GREEN}🎉 Notarization complete!${NC}"
    echo -e "${BLUE}📁 Signed app: ${APP_PATH}${NC}"
    echo ""
    echo -e "${YELLOW}Next steps:${NC}"
    echo "  1. Run 'just package-macos' to create DMG and PKG"
    echo "  2. The packages will inherit the notarized signature"
}

# Error handling
trap 'echo -e "${RED}❌ Notarization failed at line $LINENO${NC}"; exit 1' ERR

# Run main function
main "$@"

