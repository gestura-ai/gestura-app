#!/bin/bash
# Windows packaging script for Gestura.app

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
APP_NAME="Gestura"
VERSION=$(grep -o '"version": "[^"]*"' package.json | cut -d'"' -f4)
BUILD_DIR="src-tauri/target/release"
DIST_DIR="dist/windows"
SIGNING_CERT="${WINDOWS_SIGNING_CERT:-}"
SIGNING_PASSWORD="${WINDOWS_SIGNING_PASSWORD:-}"

echo -e "${BLUE}📦 Starting Windows packaging for ${APP_NAME} v${VERSION}${NC}"

# Check prerequisites
check_prerequisites() {
    echo -e "${YELLOW}🔍 Checking prerequisites...${NC}"
    
    # Check if running on Windows or WSL
    if [[ "$OSTYPE" != "msys" && "$OSTYPE" != "cygwin" && ! -f /proc/version ]]; then
        echo -e "${RED}❌ This script should be run on Windows or WSL${NC}"
        exit 1
    fi
    
    # Check for required tools
    if ! command -v makensis &> /dev/null; then
        echo -e "${RED}❌ NSIS not found. Please install NSIS.${NC}"
        exit 1
    fi
    
    if ! command -v candle &> /dev/null; then
        echo -e "${YELLOW}⚠️ WiX Toolset not found. MSI creation will be skipped.${NC}"
    fi
    
    echo -e "${GREEN}✅ Prerequisites check complete${NC}"
}

# Build the application
build_app() {
    echo -e "${YELLOW}🔨 Building application...${NC}"
    
    # Build with Tauri for Windows
    npm run tauri build -- --target x86_64-pc-windows-msvc
    
    if [ ! -f "${BUILD_DIR}/${APP_NAME}.exe" ]; then
        echo -e "${RED}❌ Build failed - executable not found${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}✅ Application built successfully${NC}"
}

# Create distribution directory
setup_dist() {
    echo -e "${YELLOW}📁 Setting up distribution directory...${NC}"
    
    rm -rf "${DIST_DIR}"
    mkdir -p "${DIST_DIR}"
    
    # Copy executable and dependencies
    cp "${BUILD_DIR}/${APP_NAME}.exe" "${DIST_DIR}/"
    
    # Copy any DLL dependencies
    if [ -d "${BUILD_DIR}/deps" ]; then
        cp -r "${BUILD_DIR}/deps"/* "${DIST_DIR}/"
    fi
    
    echo -e "${GREEN}✅ Distribution directory ready${NC}"
}

# Code sign the executable
code_sign() {
    if [ -z "${SIGNING_CERT}" ]; then
        echo -e "${YELLOW}⚠️ No signing certificate provided, skipping code signing${NC}"
        return 0
    fi
    
    echo -e "${YELLOW}🔐 Code signing executable...${NC}"
    
    # Sign the main executable
    signtool sign \
        /f "${SIGNING_CERT}" \
        /p "${SIGNING_PASSWORD}" \
        /t http://timestamp.digicert.com \
        /fd SHA256 \
        /d "${APP_NAME}" \
        /du "https://gestura.app" \
        "${DIST_DIR}/${APP_NAME}.exe"
    
    # Verify signature
    signtool verify /pa "${DIST_DIR}/${APP_NAME}.exe"
    
    echo -e "${GREEN}✅ Code signing complete${NC}"
}

# Create NSIS installer
create_nsis_installer() {
    echo -e "${YELLOW}📦 Creating NSIS installer...${NC}"
    
    INSTALLER_NAME="${APP_NAME}-${VERSION}-x64-setup.exe"
    
    # Create NSIS script
    cat > "${DIST_DIR}/installer.nsi" << EOF
!define APP_NAME "${APP_NAME}"
!define APP_VERSION "${VERSION}"
!define APP_PUBLISHER "Gestura AI"
!define APP_URL "https://gestura.app"
!define APP_EXECUTABLE "${APP_NAME}.exe"

!include "MUI2.nsh"
!include "x64.nsh"

Name "\${APP_NAME}"
OutFile "${INSTALLER_NAME}"
InstallDir "\$PROGRAMFILES64\\\${APP_NAME}"
InstallDirRegKey HKLM "Software\\\${APP_NAME}" "InstallDir"
RequestExecutionLevel admin

!define MUI_ABORTWARNING
!define MUI_ICON "..\\..\\src-tauri\\icons\\icon.ico"
!define MUI_UNICON "..\\..\\src-tauri\\icons\\icon.ico"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\\..\\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_WELCOME
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

Section "Install"
    SetOutPath "\$INSTDIR"
    
    File "${APP_NAME}.exe"
    File /nonfatal /r "deps\\*"
    
    WriteRegStr HKLM "Software\\\${APP_NAME}" "InstallDir" "\$INSTDIR"
    WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "DisplayName" "\${APP_NAME}"
    WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "UninstallString" "\$INSTDIR\\uninstall.exe"
    WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "DisplayIcon" "\$INSTDIR\\\${APP_EXECUTABLE}"
    WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "Publisher" "\${APP_PUBLISHER}"
    WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "URLInfoAbout" "\${APP_URL}"
    WriteRegStr HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "DisplayVersion" "\${APP_VERSION}"
    WriteRegDWORD HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "NoModify" 1
    WriteRegDWORD HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}" "NoRepair" 1
    
    CreateDirectory "\$SMPROGRAMS\\\${APP_NAME}"
    CreateShortcut "\$SMPROGRAMS\\\${APP_NAME}\\\${APP_NAME}.lnk" "\$INSTDIR\\\${APP_EXECUTABLE}"
    CreateShortcut "\$DESKTOP\\\${APP_NAME}.lnk" "\$INSTDIR\\\${APP_EXECUTABLE}"
    
    WriteUninstaller "\$INSTDIR\\uninstall.exe"
SectionEnd

Section "Uninstall"
    Delete "\$INSTDIR\\\${APP_EXECUTABLE}"
    Delete "\$INSTDIR\\uninstall.exe"
    RMDir /r "\$INSTDIR\\deps"
    RMDir "\$INSTDIR"
    
    Delete "\$SMPROGRAMS\\\${APP_NAME}\\\${APP_NAME}.lnk"
    RMDir "\$SMPROGRAMS\\\${APP_NAME}"
    Delete "\$DESKTOP\\\${APP_NAME}.lnk"
    
    DeleteRegKey HKLM "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\\${APP_NAME}"
    DeleteRegKey HKLM "Software\\\${APP_NAME}"
SectionEnd
EOF
    
    # Compile NSIS installer
    cd "${DIST_DIR}"
    makensis installer.nsi
    cd - > /dev/null
    
    if [ ! -f "${DIST_DIR}/${INSTALLER_NAME}" ]; then
        echo -e "${RED}❌ NSIS installer creation failed${NC}"
        exit 1
    fi
    
    # Sign the installer
    if [ -n "${SIGNING_CERT}" ]; then
        signtool sign \
            /f "${SIGNING_CERT}" \
            /p "${SIGNING_PASSWORD}" \
            /t http://timestamp.digicert.com \
            /fd SHA256 \
            /d "${APP_NAME} Installer" \
            /du "https://gestura.app" \
            "${DIST_DIR}/${INSTALLER_NAME}"
    fi
    
    echo -e "${GREEN}✅ NSIS installer created: ${INSTALLER_NAME}${NC}"
}

# Create MSI installer (if WiX is available)
create_msi_installer() {
    if ! command -v candle &> /dev/null; then
        echo -e "${YELLOW}⚠️ WiX Toolset not available, skipping MSI creation${NC}"
        return 0
    fi
    
    echo -e "${YELLOW}📦 Creating MSI installer...${NC}"
    
    MSI_NAME="${APP_NAME}-${VERSION}-x64.msi"
    
    # Create WiX source file
    cat > "${DIST_DIR}/installer.wxs" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
    <Product Id="*" Name="${APP_NAME}" Language="1033" Version="${VERSION}" 
             Manufacturer="Gestura AI" UpgradeCode="12345678-1234-1234-1234-123456789012">
        
        <Package InstallerVersion="200" Compressed="yes" InstallScope="perMachine" />
        
        <MajorUpgrade DowngradeErrorMessage="A newer version is already installed." />
        
        <MediaTemplate EmbedCab="yes" />
        
        <Feature Id="ProductFeature" Title="${APP_NAME}" Level="1">
            <ComponentGroupRef Id="ProductComponents" />
        </Feature>
        
        <Directory Id="TARGETDIR" Name="SourceDir">
            <Directory Id="ProgramFiles64Folder">
                <Directory Id="INSTALLFOLDER" Name="${APP_NAME}" />
            </Directory>
            <Directory Id="ProgramMenuFolder">
                <Directory Id="ApplicationProgramsFolder" Name="${APP_NAME}" />
            </Directory>
            <Directory Id="DesktopFolder" />
        </Directory>
        
        <ComponentGroup Id="ProductComponents" Directory="INSTALLFOLDER">
            <Component Id="MainExecutable" Guid="*">
                <File Id="MainExe" Source="${APP_NAME}.exe" KeyPath="yes">
                    <Shortcut Id="ApplicationStartMenuShortcut" Directory="ApplicationProgramsFolder"
                              Name="${APP_NAME}" Description="${APP_NAME}" WorkingDirectory="INSTALLFOLDER" />
                    <Shortcut Id="ApplicationDesktopShortcut" Directory="DesktopFolder"
                              Name="${APP_NAME}" Description="${APP_NAME}" WorkingDirectory="INSTALLFOLDER" />
                </File>
            </Component>
        </ComponentGroup>
        
        <Property Id="WIXUI_INSTALLDIR" Value="INSTALLFOLDER" />
        <UIRef Id="WixUI_InstallDir" />
        
    </Product>
</Wix>
EOF
    
    # Compile WiX
    cd "${DIST_DIR}"
    candle installer.wxs
    light installer.wixobj -out "${MSI_NAME}"
    cd - > /dev/null
    
    if [ ! -f "${DIST_DIR}/${MSI_NAME}" ]; then
        echo -e "${RED}❌ MSI installer creation failed${NC}"
        return 1
    fi
    
    # Sign the MSI
    if [ -n "${SIGNING_CERT}" ]; then
        signtool sign \
            /f "${SIGNING_CERT}" \
            /p "${SIGNING_PASSWORD}" \
            /t http://timestamp.digicert.com \
            /fd SHA256 \
            /d "${APP_NAME} MSI Installer" \
            /du "https://gestura.app" \
            "${DIST_DIR}/${MSI_NAME}"
    fi
    
    echo -e "${GREEN}✅ MSI installer created: ${MSI_NAME}${NC}"
}

# Create portable ZIP
create_portable_zip() {
    echo -e "${YELLOW}📦 Creating portable ZIP...${NC}"
    
    ZIP_NAME="${APP_NAME}-${VERSION}-x64-portable.zip"
    
    # Create portable directory
    PORTABLE_DIR="${DIST_DIR}/portable"
    mkdir -p "${PORTABLE_DIR}"
    
    cp "${DIST_DIR}/${APP_NAME}.exe" "${PORTABLE_DIR}/"
    
    # Copy dependencies
    if [ -d "${DIST_DIR}/deps" ]; then
        cp -r "${DIST_DIR}/deps" "${PORTABLE_DIR}/"
    fi
    
    # Create README for portable version
    cat > "${PORTABLE_DIR}/README.txt" << EOF
${APP_NAME} v${VERSION} - Portable Version

This is a portable version of ${APP_NAME} that doesn't require installation.

To run:
1. Extract all files to a folder
2. Run ${APP_NAME}.exe
3. The application will store settings in the same folder

System Requirements:
- Windows 10 or later (64-bit)
- 4GB RAM minimum, 8GB recommended
- 500MB free disk space
- Bluetooth 5.0+ for Haptic Harmony Ring

Support:
- Website: https://gestura.app
- Support: support@gestura.app
- Documentation: https://docs.gestura.app
EOF
    
    # Create ZIP
    cd "${DIST_DIR}"
    zip -r "${ZIP_NAME}" portable/
    cd - > /dev/null
    
    # Clean up portable directory
    rm -rf "${PORTABLE_DIR}"
    
    echo -e "${GREEN}✅ Portable ZIP created: ${ZIP_NAME}${NC}"
}

# Generate checksums
generate_checksums() {
    echo -e "${YELLOW}🔢 Generating checksums...${NC}"
    
    cd "${DIST_DIR}"
    
    # Generate SHA256 checksums
    for file in *.exe *.msi *.zip; do
        if [ -f "$file" ]; then
            sha256sum "$file" > "$file.sha256"
            echo -e "${GREEN}✅ Checksum for $file${NC}"
        fi
    done
    
    cd - > /dev/null
}

# Create release notes
create_release_info() {
    echo -e "${YELLOW}📝 Creating release information...${NC}"
    
    cat > "${DIST_DIR}/RELEASE_INFO.txt" << EOF
Gestura.app v${VERSION} - Windows Release

Build Information:
- Version: ${VERSION}
- Platform: Windows x64
- Build Date: $(date -u +"%Y-%m-%d %H:%M:%S UTC")
- Build Host: $(hostname)

Files:
- ${APP_NAME}-${VERSION}-x64-setup.exe (NSIS Installer)
- ${APP_NAME}-${VERSION}-x64.msi (MSI Installer)
- ${APP_NAME}-${VERSION}-x64-portable.zip (Portable Version)

Installation:
1. Download and run the setup.exe installer
2. Or use the MSI for enterprise deployment
3. Or extract the portable ZIP for no-install usage

System Requirements:
- Windows 10 or later (64-bit)
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
    echo -e "${BLUE}🚀 Starting Windows packaging process${NC}"
    
    check_prerequisites
    build_app
    setup_dist
    code_sign
    create_nsis_installer
    create_msi_installer
    create_portable_zip
    generate_checksums
    create_release_info
    
    echo -e "${GREEN}🎉 Windows packaging complete!${NC}"
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
