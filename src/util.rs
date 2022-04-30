use scrap::{Capturer, Display};
use std::io;
use std::io::ErrorKind::WouldBlock;
use std::process;
use std::thread;
use std::time::Duration;
use winapi::um::winuser;

/// Run a function in the background after a 10 millisecond delay (returning before it's ran).
pub fn run_in_background_on_delay<_T>(f: &'static (impl Fn() -> _T + Send + Sync)) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        f();
    });
}

/// Puts the computer to sleep.
///
/// Returns `true` if successful, `false` otherwise.
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
    // TODO: this is incredibly slow. fix.
    process::Command::new("powershell.exe")
        .arg("-command")
        .arg("& { $x = New-Object -ComObject Shell.Application; $x.minimizeall() }")
        .output()
}

/// Take a screenshot of the primary display.
///
/// Returns raw bytes of a png.
pub fn take_screenshot() -> Vec<u8> {
    /// Convert an ARGB buffer to a png.
    #[allow(non_snake_case)]
    fn ARGB_to_png(argb_buf: &[u8], height: usize) -> Vec<u8> {
        // Flip the ARGB bitmap into a BGRA bitmap.
        let bgra_buf: Vec<u8> = argb_buf
            .chunks_exact(4)
            .flat_map(|c| [c[2], c[1], c[0], 255])
            .collect();

        let width = bgra_buf.len() / height / 4;
        // idk how large this buffer should be initialized as. this seems like a safe bet.
        let mut png = Vec::with_capacity(width * height * 4);
        let mut encoder = repng::Options::smallest(width as u32, height as u32)
            .build(&mut png)
            .unwrap();
        encoder.write(&bgra_buf).unwrap();
        encoder.finish().unwrap();
        png
    }

    let one_frame = Duration::from_secs(1) / 60;
    let display = Display::primary().expect("Couldn't find primary display.");
    let mut capturer = Capturer::new(display).expect("Couldn't begin capture.");
    let height = capturer.height();

    // Wait until there's a frame.
    loop {
        match capturer.frame() {
            Err(e) if e.kind() == WouldBlock => thread::sleep(one_frame),
            Err(e) => panic!("Error: {}", e),
            Ok(argb_bitmap) => return ARGB_to_png(&argb_bitmap, height),
        }
    }
}
