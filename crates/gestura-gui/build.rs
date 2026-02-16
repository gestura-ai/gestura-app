/// Build script for `gestura-gui`.
///
/// Notes:
/// - We set a minimum macOS deployment target for native dependencies.
/// - In non-release builds we set a **minimal inline** `TAURI_CONFIG` override
///   (Tauri expects JSON here, not a file path) that disables `bundle.externalBin`.
///   This prevents debug/test builds from requiring packaged CLI sidecars to
///   exist on disk (those are staged by packaging/CI).
fn main() {
    // Set minimum macOS deployment target for C/C++ dependencies (whisper.cpp requires 10.15+)
    // This is needed because whisper.cpp uses std::filesystem which requires macOS 10.15+
    if std::env::var("MACOSX_DEPLOYMENT_TARGET").is_err() {
        println!("cargo:rustc-env=MACOSX_DEPLOYMENT_TARGET=10.15");
    }

    // Expose build date as a compile-time env var for runtime version reporting.
    set_build_metadata();

    let profile = std::env::var("PROFILE").unwrap_or_default();

    // Only ensure icon exists during release builds to prevent dev rebuild loops.
    if profile == "release" {
        ensure_default_icon();
    }

    maybe_set_dev_tauri_config(&profile);
    tauri_build::build()
}

/// In debug/test builds, set an inline `TAURI_CONFIG` override when the user
/// did not explicitly set one.
///
/// `TAURI_CONFIG` is parsed as **inline JSON** by Tauri (it is not a config
/// file path). We use it to remove `bundle.externalBin` in non-release builds
/// so workspace builds can succeed without packaging artifacts.
fn maybe_set_dev_tauri_config(profile: &str) {
    if profile == "release" {
        return;
    }

    // Respect an explicit override.
    if std::env::var_os("TAURI_CONFIG").is_some() {
        return;
    }

    // Remove `bundle.externalBin` so debug/test builds don't fail if the CLI
    // sidecar isn't staged.
    //
    // Note: setting to an empty list keeps the schema shape intact.
    const DEV_TAURI_CONFIG_OVERRIDE_JSON: &str = r#"{"bundle":{"externalBin":[]}}"#;

    // SAFETY: In Rust 2024, mutating the process environment is `unsafe`.
    // Build scripts execute in a controlled, single-process build context;
    // we only set this variable once before invoking `tauri_build::build()`.
    unsafe {
        std::env::set_var("TAURI_CONFIG", DEV_TAURI_CONFIG_OVERRIDE_JSON);
    }
}

/// Ensure a minimal default icon exists.
///
/// This is only invoked in release builds to avoid triggering rebuild loops
/// during development.
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
        println!(
            "cargo:warning=Generated default icon at {}",
            icon_path.display()
        );
        Ok(())
    })() {
        println!(
            "cargo:warning=failed to generate default RGBA icon: {}",
            err
        );
    }
}

/// Expose build metadata as compile-time environment variables.
///
/// Sets:
/// - `GESTURA_BUILD_DATE` — ISO-8601 date (e.g. `2026-02-16`)
fn set_build_metadata() {
    use chrono::Utc;
    let date = Utc::now().format("%Y-%m-%d").to_string();
    println!("cargo:rustc-env=GESTURA_BUILD_DATE={date}");
}
