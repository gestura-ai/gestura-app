#!/bin/bash
# Linux packaging script for Gestura.app

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
APP_NAME="gestura"
APP_DISPLAY_NAME="Gestura"
VERSION=$(grep -o '"version": "[^"]*"' package.json | cut -d'"' -f4)
BUILD_DIR="src-tauri/target/release"
DIST_DIR="dist/linux"
MAINTAINER="Gestura AI <support@gestura.app>"
DESCRIPTION="Voice and gesture control application with Haptic Harmony ring integration"

echo -e "${BLUE}📦 Starting Linux packaging for ${APP_DISPLAY_NAME} v${VERSION}${NC}"

# Check prerequisites
check_prerequisites() {
    echo -e "${YELLOW}🔍 Checking prerequisites...${NC}"
    
    # Check for required tools
    local missing_tools=()
    
    if ! command -v dpkg-deb &> /dev/null; then
        missing_tools+=("dpkg-deb")
    fi
    
    if ! command -v rpmbuild &> /dev/null; then
        missing_tools+=("rpmbuild")
    fi
    
    if ! command -v appimagetool &> /dev/null; then
        echo -e "${YELLOW}⚠️ appimagetool not found. AppImage creation will be skipped.${NC}"
    fi
    
    if [ ${#missing_tools[@]} -gt 0 ]; then
        echo -e "${YELLOW}⚠️ Missing tools: ${missing_tools[*]}${NC}"
        echo -e "${YELLOW}Installing missing tools...${NC}"
        
        if command -v apt-get &> /dev/null; then
            sudo apt-get update
            sudo apt-get install -y dpkg-dev rpm
        elif command -v dnf &> /dev/null; then
            sudo dnf install -y dpkg-dev rpm-build
        elif command -v pacman &> /dev/null; then
            sudo pacman -S --noconfirm dpkg rpm-tools
        fi
    fi
    
    echo -e "${GREEN}✅ Prerequisites check complete${NC}"
}

# Build the application
build_app() {
    echo -e "${YELLOW}🔨 Building application...${NC}"
    
    # Build with Tauri for Linux
    npm run tauri build -- --target x86_64-unknown-linux-gnu
    
    if [ ! -f "${BUILD_DIR}/${APP_NAME}" ]; then
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
    
    echo -e "${GREEN}✅ Distribution directory ready${NC}"
}

# Create Debian package
create_deb_package() {
    echo -e "${YELLOW}📦 Creating Debian package...${NC}"
    
    DEB_NAME="${APP_NAME}_${VERSION}_amd64.deb"
    DEB_DIR="${DIST_DIR}/deb"
    
    # Create package structure
    mkdir -p "${DEB_DIR}/DEBIAN"
    mkdir -p "${DEB_DIR}/usr/bin"
    mkdir -p "${DEB_DIR}/usr/share/applications"
    mkdir -p "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps"
    mkdir -p "${DEB_DIR}/usr/share/doc/${APP_NAME}"
    mkdir -p "${DEB_DIR}/usr/share/man/man1"
    
    # Copy executable
    cp "${BUILD_DIR}/${APP_NAME}" "${DEB_DIR}/usr/bin/"
    chmod +x "${DEB_DIR}/usr/bin/${APP_NAME}"
    
    # Copy icon
    cp "src-tauri/icons/128x128.png" "${DEB_DIR}/usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png"
    
    # Create desktop file
    cat > "${DEB_DIR}/usr/share/applications/${APP_NAME}.desktop" << EOF
[Desktop Entry]
Name=${APP_DISPLAY_NAME}
Comment=${DESCRIPTION}
Exec=${APP_NAME}
Icon=${APP_NAME}
Terminal=false
Type=Application
Categories=Utility;Accessibility;
Keywords=voice;gesture;control;haptic;ring;
StartupNotify=true
EOF
    
    # Create control file
    cat > "${DEB_DIR}/DEBIAN/control" << EOF
Package: ${APP_NAME}
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6 (>= 2.31), libgcc-s1 (>= 3.0), libgtk-3-0 (>= 3.24), libwebkit2gtk-4.0-37 (>= 2.30), libbluetooth3 (>= 5.50)
Maintainer: ${MAINTAINER}
Description: ${DESCRIPTION}
 Gestura.app is a revolutionary voice and gesture control application that
 transforms how you interact with your computer. Features include:
 .
  * Advanced voice recognition with multiple language support
  * Precise gesture control via Haptic Harmony ring
  * Customizable voice commands and gestures
  * Plugin system for extensibility
  * Privacy-focused local processing
  * Cross-platform compatibility
Homepage: https://gestura.app
EOF
    
    # Create postinst script
    cat > "${DEB_DIR}/DEBIAN/postinst" << 'EOF'
#!/bin/bash
set -e

# Update desktop database
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications
fi

# Update icon cache
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q /usr/share/icons/hicolor
fi

# Add user to audio group if not already
if ! groups "$SUDO_USER" | grep -q audio; then
    usermod -a -G audio "$SUDO_USER" || true
fi

echo "Gestura.app installed successfully!"
echo "You may need to log out and back in for group changes to take effect."
EOF
    
    chmod +x "${DEB_DIR}/DEBIAN/postinst"
    
    # Create postrm script
    cat > "${DEB_DIR}/DEBIAN/postrm" << 'EOF'
#!/bin/bash
set -e

if [ "$1" = "remove" ]; then
    # Update desktop database
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q /usr/share/applications
    fi
    
    # Update icon cache
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q /usr/share/icons/hicolor
    fi
fi
EOF
    
    chmod +x "${DEB_DIR}/DEBIAN/postrm"
    
    # Create copyright file
    cat > "${DEB_DIR}/usr/share/doc/${APP_NAME}/copyright" << EOF
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: ${APP_DISPLAY_NAME}
Upstream-Contact: ${MAINTAINER}
Source: https://gestura.app

Files: *
Copyright: 2024 Gestura AI
License: MIT
 Permission is hereby granted, free of charge, to any person obtaining a copy
 of this software and associated documentation files (the "Software"), to deal
 in the Software without restriction, including without limitation the rights
 to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 copies of the Software, and to permit persons to whom the Software is
 furnished to do so, subject to the following conditions:
 .
 The above copyright notice and this permission notice shall be included in all
 copies or substantial portions of the Software.
 .
 THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 SOFTWARE.
EOF
    
    # Create changelog
    cat > "${DEB_DIR}/usr/share/doc/${APP_NAME}/changelog.Debian.gz" << EOF
${APP_NAME} (${VERSION}) unstable; urgency=medium

  * Initial release of ${APP_DISPLAY_NAME}
  * Voice recognition with multiple language support
  * Gesture control via Haptic Harmony ring
  * Plugin system and customization options

 -- ${MAINTAINER}  $(date -R)
EOF
    
    gzip "${DEB_DIR}/usr/share/doc/${APP_NAME}/changelog.Debian.gz"
    
    # Build package
    dpkg-deb --build "${DEB_DIR}" "${DIST_DIR}/${DEB_NAME}"
    
    # Clean up
    rm -rf "${DEB_DIR}"
    
    if [ ! -f "${DIST_DIR}/${DEB_NAME}" ]; then
        echo -e "${RED}❌ Debian package creation failed${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}✅ Debian package created: ${DEB_NAME}${NC}"
}

# Create RPM package
create_rpm_package() {
    echo -e "${YELLOW}📦 Creating RPM package...${NC}"
    
    RPM_NAME="${APP_NAME}-${VERSION}-1.x86_64.rpm"
    RPM_BUILD_DIR="${DIST_DIR}/rpmbuild"
    
    # Create RPM build structure
    mkdir -p "${RPM_BUILD_DIR}"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
    
    # Create spec file
    cat > "${RPM_BUILD_DIR}/SPECS/${APP_NAME}.spec" << EOF
Name:           ${APP_NAME}
Version:        ${VERSION}
Release:        1%{?dist}
Summary:        ${DESCRIPTION}

License:        MIT
URL:            https://gestura.app
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  gcc
Requires:       glibc >= 2.31, gtk3 >= 3.24, webkit2gtk3 >= 2.30, bluez >= 5.50

%description
Gestura.app is a revolutionary voice and gesture control application that
transforms how you interact with your computer. Features include advanced
voice recognition, precise gesture control via Haptic Harmony ring,
customizable commands, plugin system, and privacy-focused local processing.

%prep
%setup -q

%build
# Binary is pre-built

%install
rm -rf %{buildroot}
mkdir -p %{buildroot}/usr/bin
mkdir -p %{buildroot}/usr/share/applications
mkdir -p %{buildroot}/usr/share/icons/hicolor/256x256/apps

install -m 755 ${APP_NAME} %{buildroot}/usr/bin/
install -m 644 ${APP_NAME}.desktop %{buildroot}/usr/share/applications/
install -m 644 ${APP_NAME}.png %{buildroot}/usr/share/icons/hicolor/256x256/apps/

%files
/usr/bin/${APP_NAME}
/usr/share/applications/${APP_NAME}.desktop
/usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png

%post
/usr/bin/update-desktop-database -q /usr/share/applications || :
/usr/bin/gtk-update-icon-cache -q /usr/share/icons/hicolor || :

%postun
/usr/bin/update-desktop-database -q /usr/share/applications || :
/usr/bin/gtk-update-icon-cache -q /usr/share/icons/hicolor || :

%changelog
* $(date '+%a %b %d %Y') ${MAINTAINER} - ${VERSION}-1
- Initial release
EOF
    
    # Create source tarball
    TARBALL_DIR="${RPM_BUILD_DIR}/SOURCES/${APP_NAME}-${VERSION}"
    mkdir -p "${TARBALL_DIR}"
    
    cp "${BUILD_DIR}/${APP_NAME}" "${TARBALL_DIR}/"
    cp "src-tauri/icons/128x128.png" "${TARBALL_DIR}/${APP_NAME}.png"
    
    # Create desktop file
    cat > "${TARBALL_DIR}/${APP_NAME}.desktop" << EOF
[Desktop Entry]
Name=${APP_DISPLAY_NAME}
Comment=${DESCRIPTION}
Exec=${APP_NAME}
Icon=${APP_NAME}
Terminal=false
Type=Application
Categories=Utility;Accessibility;
Keywords=voice;gesture;control;haptic;ring;
StartupNotify=true
EOF
    
    # Create tarball
    cd "${RPM_BUILD_DIR}/SOURCES"
    tar -czf "${APP_NAME}-${VERSION}.tar.gz" "${APP_NAME}-${VERSION}"
    cd - > /dev/null
    
    # Build RPM
    rpmbuild --define "_topdir ${RPM_BUILD_DIR}" -ba "${RPM_BUILD_DIR}/SPECS/${APP_NAME}.spec"
    
    # Copy RPM to dist directory
    cp "${RPM_BUILD_DIR}/RPMS/x86_64/${RPM_NAME}" "${DIST_DIR}/"
    
    # Clean up
    rm -rf "${RPM_BUILD_DIR}"
    
    if [ ! -f "${DIST_DIR}/${RPM_NAME}" ]; then
        echo -e "${RED}❌ RPM package creation failed${NC}"
        exit 1
    fi
    
    echo -e "${GREEN}✅ RPM package created: ${RPM_NAME}${NC}"
}

# Create AppImage
create_appimage() {
    if ! command -v appimagetool &> /dev/null; then
        echo -e "${YELLOW}⚠️ appimagetool not available, skipping AppImage creation${NC}"
        return 0
    fi
    
    echo -e "${YELLOW}📦 Creating AppImage...${NC}"
    
    APPIMAGE_NAME="${APP_DISPLAY_NAME}-${VERSION}-x86_64.AppImage"
    APPDIR="${DIST_DIR}/${APP_DISPLAY_NAME}.AppDir"
    
    # Create AppDir structure
    mkdir -p "${APPDIR}/usr/bin"
    mkdir -p "${APPDIR}/usr/share/applications"
    mkdir -p "${APPDIR}/usr/share/icons/hicolor/256x256/apps"
    
    # Copy executable
    cp "${BUILD_DIR}/${APP_NAME}" "${APPDIR}/usr/bin/"
    chmod +x "${APPDIR}/usr/bin/${APP_NAME}"
    
    # Copy icon
    cp "src-tauri/icons/128x128.png" "${APPDIR}/usr/share/icons/hicolor/256x256/apps/${APP_NAME}.png"
    cp "src-tauri/icons/128x128.png" "${APPDIR}/${APP_NAME}.png"
    
    # Create desktop file
    cat > "${APPDIR}/${APP_NAME}.desktop" << EOF
[Desktop Entry]
Name=${APP_DISPLAY_NAME}
Comment=${DESCRIPTION}
Exec=${APP_NAME}
Icon=${APP_NAME}
Terminal=false
Type=Application
Categories=Utility;Accessibility;
Keywords=voice;gesture;control;haptic;ring;
StartupNotify=true
EOF
    
    cp "${APPDIR}/${APP_NAME}.desktop" "${APPDIR}/usr/share/applications/"
    
    # Create AppRun script
    cat > "${APPDIR}/AppRun" << 'EOF'
#!/bin/bash
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
export LD_LIBRARY_PATH="${HERE}/usr/lib:${LD_LIBRARY_PATH}"
exec "${HERE}/usr/bin/gestura" "$@"
EOF
    
    chmod +x "${APPDIR}/AppRun"
    
    # Build AppImage
    cd "${DIST_DIR}"
    appimagetool "${APP_DISPLAY_NAME}.AppDir" "${APPIMAGE_NAME}"
    cd - > /dev/null
    
    # Clean up
    rm -rf "${APPDIR}"
    
    if [ ! -f "${DIST_DIR}/${APPIMAGE_NAME}" ]; then
        echo -e "${RED}❌ AppImage creation failed${NC}"
        return 1
    fi
    
    echo -e "${GREEN}✅ AppImage created: ${APPIMAGE_NAME}${NC}"
}

# Generate checksums
generate_checksums() {
    echo -e "${YELLOW}🔢 Generating checksums...${NC}"
    
    cd "${DIST_DIR}"
    
    # Generate SHA256 checksums
    for file in *.deb *.rpm *.AppImage; do
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
Gestura.app v${VERSION} - Linux Release

Build Information:
- Version: ${VERSION}
- Platform: Linux x86_64
- Build Date: $(date -u +"%Y-%m-%d %H:%M:%S UTC")
- Build Host: $(hostname)

Files:
- ${APP_NAME}_${VERSION}_amd64.deb (Debian/Ubuntu Package)
- ${APP_NAME}-${VERSION}-1.x86_64.rpm (Red Hat/Fedora Package)
- ${APP_DISPLAY_NAME}-${VERSION}-x86_64.AppImage (Universal Linux)

Installation:
Debian/Ubuntu:
  sudo dpkg -i ${APP_NAME}_${VERSION}_amd64.deb
  sudo apt-get install -f  # Fix dependencies if needed

Red Hat/Fedora:
  sudo rpm -i ${APP_NAME}-${VERSION}-1.x86_64.rpm
  # Or: sudo dnf install ${APP_NAME}-${VERSION}-1.x86_64.rpm

AppImage:
  chmod +x ${APP_DISPLAY_NAME}-${VERSION}-x86_64.AppImage
  ./${APP_DISPLAY_NAME}-${VERSION}-x86_64.AppImage

System Requirements:
- Linux kernel 4.15+ (Ubuntu 18.04+, Fedora 28+, etc.)
- glibc 2.31+
- GTK 3.24+
- WebKit2GTK 2.30+
- BlueZ 5.50+ for Haptic Harmony Ring
- 4GB RAM minimum, 8GB recommended
- 500MB free disk space

Support:
- Website: https://gestura.app
- Support: support@gestura.app
- Documentation: https://docs.gestura.app
EOF
    
    echo -e "${GREEN}✅ Release information created${NC}"
}

# Main execution
main() {
    echo -e "${BLUE}🚀 Starting Linux packaging process${NC}"
    
    check_prerequisites
    build_app
    setup_dist
    create_deb_package
    create_rpm_package
    create_appimage
    generate_checksums
    create_release_info
    
    echo -e "${GREEN}🎉 Linux packaging complete!${NC}"
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
