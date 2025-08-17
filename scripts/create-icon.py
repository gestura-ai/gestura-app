#!/usr/bin/env python3
"""
Create a simple icon for Gestura.app to prevent build loops
"""

from PIL import Image, ImageDraw, ImageFont
import os

def create_icon():
    # Create a 512x512 icon
    size = 512
    img = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    
    # Draw a simple circular background
    margin = 50
    circle_bbox = [margin, margin, size - margin, size - margin]
    draw.ellipse(circle_bbox, fill=(64, 120, 192, 255), outline=(32, 80, 160, 255), width=8)
    
    # Try to add text
    try:
        # Try to use a system font
        font_size = 120
        try:
            font = ImageFont.truetype("/System/Library/Fonts/Arial.ttf", font_size)
        except:
            try:
                font = ImageFont.truetype("arial.ttf", font_size)
            except:
                font = ImageFont.load_default()
        
        # Draw "G" for Gestura
        text = "G"
        bbox = draw.textbbox((0, 0), text, font=font)
        text_width = bbox[2] - bbox[0]
        text_height = bbox[3] - bbox[1]
        
        x = (size - text_width) // 2
        y = (size - text_height) // 2 - 20
        
        draw.text((x, y), text, fill=(255, 255, 255, 255), font=font)
        
    except Exception as e:
        print(f"Could not add text: {e}")
        # Just draw a simple shape instead
        center = size // 2
        draw.ellipse([center-60, center-60, center+60, center+60], fill=(255, 255, 255, 255))
    
    return img

def main():
    # Create the icon
    icon = create_icon()
    
    # Save in multiple sizes for different platforms
    sizes = [16, 32, 64, 128, 256, 512]
    
    icon_dir = "src-tauri/icons"
    os.makedirs(icon_dir, exist_ok=True)
    
    for size in sizes:
        resized = icon.resize((size, size), Image.Resampling.LANCZOS)
        resized.save(f"{icon_dir}/{size}x{size}.png")
        print(f"Created {size}x{size}.png")
    
    # Save the main icon
    icon.save(f"{icon_dir}/icon.png")
    print("Created icon.png")
    
    # Create ICO for Windows
    try:
        icon.save(f"{icon_dir}/icon.ico", format='ICO', sizes=[(16,16), (32,32), (48,48), (64,64), (128,128), (256,256)])
        print("Created icon.ico")
    except Exception as e:
        print(f"Could not create ICO: {e}")
    
    print("Icon creation complete!")

if __name__ == "__main__":
    main()
