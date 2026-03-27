//! macOS application (Dock) icon helper.
//!
//! When the app switches from `Accessory` → `Regular` activation policy, macOS
//! shows the application icon in the Dock. In production `.app` bundles this is
//! resolved automatically from `Info.plist` (`CFBundleIconFile`). In development
//! builds (raw Cargo binary, no bundle) macOS falls back to a generic terminal
//! icon instead.
//!
//! This module explicitly calls `[NSApplication sharedApplication]
//! setApplicationIconImage:` so the correct Gestura icon appears in the Dock
//! in both dev and production environments.
//!
//! **No additional crate dependencies** — we link directly against the system
//! `libobjc.dylib` and `AppKit.framework` that are always present on macOS.

// ── Public surface ────────────────────────────────────────────────────────────

/// Explicitly set the application Dock icon to the bundled Gestura icon.
///
/// On macOS: calls `NSApp.setApplicationIconImage` with the PNG embedded at
/// compile time. Safe to call from any thread after Tauri has initialised the
/// event loop.
///
/// On every other platform: no-op.
pub fn apply_dock_icon() {
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = macos::set_dock_icon() {
            // Non-fatal: the window still opens, just with a generic icon.
            tracing::warn!("Could not set Dock icon: {e}");
        } else {
            tracing::debug!("Dock icon set to Gestura app icon");
        }
    }
}

// ── macOS implementation ──────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};
    use std::mem;

    /// Native macOS icon bytes embedded into the binary at compile time.
    ///
    /// Path is relative to this source file → `../icons/icon.icns`.
    const APP_ICON_ICNS: &[u8] = include_bytes!("../icons/icon.icns");

    /// PNG icon bytes embedded into the binary as a fallback.
    ///
    /// Path is relative to this source file → `../icons/icon.png`.
    const APP_ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

    // Opaque Objective-C object / selector pointer types.
    type Id = *mut c_void;
    type Sel = *mut c_void;

    // Raw symbols from libobjc — always available on macOS.
    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        // objc_msgSend is variadic and is transmuted below for each specific
        // call signature (pointer-returning or void-returning).
        fn objc_msgSend();
    }

    /// Set `[NSApplication sharedApplication].applicationIconImage`.
    ///
    /// Returns an error string on failure (class / object lookup returned nil).
    pub(super) fn set_dock_icon() -> Result<(), &'static str> {
        unsafe {
            // ------------------------------------------------------------------
            // 1. Build NSImage from embedded icon bytes.
            // ------------------------------------------------------------------
            let ns_data_cls = objc_getClass(c"NSData".as_ptr());
            if ns_data_cls.is_null() {
                return Err("NSData class not found");
            }

            let sel_data_bytes = sel_registerName(c"dataWithBytes:length:".as_ptr());

            let ns_image_cls = objc_getClass(c"NSImage".as_ptr());
            if ns_image_cls.is_null() {
                return Err("NSImage class not found");
            }

            let sel_alloc = sel_registerName(c"alloc".as_ptr());
            let sel_init_data = sel_registerName(c"initWithData:".as_ptr());

            let make_ns_data = |bytes: &[u8]| {
                let f: extern "C" fn(Id, Sel, *const u8, usize) -> Id =
                    mem::transmute(objc_msgSend as *const ());
                f(ns_data_cls, sel_data_bytes, bytes.as_ptr(), bytes.len())
            };

            let make_ns_image = |bytes: &[u8]| {
                let ns_data = make_ns_data(bytes);
                if ns_data.is_null() {
                    return std::ptr::null_mut();
                }

                let alloc: Id = {
                    let f: extern "C" fn(Id, Sel) -> Id = mem::transmute(objc_msgSend as *const ());
                    f(ns_image_cls, sel_alloc)
                };
                if alloc.is_null() {
                    return std::ptr::null_mut();
                }

                let f: extern "C" fn(Id, Sel, Id) -> Id = mem::transmute(objc_msgSend as *const ());
                f(alloc, sel_init_data, ns_data)
            };

            // Prefer ICNS so macOS can pick the best icon representation for the
            // current Dock scale. Fall back to the existing PNG if ICNS decoding
            // is unavailable for any reason.
            let ns_image: Id = {
                let ns_image = make_ns_image(APP_ICON_ICNS);
                if ns_image.is_null() {
                    make_ns_image(APP_ICON_PNG)
                } else {
                    ns_image
                }
            };
            if ns_image.is_null() {
                return Err("NSImage initWithData: returned nil for ICNS and PNG");
            }

            // ------------------------------------------------------------------
            // 2. Set the application icon on the shared NSApplication instance.
            // ------------------------------------------------------------------
            let ns_app_cls = objc_getClass(c"NSApplication".as_ptr());
            if ns_app_cls.is_null() {
                return Err("NSApplication class not found");
            }

            let sel_shared = sel_registerName(c"sharedApplication".as_ptr());

            // Signature: (Class, SEL) -> Id
            let ns_app: Id = {
                let f: extern "C" fn(Id, Sel) -> Id = mem::transmute(objc_msgSend as *const ());
                f(ns_app_cls, sel_shared)
            };
            if ns_app.is_null() {
                return Err("[NSApplication sharedApplication] returned nil");
            }

            let sel_set_icon = sel_registerName(c"setApplicationIconImage:".as_ptr());

            // Signature: (Id, SEL, Id) -> void
            {
                let f: extern "C" fn(Id, Sel, Id) = mem::transmute(objc_msgSend as *const ());
                f(ns_app, sel_set_icon, ns_image);
            }

            Ok(())
        }
    }
}
