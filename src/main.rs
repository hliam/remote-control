#![cfg_attr(feature = "no_term", windows_subsystem = "windows")]
mod server;
mod util;

use std::error;
use std::fmt::Display;
use std::io;

use server::{Logger, MapResponse, Response, ResultExt, Server};

// TODO: add encryption and better (encryption-bases) authentication

fn main() {
    let err = run().unwrap_err();

    match err.downcast_ref::<io::Error>() {
        Some(io_err) if io_err.kind() == io::ErrorKind::AddrInUse => {
            eprintln!("Error: server socket is already in use--is another instance running?");
        }
        _ => eprintln!("Error: {err}"),
    }
}

/// Run the server.
fn run() -> Result<(), Box<dyn error::Error>> {
    // We use a dummy logger on release builds.
    #[allow(unreachable_code)]
    #[cfg(debug_assertions)]
    let logger = DebugLogger {};
    #[cfg(not(debug_assertions))]
    let logger = ();

    // TODO: clean up the results and errors here
    Server::from_config_file(logger)?
        .run(|r| match r.path.as_str() {
            "/minimize" => util::minimize_windows()
                .inspect_err(|e| logger.server_error(&format!("Failed to minimize windows: {e}")))
                .to_status_response(500),

            "/lock_screen" => util::lock_the_screen()
                .inspect_err(|e| logger.server_error(e))
                .to_status_response(500),

            "/ping" => Response::from_status(200),

            "/sleep" => {
                util::sleep_computer();
                Response::from_status(200)
            }

            "/sleep_display" => util::sleep_display()
                .inspect_err(|e| {
                    logger.server_error(&format!("Failed to sleep display: {e}"));
                })
                .to_status_response(500),

            "/screenshot" => util::take_screenshot().into_response(Response::from_png),

            other => {
                logger.connection_refused(&format!("Invalid endpoint requested: \"{other}\""));
                Response::from_status(404)
            }
        })
        .map_err(Into::into)
}

// We allow dead code because there'll be warnings when compiling in release mode otherwise.
#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
struct DebugLogger;

impl DebugLogger {
    /// Prints a log entry.
    ///
    /// For example: `[11:30:15 connection refused] key is invalid`
    #[allow(dead_code)]
    fn print(title: &str, msg: &impl Display, color: Color) {
        println!(
            "[\x1b[{}m{} {title}\x1b[0m] {msg}",
            color.ansii_code(),
            chrono::Local::now().format("%-H:%M:%S")
        );
    }
}

impl server::Logger for DebugLogger {
    fn started_listening(&self, sock_addr: std::net::SocketAddrV4) {
        Self::print(
            "started listening",
            &format!("on {}", sock_addr),
            Color::BrightGreen,
        );
    }
    fn got_connection(&self, from: std::net::SocketAddr, to_path: &str) {
        Self::print(
            "got connection",
            &format!("from {from} to {to_path}"),
            Color::Blue,
        );
    }
    fn info(&self, msg: &impl Display) {
        Self::print("info", msg, Color::Blue);
    }
    fn connection_refused(&self, msg: &impl Display) {
        Self::print("connection refused", msg, Color::Red);
    }
    fn server_error(&self, msg: &impl Display) {
        Self::print("server error", msg, Color::BrightRed);
    }
}

#[allow(dead_code)]
enum Color {
    Blue,
    BrightGreen,
    BrightRed,
    Red,
}
impl Color {
    fn ansii_code(&self) -> &str {
        match self {
            Self::Blue => "34",
            Self::BrightGreen => "92",
            Self::BrightRed => "91",
            Self::Red => "31",
        }
    }
}
