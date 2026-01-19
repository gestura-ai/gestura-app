#!/bin/bash
# Generate tray icons from system-tray.svg
# Creates black and white variants at 1x (22px) and 2x (44px) sizes

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ICONS_DIR="$PROJECT_ROOT/crates/gestura-gui/icons"
TRAY_DIR="$ICONS_DIR/tray"
SVG_FILE="$ICONS_DIR/system-tray.svg"

# Check if SVG exists
if [ ! -f "$SVG_FILE" ]; then
    echo "Error: $SVG_FILE not found"
    exit 1
fi

# Create tray directory if it doesn't exist
mkdir -p "$TRAY_DIR"

echo "Generating tray icons from $SVG_FILE..."

# Function to generate icon with specific color
generate_icon() {
    local color=$1
    local size=$2
    local suffix=$3
    local output="$TRAY_DIR/icon-${color}@${suffix}.png"
    
    echo "  Creating $output (${size}x${size})..."
    
    if command -v inkscape &> /dev/null; then
        # Use Inkscape for SVG to PNG conversion
        if [ "$color" = "white" ]; then
            # For white icons, we need to invert the colors
            # First export to temp file, then invert
            local temp_file=$(mktemp).png
            inkscape "$SVG_FILE" --export-filename="$temp_file" \
                --export-width=$size --export-height=$size \
                --export-background-opacity=0 2>/dev/null
            
            # Invert colors using ImageMagick
            if command -v convert &> /dev/null; then
                convert "$temp_file" -negate -channel RGB -negate "$output"
            else
                # Fallback: just copy the file
                cp "$temp_file" "$output"
            fi
            rm -f "$temp_file"
        else
            # Black icon - export directly
            inkscape "$SVG_FILE" --export-filename="$output" \
                --export-width=$size --export-height=$size \
                --export-background-opacity=0 2>/dev/null
        fi
    elif command -v convert &> /dev/null; then
        # Fallback to ImageMagick
        if [ "$color" = "white" ]; then
            convert -background none -density 300 "$SVG_FILE" \
                -resize ${size}x${size} -negate -channel RGB -negate "$output"
        else
            convert -background none -density 300 "$SVG_FILE" \
                -resize ${size}x${size} "$output"
        fi
    else
        echo "Error: Neither inkscape nor convert (ImageMagick) found"
        exit 1
    fi
}

# Generate all variants
# macOS menu bar icons are typically 22x22 (1x) and 44x44 (2x)
generate_icon "black" 22 "1x"
generate_icon "black" 44 "2x"
generate_icon "white" 22 "1x"
generate_icon "white" 44 "2x"

echo ""
echo "Tray icons generated successfully!"
echo "Files created in: $TRAY_DIR"
ls -la "$TRAY_DIR"
