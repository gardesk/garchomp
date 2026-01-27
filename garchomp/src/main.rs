//! garchomp - X11 compositor for gar desktop environment.

mod compositor;
mod ipc;
mod render;
mod x11;

use anyhow::{Context, Result};
use clap::Parser;
use garchomp_ipc::{Request, Response, WindowInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use x11rb::connection::Connection as _;

#[derive(Parser)]
#[command(name = "garchomp", about = "X11 compositor for gar desktop environment")]
struct Cli {
    /// X11 display to connect to.
    #[arg(short, long, env = "DISPLAY")]
    display: Option<String>,

    /// Path to configuration file.
    #[arg(short, long, default_value = "~/.config/gar/init.lua")]
    config: String,

    /// Increase logging verbosity.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Disable HDR support.
    #[arg(long)]
    no_hdr: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    init_logging(cli.verbose);

    tracing::info!("garchomp compositor starting");

    // Set up signal handling
    let running = Arc::new(AtomicBool::new(true));
    setup_signal_handlers(running.clone())?;

    // Create compositor
    let mut compositor =
        compositor::Compositor::new().context("Failed to initialize compositor")?;

    // Create IPC server
    let mut ipc_server = ipc::IpcServer::new().context("Failed to start IPC server")?;

    tracing::info!(
        "Compositor initialized, tracking {} windows",
        compositor.windows.len()
    );

    // Main event loop
    while running.load(Ordering::Relaxed) && compositor.running {
        // Handle IPC requests (non-blocking)
        while let Some(request) = ipc_server.poll() {
            handle_ipc_request(&mut compositor, request);
        }

        // Handle X11 events (non-blocking poll)
        if let Ok(Some(event)) = compositor.conn.conn.poll_for_event() {
            if let Err(e) = compositor.handle_event(event) {
                tracing::error!("Error handling event: {}", e);
            }
        }

        // Small sleep to prevent busy-waiting
        // TODO: Use proper event-driven approach with poll/select
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    tracing::info!("Shutting down");
    compositor.shutdown()?;

    Ok(())
}

fn init_logging(verbosity: u8) {
    let filter = match verbosity {
        0 => "garchomp=info",
        1 => "garchomp=debug",
        _ => "garchomp=trace",
    };

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn setup_signal_handlers(running: Arc<AtomicBool>) -> Result<()> {
    ctrlc::set_handler(move || {
        tracing::info!("Received shutdown signal");
        running.store(false, Ordering::Relaxed);
    })
    .context("Failed to set signal handler")?;

    Ok(())
}

fn handle_ipc_request(compositor: &mut compositor::Compositor, request: ipc::ClientRequest) {
    let response = match &request.request {
        Request::Ping => Response::Pong,
        Request::Reload => {
            tracing::info!("Config reload requested");
            // TODO: Implement config reload
            Response::Ok
        }
        Request::SetEffect { effect, enabled } => {
            tracing::info!("Set effect {} = {}", effect, enabled);
            // TODO: Implement effect toggle
            Response::Ok
        }
        Request::SetBlurStrength { strength } => {
            tracing::info!("Set blur strength = {}", strength);
            // TODO: Implement blur strength
            Response::Ok
        }
        Request::GetWindowInfo { window } => {
            if let Some(tracked) = compositor.windows.get(&(*window as u32)) {
                Response::WindowInfo(WindowInfo {
                    id: tracked.id,
                    x: tracked.x,
                    y: tracked.y,
                    width: tracked.width,
                    height: tracked.height,
                    mapped: tracked.mapped,
                    override_redirect: tracked.override_redirect,
                })
            } else {
                Response::Error {
                    message: "Window not found".into(),
                }
            }
        }
        Request::ListWindows => {
            let windows: Vec<WindowInfo> = compositor
                .windows
                .values()
                .map(|w| WindowInfo {
                    id: w.id,
                    x: w.x,
                    y: w.y,
                    width: w.width,
                    height: w.height,
                    mapped: w.mapped,
                    override_redirect: w.override_redirect,
                })
                .collect();
            Response::WindowList { windows }
        }
    };

    if let Err(e) = request.respond(response) {
        tracing::warn!("Failed to send IPC response: {}", e);
    }
}
