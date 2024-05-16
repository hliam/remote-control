//! TODO: delete this doc
#![cfg_attr(feature = "no_term", windows_subsystem = "windows")]
mod server;
mod util;

use std::error;
use std::io;

#[cfg(not(debug_assertions))]
use server::DummyLogger;
use server::{Logger, MapResponse, Response, ResultExt, Server};

// TODO: document modules

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
    let logger = DummyLogger {};

    Server::from_config_file(logger)?
        .run(|r| match r.path.as_str() {
            "/minimize" => util::minimize_windows()
                .inspect_err(|e| logger.server_error(&format!("Failed to minimize windows; {e}")))
                .to_status_response(500),

            "/ping" => Response::from_status(200),

            "/sleep" => {
                util::sleep_computer();
                Response::from_status(200)
            }

            "/sleep_display" => util::sleep_display()
                .inspect_err(|e| {
                    logger.server_error(&format!("Failed to sleep display; {e}"));
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

#[derive(Debug, Copy, Clone)]
struct DebugLogger;

impl DebugLogger {
    /// Prints a log entry.
    ///
    /// For example: `[11:30:15 connection refused] key is invalid`
    fn print(title: &str, msg: &str) {
        println!(
            "[{} {title}] {msg}",
            chrono::Local::now().format("%-H:%M:%S")
        );
    }
}

// TODO: add colored log output
impl server::Logger for DebugLogger {
    fn info(&self, msg: &str) {
        Self::print("info", msg);
    }
    fn connection_refused(&self, msg: &str) {
        Self::print("connection refused", msg);
    }
    fn server_error(&self, msg: &str) {
        Self::print("server error", msg);
    }
}
