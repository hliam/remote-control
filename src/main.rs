#![cfg_attr(feature = "no_term", windows_subsystem = "windows")]
mod server;
mod util;

#[cfg(not(debug_assertions))]
use server::DummyLogger;
use server::{Logger, MapResponse, Response, Server};

#[derive(Debug, Copy, Clone)]
struct DebugLogger;

impl server::Logger for DebugLogger {
    fn info(&self, msg: &str) {
        println!("[{} info] {msg}", chrono::Local::now().format("%-H:%M:%S"));
    }
    fn connection_closed(&self, msg: &str) {
        println!(
            "[{} connection closed] {msg}",
            chrono::Local::now().format("%-H:%M:%S")
        );
    }
}

fn main() {
    // We use a dummy logger on release builds.
    #[allow(unreachable_code)]
    #[cfg(debug_assertions)]
    let logger = DebugLogger {};
    #[cfg(not(debug_assertions))]
    let logger = DummyLogger {};

    Server::from_env(logger)
        .expect("key can't be empty")
        .run(&|r| match r.path.as_str() {
            "/minimize" => {
                util::minimize_windows();
                Response::from_status(200)
            }
            "/ping" => Response::from_status(200),
            "/sleep" => {
                util::sleep_computer();
                Response::from_status(200)
            }
            "/sleep_display" => {
                util::sleep_display();
                Response::from_status(200)
            }
            "/screenshot" => util::take_screenshot().into_response(Response::from_png),
            other => {
                logger.connection_closed(&format!("Invalid endpoint requested: \"{other}\""));
                Response::from_status(404)
            }
        })
        .expect("failed to run server");
}
