//! Underlying utilities for interacting with the operating system.
//!
//! This module handles the actual actions of requests made to the server.

use std::os::windows::prelude::OsStrExt;
use std::{ffi::OsStr, fmt};

use screenshots::image::codecs::png;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::{shared::windef::HWND, um::winuser};

/// A handle. This might be null.
struct Hwnd(HWND);

impl Hwnd {
    /// Returns `Some(self)` if the handle isn't null, and `None` if it is.
    #[must_use]
    fn some_non_null(self) -> Option<Self> {
        (!self.0.is_null()).then_some(self)
    }

    /// Posts a message with this handle.
    unsafe fn post_message(self, msg: UINT, w_param: WPARAM, l_param: LPARAM) -> i32 {
        unsafe { winuser::PostMessageW(self.0, msg, w_param, l_param) }
    }
}

/// An error indicated a needed process isn't currently running.
#[derive(Debug, Clone)]
pub struct ProcessNotRunningError {
    /// The (friendly) process name.
    process_name: &'static str,
}

impl ProcessNotRunningError {
    /// Creates a new `ProcessNotRunningError`.
    #[must_use]
    pub const fn new(process_name: &'static str) -> Self {
        Self { process_name }
    }
}

impl fmt::Display for ProcessNotRunningError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} process isn't running", self.process_name)
    }
}

/// Gets the window handle of the task bar, if it's running.
fn get_taskbar_hwnd() -> Result<Hwnd, ProcessNotRunningError> {
    let name: Vec<_> = OsStr::new("Shell_TrayWnd\0").encode_wide().collect();
    unsafe {
        Hwnd(winuser::FindWindowW(name.as_ptr(), std::ptr::null()))
            .some_non_null()
            .ok_or(ProcessNotRunningError::new("taskbar"))
    }
}

/// Gets the window handle of the desktop, if it's running.
fn get_desktop_window_hwnd() -> Result<Hwnd, ProcessNotRunningError> {
    unsafe {
        Hwnd(winuser::GetDesktopWindow())
            .some_non_null()
            .ok_or(ProcessNotRunningError::new("desktop window"))
    }
}

/// Puts the computer to sleep.
///
/// Returns `true` if successful, `false` otherwise.
pub fn sleep_computer() -> bool {
    // TODO: this is should put the computer into modern standby. It doesn't. Fix.
    // TODO: figure out if this needs a delay (in separate thread) when it works. Sleep-sleep does
    //       but maybe modern standby doesn't?
    unsafe { winapi::um::powrprof::SetSuspendState(1, 1, 0) == 1 }
}

/// Puts the display to sleep.
///
/// This will silently fail if there is no taskbar process running.
pub fn sleep_display() -> Result<(), ProcessNotRunningError> {
    // TODO: this doesn't work. might just be my computer.
    get_desktop_window_hwnd().map(|hwnd| unsafe {
        hwnd.post_message(winuser::WM_SYSCOMMAND, winuser::SC_MONITORPOWER, 2);
    })
}

/// Locks the screen.
pub fn lock_the_screen() -> Result<(), LockScreenError> {
    unsafe {
        if winuser::LockWorkStation() == 0 {
            Err(LockScreenError {})
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct LockScreenError;
impl std::error::Error for LockScreenError {}
impl fmt::Display for LockScreenError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("failed to lock the screen")
    }
}

/// Minimizes all open windows.
///
/// This will silently fail if there is no taskbar process running.
pub fn minimize_windows() -> Result<(), ProcessNotRunningError> {
    get_taskbar_hwnd().map(|hwnd| unsafe {
        hwnd.post_message(winuser::WM_COMMAND, 419, 0);
    })
}

/// Take a screenshot of the primary display.
///
/// Returns raw bytes of a png.
pub fn take_screenshot() -> Result<Vec<u8>, NoDisplayError> {
    let screens = screenshots::Screen::all().expect("failed to get screens for screenshoting");
    let primary_screen = screens.first().ok_or(NoDisplayError)?;
    let bitmap = primary_screen.capture().expect("failed to capture screen");

    let height = primary_screen.display_info.height as usize;
    let width = primary_screen.display_info.width as usize;
    // haphazard testing brought me to this estimate for the capacity
    let mut png_buf = Vec::with_capacity(height * width / 5);

    bitmap
        .write_with_encoder(png::PngEncoder::new_with_quality(
            &mut png_buf,
            png::CompressionType::Fast,
            png::FilterType::Adaptive,
        ))
        .expect("error encoding screenshot png");
    Ok(png_buf)
}

/// Occurs when a screenshot was attempted but there was no display.
#[derive(Debug, Copy, Clone)]
pub struct NoDisplayError;
impl std::error::Error for NoDisplayError {}
impl fmt::Display for NoDisplayError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("no display found (to screenshot)")
    }
}
impl From<NoDisplayError> for crate::Response {
    fn from(value: NoDisplayError) -> Self {
        Self::from_message(400, value.to_string())
    }
}
