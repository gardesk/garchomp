//! garchompctl - Control utility for garchomp compositor.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use garchomp_ipc::{Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[derive(Parser)]
#[command(name = "garchompctl", about = "Control garchomp compositor")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Reload configuration.
    Reload,
    /// Enable an effect.
    Enable { effect: String },
    /// Disable an effect.
    Disable { effect: String },
    /// Set blur strength (0-20).
    Blur { strength: u32 },
    /// List managed windows.
    Windows,
    /// Ping the compositor.
    Ping,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let request = match cli.command {
        Commands::Reload => Request::Reload,
        Commands::Enable { effect } => Request::SetEffect {
            effect,
            enabled: true,
        },
        Commands::Disable { effect } => Request::SetEffect {
            effect,
            enabled: false,
        },
        Commands::Blur { strength } => Request::SetBlurStrength { strength },
        Commands::Windows => Request::ListWindows,
        Commands::Ping => Request::Ping,
    };

    let response = send_request(&request)?;

    match response {
        Response::Ok => println!("OK"),
        Response::Pong => println!("pong"),
        Response::Error { message } => {
            eprintln!("Error: {}", message);
            std::process::exit(1);
        }
        Response::WindowInfo(info) => {
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        Response::WindowList { windows } => {
            for win in windows {
                println!(
                    "{:#010x}  {}x{}+{}+{}  {}",
                    win.id,
                    win.width,
                    win.height,
                    win.x,
                    win.y,
                    if win.mapped { "mapped" } else { "unmapped" }
                );
            }
        }
    }

    Ok(())
}

fn send_request(request: &Request) -> Result<Response> {
    let socket_path = format!(
        "{}/garchomp.sock",
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())
    );

    let mut stream =
        UnixStream::connect(&socket_path).context("Failed to connect to garchomp socket")?;

    let mut msg = serde_json::to_string(request)?;
    msg.push('\n');
    stream.write_all(msg.as_bytes())?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    serde_json::from_str(&line).context("Failed to parse response")
}
