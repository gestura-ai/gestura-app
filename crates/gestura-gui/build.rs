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

/// Set `TAURI_CONFIG` overrides so builds succeed regardless of whether the
/// ffmpeg sidecar has been staged on disk.
///
/// - **Debug / test builds**: strip *all* `externalBin` entries so the build
///   never requires packaged sidecars.
/// - **Release builds**: keep the gestura CLI sidecar but silently drop the
///   ffmpeg entry when `binaries/ffmpeg-<TARGET>` does not exist.  The runtime
///   resolver in `screen.rs` falls back to system ffmpeg automatically.
///
/// `TAURI_CONFIG` is parsed as **inline JSON** by Tauri (it is not a file path).
fn maybe_set_dev_tauri_config(profile: &str) {
    if profile == "release" {
        maybe_exclude_missing_ffmpeg_release();
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

/// For release builds, check whether the ffmpeg sidecar has been staged for
/// the current `TARGET` triple.  If it is absent, emit a warning and override
/// `TAURI_CONFIG` to remove ffmpeg from `bundle.externalBin` so the build
/// still succeeds.  The gestura CLI sidecar is always kept.
///
/// When the ffmpeg binary IS present (i.e. the packaging script has run), this
/// function returns without touching `TAURI_CONFIG` and `tauri.conf.json` is
/// used verbatim — so ffmpeg gets bundled into the installer as intended.
fn maybe_exclude_missing_ffmpeg_release() {
    // Respect an explicit caller override.
    if std::env::var_os("TAURI_CONFIG").is_some() {
        return;
    }

    let target = std::env::var("TARGET").unwrap_or_default();
    let ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let ffmpeg_path = format!("binaries/ffmpeg-{target}{ext}");

    if std::path::Path::new(&ffmpeg_path).exists() {
        // Binary is staged — use tauri.conf.json as-is.
        return;
    }

    println!(
        "cargo:warning=ffmpeg sidecar not staged at '{ffmpeg_path}'; \
         excluding from installer bundle. Run scripts/package-mac.sh \
         (or the equivalent for your platform) to stage a bundled ffmpeg, \
         or set GESTURA_FFMPEG_PATH at runtime to point to a local binary."
    );

    // Keep gestura in externalBin but remove ffmpeg so the build succeeds.
    // SAFETY: build scripts run in a controlled, single-process context;
    // we only mutate the environment once before tauri_build::build().
    unsafe {
        std::env::set_var(
            "TAURI_CONFIG",
            r#"{"bundle":{"externalBin":["binaries/gestura"]}}"#,
        );
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
