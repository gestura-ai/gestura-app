#!/bin/bash
# Automated release script for Gestura.app

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
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

# Check if version is provided
if [ $# -eq 0 ]; then
    print_error "Usage: $0 <version>"
    print_error "Example: $0 1.0.0"
    exit 1
fi

VERSION=$1
TAG="v$VERSION"

# Validate version format
if ! [[ $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    print_error "Invalid version format. Use semantic versioning (e.g., 1.0.0)"
    exit 1
fi

print_status "Preparing release $TAG"

# Check if we're on main branch
CURRENT_BRANCH=$(git branch --show-current)
if [ "$CURRENT_BRANCH" != "main" ]; then
    print_warning "You are not on the main branch (current: $CURRENT_BRANCH)"
    read -p "Continue anyway? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Check for uncommitted changes
if ! git diff-index --quiet HEAD --; then
    print_error "You have uncommitted changes. Please commit or stash them first."
    exit 1
fi

# Update all version sources via the Just recipe (workspace Cargo.toml → tauri.conf.json → package.json)
print_status "Updating version across workspace to $VERSION"
just set-version "$VERSION"

# Run tests
print_status "Running tests"
cargo test --workspace --quiet

# Build frontend
print_status "Building frontend"
(cd crates/gestura-gui/frontend && npm run build)

# Build Tauri app for testing
print_status "Building Tauri app"
cargo build -p gestura-gui --release

# Create changelog entry
print_status "Creating changelog entry"
CHANGELOG_FILE="CHANGELOG.md"
if [ ! -f "$CHANGELOG_FILE" ]; then
    echo "# Changelog" > $CHANGELOG_FILE
    echo "" >> $CHANGELOG_FILE
fi

# Add new version to changelog
TEMP_FILE=$(mktemp)
echo "## [$VERSION] - $(date +%Y-%m-%d)" > $TEMP_FILE
echo "" >> $TEMP_FILE
echo "### Added" >> $TEMP_FILE
echo "- New features and improvements" >> $TEMP_FILE
echo "" >> $TEMP_FILE
echo "### Changed" >> $TEMP_FILE
echo "- Updates and modifications" >> $TEMP_FILE
echo "" >> $TEMP_FILE
echo "### Fixed" >> $TEMP_FILE
echo "- Bug fixes and corrections" >> $TEMP_FILE
echo "" >> $TEMP_FILE
cat $CHANGELOG_FILE >> $TEMP_FILE
mv $TEMP_FILE $CHANGELOG_FILE

print_warning "Please edit $CHANGELOG_FILE to add specific changes for this release"
read -p "Press Enter to continue after editing the changelog..."

# Commit changes
print_status "Committing version changes"
git add .
git commit -m "chore: bump version to $VERSION"

# Create and push tag
print_status "Creating and pushing tag $TAG"
git tag -a $TAG -m "Release $TAG"
git push origin main
git push origin $TAG

print_status "Release $TAG has been created and pushed!"
print_status "GitHub Actions will now build and publish the release."
print_status "Monitor the progress at: https://github.com/$(git config --get remote.origin.url | sed 's/.*github.com[:/]\([^.]*\).*/\1/')/actions"

# Open release page
if command -v open &> /dev/null; then
    REPO_URL=$(git config --get remote.origin.url | sed 's/git@github.com:/https:\/\/github.com\//' | sed 's/\.git$//')
    open "$REPO_URL/releases/tag/$TAG"
elif command -v xdg-open &> /dev/null; then
    REPO_URL=$(git config --get remote.origin.url | sed 's/git@github.com:/https:\/\/github.com\//' | sed 's/\.git$//')
    xdg-open "$REPO_URL/releases/tag/$TAG"
fi

print_status "Release process completed successfully!"
