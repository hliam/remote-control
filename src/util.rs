use std::os::windows::prelude::OsStrExt;
use std::{ffi::OsStr, fmt};
use winapi::{shared::windef::HWND, um::winuser};

use image::codecs::png;

/// Gets the window handle of the task bar, if it's running.
fn get_taskbar_hwnd() -> Option<HWND> {
    let name: Vec<_> = OsStr::new("Shell_TrayWnd\0").encode_wide().collect();
    let taskbar_hwnd = unsafe { winuser::FindWindowW(name.as_ptr(), std::ptr::null()) };
    if taskbar_hwnd == std::ptr::null_mut() {
        None
    } else {
        Some(taskbar_hwnd)
    }
}

/// Gets the window handle of the desktop, if it's running.
fn get_desktop_window_hwnd() -> Option<HWND> {
    let hwnd = unsafe { winuser::GetDesktopWindow() };
    if hwnd == std::ptr::null_mut() {
        None
    } else {
        Some(hwnd)
    }
}

/// Puts the computer to sleep.
///
/// Returns `true` if successful, `false` otherwise.
pub fn sleep_computer() -> bool {
    // TODO: this is weird and doesn't work sometimes(?) and it blocks for weirdly long(?) maybe
    // make the system go into modern standby instead?
    // TODO: figure out if this needs a delay (in separate thread) when it works.
    unsafe { winapi::um::powrprof::SetSuspendState(0, 1, 0) == 1 }
}

/// Puts the display to sleep.
///
/// This will silently fail if there is no taskbar process running.
pub fn sleep_display() {
    // TODO: this doesn't work. might just be my computer. either:
    // (1) Dell's dubious implementation--they turn the entire system off instead of just the
    // monitor, or
    // (2) it's some weird modern standby stuff and I broke it on my computer.

    if let Some(hwnd) = get_desktop_window_hwnd() {
        unsafe { winuser::PostMessageW(hwnd, winuser::WM_SYSCOMMAND, winuser::SC_MONITORPOWER, 2) };
    }
}

/// Minimizes all open windows.
///
/// This will silently fail if there is no taskbar process running.
pub fn minimize_windows() {
    unsafe {
        // 419 is the minimizeall message.
        get_taskbar_hwnd().map(|hwnd| winuser::PostMessageW(hwnd, winuser::WM_COMMAND, 419, 0));
    }
}

/// Take a screenshot of the primary display.
///
/// Returns raw bytes of a png.
pub fn take_screenshot() -> Result<Vec<u8>, NoDisplayError> {
    let screens = screenshots::Screen::all().expect("failed to get screens for screenshoting");
    let primary_screen = screens.get(0).ok_or(NoDisplayError)?;
    let bitmap = primary_screen.capture().expect("failed to capture screen");

    let height = primary_screen.display_info.height as usize;
    let width = primary_screen.display_info.width as usize;
    // haphazard testing brought me to this number for the capacity
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
