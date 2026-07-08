#!/bin/bash
# Deployment automation script for Gestura.app

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_step() {
    echo -e "${BLUE}[STEP]${NC} $1"
}

# Configuration
DEPLOY_ENV=${1:-"staging"}
BUILD_DIR="dist"
TAURI_BUILD_DIR="src-tauri/target/release"

print_status "Starting deployment for environment: $DEPLOY_ENV"

# Validate environment
case $DEPLOY_ENV in
    "staging"|"production"|"development")
        print_status "Valid environment: $DEPLOY_ENV"
        ;;
    *)
        print_error "Invalid environment. Use: staging, production, or development"
        exit 1
        ;;
esac

# Pre-deployment checks
print_step "Running pre-deployment checks"

# Check if Node.js is installed
if ! command -v node &> /dev/null; then
    print_error "Node.js is not installed"
    exit 1
fi

# Check if Rust is installed
if ! command -v cargo &> /dev/null; then
    print_error "Rust/Cargo is not installed"
    exit 1
fi

# Check if npm dependencies are installed
if [ ! -d "node_modules" ]; then
    print_warning "Node modules not found, installing..."
    npm ci
fi

# Environment-specific configuration
case $DEPLOY_ENV in
    "production")
        print_step "Configuring for production deployment"
        export NODE_ENV=production
        export TAURI_ENV=production
        BUILD_FLAGS="--release"
        ;;
    "staging")
        print_step "Configuring for staging deployment"
        export NODE_ENV=staging
        export TAURI_ENV=staging
        BUILD_FLAGS="--release"
        ;;
    "development")
        print_step "Configuring for development deployment"
        export NODE_ENV=development
        export TAURI_ENV=development
        BUILD_FLAGS=""
        ;;
esac

# Clean previous builds
print_step "Cleaning previous builds"
rm -rf $BUILD_DIR
rm -rf $TAURI_BUILD_DIR

# Install dependencies
print_step "Installing dependencies"
npm ci

# Run tests
print_step "Running tests"
cd src-tauri
cargo test --quiet
cd ..

# Build frontend
print_step "Building frontend"
npm run build

# Verify frontend build
if [ ! -d "$BUILD_DIR" ]; then
    print_error "Frontend build failed - $BUILD_DIR not found"
    exit 1
fi

print_status "Frontend build completed successfully"

# Build Tauri application
print_step "Building Tauri application"
cd src-tauri
if [ "$DEPLOY_ENV" = "production" ]; then
    cargo build --release
else
    cargo build $BUILD_FLAGS
fi
cd ..

# Verify Tauri build
BINARY_NAME="gestura"
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    BINARY_NAME="gestura.exe"
fi

BINARY_PATH="$TAURI_BUILD_DIR/$BINARY_NAME"
if [ "$DEPLOY_ENV" != "production" ]; then
    BINARY_PATH="src-tauri/target/debug/$BINARY_NAME"
fi

if [ ! -f "$BINARY_PATH" ]; then
    print_error "Tauri build failed - binary not found at $BINARY_PATH"
    exit 1
fi

print_status "Tauri build completed successfully"

# Create deployment package
print_step "Creating deployment package"
PACKAGE_DIR="deploy-$DEPLOY_ENV-$(date +%Y%m%d-%H%M%S)"
mkdir -p $PACKAGE_DIR

# Copy built files
cp -r $BUILD_DIR $PACKAGE_DIR/frontend
mkdir -p $PACKAGE_DIR/backend
cp $BINARY_PATH $PACKAGE_DIR/backend/
cp README.md $PACKAGE_DIR/
cp CHANGELOG.md $PACKAGE_DIR/
cp TODO.md $PACKAGE_DIR/

# Create deployment manifest
cat > $PACKAGE_DIR/deployment-manifest.json << EOF
{
  "environment": "$DEPLOY_ENV",
  "version": "$(grep '"version"' package.json | cut -d'"' -f4)",
  "build_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "git_commit": "$(git rev-parse HEAD)",
  "git_branch": "$(git branch --show-current)",
  "node_version": "$(node --version)",
  "npm_version": "$(npm --version)",
  "rust_version": "$(rustc --version)",
  "platform": "$OSTYPE"
}
EOF

# Create installation script
cat > $PACKAGE_DIR/install.sh << 'EOF'
#!/bin/bash
# Installation script for Gestura.app

set -e

INSTALL_DIR="/opt/gestura"
BINARY_NAME="gestura"

echo "Installing Gestura.app..."

# Create installation directory
sudo mkdir -p $INSTALL_DIR

# Copy files
sudo cp backend/$BINARY_NAME $INSTALL_DIR/
sudo cp -r frontend $INSTALL_DIR/
sudo cp *.md $INSTALL_DIR/

# Make binary executable
sudo chmod +x $INSTALL_DIR/$BINARY_NAME

# Create symlink for global access
sudo ln -sf $INSTALL_DIR/$BINARY_NAME /usr/local/bin/gestura

echo "Installation completed successfully!"
echo "Run 'gestura' to start the application"
EOF

chmod +x $PACKAGE_DIR/install.sh

# Create archive
print_step "Creating deployment archive"
ARCHIVE_NAME="gestura-$DEPLOY_ENV-$(date +%Y%m%d-%H%M%S).tar.gz"
tar -czf $ARCHIVE_NAME $PACKAGE_DIR

print_status "Deployment package created: $ARCHIVE_NAME"

# Environment-specific deployment actions
case $DEPLOY_ENV in
    "production")
        print_step "Production deployment actions"
        # Here you would typically:
        # - Upload to production servers
        # - Update load balancers
        # - Run database migrations
        # - Send notifications
        print_status "Production deployment would be triggered here"
        ;;
    "staging")
        print_step "Staging deployment actions"
        # Here you would typically:
        # - Deploy to staging servers
        # - Run integration tests
        # - Update staging database
        print_status "Staging deployment would be triggered here"
        ;;
    "development")
        print_step "Development deployment actions"
        # Here you would typically:
        # - Deploy to development environment
        # - Run development-specific tests
        print_status "Development deployment completed locally"
        ;;
esac

# Generate deployment report
print_step "Generating deployment report"
REPORT_FILE="deployment-report-$DEPLOY_ENV-$(date +%Y%m%d-%H%M%S).md"

cat > $REPORT_FILE << EOF
# Deployment Report

## Environment
- **Target**: $DEPLOY_ENV
- **Date**: $(date)
- **Version**: $(grep '"version"' package.json | cut -d'"' -f4)
- **Git Commit**: $(git rev-parse HEAD)
- **Git Branch**: $(git branch --show-current)

## Build Information
- **Node.js**: $(node --version)
- **npm**: $(npm --version)
- **Rust**: $(rustc --version)
- **Platform**: $OSTYPE

## Artifacts
- **Archive**: $ARCHIVE_NAME
- **Package Directory**: $PACKAGE_DIR
- **Binary**: $BINARY_PATH

## Deployment Status
✅ Pre-deployment checks passed
✅ Dependencies installed
✅ Tests passed
✅ Frontend built successfully
✅ Backend built successfully
✅ Deployment package created
✅ Archive generated

## Next Steps
1. Verify the deployment package
2. Test the installation script
3. Deploy to target environment
4. Run post-deployment verification

EOF

print_status "Deployment report generated: $REPORT_FILE"

# Cleanup temporary files
print_step "Cleaning up temporary files"
rm -rf $PACKAGE_DIR

# Final summary
print_status "Deployment preparation completed successfully!"
echo ""
echo "📦 Archive: $ARCHIVE_NAME"
echo "📋 Report: $REPORT_FILE"
echo ""
echo "Next steps:"
echo "1. Test the archive on target environment"
echo "2. Run post-deployment verification"
echo "3. Monitor application health"

# Open deployment report if possible
if command -v open &> /dev/null; then
    open $REPORT_FILE
elif command -v xdg-open &> /dev/null; then
    xdg-open $REPORT_FILE
fi
