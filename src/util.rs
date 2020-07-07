use std::io;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
use std::process;

#[cfg(target_os = "macos")]
use lazy_static::lazy_static;
#[cfg(target_os = "windows")]
use winapi::um::winuser;

#[cfg(target_os = "macos")]
lazy_static! {
    static ref MACOS_MINIMIZE_WINDOWS_SCRIPT_PATH: PathBuf = std::env::current_exe()
        .expect("failed to get current exe")
        .parent()
        .unwrap_or(Path::new("/"))
        .join("macos_minimize_windows.scpt");
}

/// Puts the computer to sleep.
#[cfg(target_os = "windows")]
pub fn sleep_computer() -> bool {
    unsafe { winapi::um::powrprof::SetSuspendState(0, 1, 0) == 1 }
}

/// Puts the computer to sleep.
#[cfg(target_os = "macos")]
pub fn sleep_computer() -> bool {
    process::Command::new("pmset")
        .arg("sleepnow")
        .output()
        .is_ok()
}

/// Puts the display to sleep.
#[cfg(target_os = "windows")]
pub fn sleep_display() {
    unsafe {
        winuser::SendMessageA(
            winuser::HWND_BROADCAST,
            winuser::WM_SYSCOMMAND,
            winuser::SC_MONITORPOWER,
            2,
        )
    };
}

/// Puts the display to sleep.
#[cfg(target_os = "macos")]
pub fn sleep_display() {
    process::Command::new("pmset")
        .arg("displaysleepnow")
        .output()
        .expect("failed to sleep display");
}

/// Minimizes all open windows.
#[cfg(target_os = "windows")]
pub fn minimize_windows() -> io::Result<process::Output> {
    process::Command::new("powershell.exe")
        .arg("-command")
        .arg("& { $x = New-Object -ComObject Shell.Application; $x.minimizeall() }")
        .output()
}

/// Minimizes all open windows.
#[cfg(target_os = "macos")]
pub fn minimize_windows() -> io::Result<process::Output> {
    println!("{}", MACOS_MINIMIZE_WINDOWS_SCRIPT_PATH.display());
    process::Command::new("osascript")
        .arg(&*MACOS_MINIMIZE_WINDOWS_SCRIPT_PATH)
        .output()
}
