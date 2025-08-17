fn main() {
    // Only ensure icon exists during release builds to prevent dev rebuild loops
    if std::env::var("PROFILE").unwrap_or_default() == "release" {
        ensure_default_icon();
    }
    tauri_build::build()
}

fn ensure_default_icon() {
    use std::fs;
    use std::path::Path;

    let icon_dir = Path::new("icons");
    let icon_path = icon_dir.join("icon.png");

    // Only create the icon if it doesn't exist to prevent rebuild loops
    if icon_path.exists() {
        return;
    }

    let _ = fs::create_dir_all(icon_dir);

    // Generate a 1x1 transparent RGBA PNG using the `image` crate (build-dep)
    // This guarantees a valid RGBA PNG acceptable by tauri-build
    if let Err(err) = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut img = image::RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        img.save(&icon_path)?;
        // Removed cargo:rerun-if-changed to prevent rebuild loops
        println!("cargo:warning=Generated default icon at {}", icon_path.display());
        Ok(())
    })() {
        println!("cargo:warning=failed to generate default RGBA icon: {}", err);
    }
}

