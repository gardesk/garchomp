//! garchomp - X11 compositor for gar desktop environment.

mod compositor;
mod config;
mod ipc;
mod render;
mod x11;

use anyhow::{Context, Result};
use clap::Parser;
use garchomp_ipc::{Request, Response, WindowInfo};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use std::os::unix::io::BorrowedFd;
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    init_logging(cli.verbose);

    tracing::info!("garchomp compositor starting");

    // Set up signal handling
    let running = Arc::new(AtomicBool::new(true));
    setup_signal_handlers(running.clone())?;

    // Create compositor (async for GPU initialization)
    let mut compositor = compositor::Compositor::new()
        .await
        .context("Failed to initialize compositor")?;

    // Create IPC server
    let mut ipc_server = ipc::IpcServer::new().context("Failed to start IPC server")?;

    tracing::info!(
        "Compositor initialized, tracking {} windows",
        compositor.windows.len()
    );

    // Get file descriptors for polling
    let x11_fd = compositor.conn.as_raw_fd();
    let ipc_fd = ipc_server.as_raw_fd();

    // Main event loop with proper polling
    while running.load(Ordering::Relaxed) && compositor.running {
        // Set up poll fds
        // SAFETY: We know these fds are valid for the duration of this loop iteration
        let mut poll_fds = [
            PollFd::new(unsafe { BorrowedFd::borrow_raw(x11_fd) }, PollFlags::POLLIN),
            PollFd::new(unsafe { BorrowedFd::borrow_raw(ipc_fd) }, PollFlags::POLLIN),
        ];

        // Wait for events (16ms timeout for ~60fps rendering, or immediate if redraw needed)
        let timeout = if compositor.needs_redraw() {
            PollTimeout::ZERO
        } else {
            PollTimeout::try_from(16).unwrap_or(PollTimeout::ZERO)
        };
        let _ = poll(&mut poll_fds, timeout);

        // Handle X11 events
        while let Ok(Some(event)) = compositor.conn.conn.poll_for_event() {
            if let Err(e) = compositor.handle_event(event) {
                tracing::error!("Error handling event: {}", e);
            }
        }

        // Handle IPC requests
        while let Some(request) = ipc_server.poll() {
            handle_ipc_request(&mut compositor, request);
        }

        // Render if needed
        if compositor.needs_redraw() {
            if let Err(e) = compositor.render() {
                tracing::error!("Render error: {}", e);
            }
        }
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
                let focused = compositor.is_window_focused(tracked.id);
                Response::WindowInfo(WindowInfo {
                    id: tracked.id,
                    x: tracked.x,
                    y: tracked.y,
                    width: tracked.width,
                    height: tracked.height,
                    mapped: tracked.mapped,
                    override_redirect: tracked.override_redirect,
                    workspace: compositor.workspaces.get_window_workspace(tracked.id),
                    focused,
                    fullscreen: false, // TODO: track fullscreen state
                    class: None,       // TODO: get WM_CLASS
                    title: None,       // TODO: get _NET_WM_NAME
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
                .map(|w| {
                    let focused = compositor.is_window_focused(w.id);
                    WindowInfo {
                        id: w.id,
                        x: w.x,
                        y: w.y,
                        width: w.width,
                        height: w.height,
                        mapped: w.mapped,
                        override_redirect: w.override_redirect,
                        workspace: compositor.workspaces.get_window_workspace(w.id),
                        focused,
                        fullscreen: false,
                        class: None,
                        title: None,
                    }
                })
                .collect();
            Response::WindowList { windows }
        }
        Request::Version { version } => {
            tracing::debug!("Client version: {}", version);
            Response::Version {
                version: garchomp_ipc::PROTOCOL_VERSION,
                name: "garchomp".to_string(),
            }
        }
        Request::Status => {
            Response::Status(garchomp_ipc::CompositorStatus {
                version: garchomp_ipc::PROTOCOL_VERSION,
                window_count: compositor.windows.len(),
                current_workspace: compositor.workspaces.current,
                effects_enabled: garchomp_ipc::EffectsStatus {
                    blur: compositor.effects.blur_enabled,
                    shadows: compositor.effects.shadow_enabled,
                    animations: compositor.effects.fade_enabled,
                    blur_strength: compositor.effects.blur_strength,
                },
                connected_to_gar: compositor.is_connected_to_gar(),
            })
        }
    };

    if let Err(e) = request.respond(response) {
        tracing::warn!("Failed to send IPC response: {}", e);
    }
}
