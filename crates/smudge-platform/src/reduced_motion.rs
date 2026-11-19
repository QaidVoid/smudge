//! The system's reduced-motion setting on Windows and macOS, read on demand.
//!
//! Neither system's change notice reaches the window through the windowing
//! library, so the window reads the setting when it starts and again each
//! time it regains the focus, which is when a user who changed it in the
//! system settings comes back.

/// Whether the system asks for reduced motion, or `None` when the system
/// does not answer.
///
/// Windows answers through its client area animation setting, which the
/// accessibility settings turn off to reduce motion.
#[cfg(target_os = "windows")]
pub(crate) fn reduced() -> Option<bool> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    use windows_sys::core::BOOL;

    let mut animated: BOOL = 0;
    // SAFETY: SPI_GETCLIENTAREAANIMATION reads one BOOL into the memory the
    // third argument points at, which is `animated`, alive and writable for
    // the call. The second argument must be zero for this action, and the
    // flags are zero because a read updates no profile.
    let answered = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animated as *mut BOOL).cast(),
            0,
        )
    };
    (answered != 0).then_some(animated == 0)
}

/// Whether the system asks for reduced motion, as the accessibility
/// display settings hold it.
#[cfg(target_os = "macos")]
pub(crate) fn reduced() -> Option<bool> {
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    Some(workspace.accessibilityDisplayShouldReduceMotion())
}
