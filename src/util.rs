use std::io;
use std::process;
use winapi::um::winuser;

/// Puts the computer to sleep.
pub fn sleep_computer() -> bool {
    unsafe { winapi::um::powrprof::SetSuspendState(0, 1, 0) == 1 }
}

/// Puts the display to sleep.
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

/// Minimizes all open windows.
pub fn minimize_windows() -> io::Result<process::Output> {
    process::Command::new("powershell.exe")
        .arg("-command")
        .arg("& { $x = New-Object -ComObject Shell.Application; $x.minimizeall() }")
        .output()
}
