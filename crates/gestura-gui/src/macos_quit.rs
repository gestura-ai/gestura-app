//! macOS-native quit interception for tray-first behavior.
//!
//! Dock `Quit` and `Cmd+Q` dispatch the native `terminate:` action on
//! `NSApplication`, which is distinct from Tauri's last-window `ExitRequested`
//! path. This module intercepts that action so Gestura can close managed windows
//! while keeping the tray process alive unless the app explicitly requested exit.

/// Install the macOS native quit interceptor.
///
/// On non-macOS platforms this is a no-op.
pub fn install_quit_interceptor() {
    #[cfg(target_os = "macos")]
    {
        if let Err(error) = macos::install_quit_interceptor() {
            tracing::warn!(%error, "Failed to install macOS native quit interceptor");
        } else {
            tracing::debug!("Installed macOS native quit interceptor");
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};
    use std::mem;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicPtr, Ordering};

    type Id = *mut c_void;
    type Sel = *mut c_void;
    type Method = *mut c_void;
    type Imp = *mut c_void;

    static ORIGINAL_TERMINATE_IMP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static INSTALL_RESULT: OnceLock<Result<(), &'static str>> = OnceLock::new();

    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn class_getInstanceMethod(cls: Id, name: Sel) -> Method;
        fn method_setImplementation(method: Method, imp: Imp) -> Imp;
    }

    pub(super) fn install_quit_interceptor() -> Result<(), &'static str> {
        *INSTALL_RESULT.get_or_init(|| unsafe { install_quit_interceptor_inner() })
    }

    unsafe fn install_quit_interceptor_inner() -> Result<(), &'static str> {
        let ns_app_cls = unsafe { objc_getClass(c"NSApplication".as_ptr()) };
        if ns_app_cls.is_null() {
            return Err("NSApplication class not found");
        }

        let sel_terminate = unsafe { sel_registerName(c"terminate:".as_ptr()) };
        let terminate_method = unsafe { class_getInstanceMethod(ns_app_cls, sel_terminate) };
        if terminate_method.is_null() {
            return Err("NSApplication terminate: method not found");
        }

        let original =
            unsafe { method_setImplementation(terminate_method, gestura_terminate as Imp) };
        if original.is_null() {
            return Err("Failed to capture original NSApplication terminate: implementation");
        }

        ORIGINAL_TERMINATE_IMP.store(original, Ordering::SeqCst);
        Ok(())
    }

    unsafe extern "C" fn gestura_terminate(this: Id, cmd: Sel, sender: Id) {
        if crate::app_lifecycle::is_exit_requested() || !crate::tray::is_tray_running() {
            call_original_terminate(this, cmd, sender);
            return;
        }

        tracing::info!(
            "Intercepted macOS terminate: closing managed windows and keeping tray alive"
        );

        if let Some(manager) = crate::window_manager::get_window_manager() {
            manager.close_all_windows();
        } else {
            tracing::warn!("Window manager unavailable while intercepting macOS terminate request");
        }
    }

    fn call_original_terminate(this: Id, cmd: Sel, sender: Id) {
        let original = ORIGINAL_TERMINATE_IMP.load(Ordering::SeqCst);
        if original.is_null() {
            tracing::warn!("Original NSApplication terminate: implementation missing");
            return;
        }

        unsafe {
            let function: extern "C" fn(Id, Sel, Id) = mem::transmute(original);
            function(this, cmd, sender);
        }
    }
}
