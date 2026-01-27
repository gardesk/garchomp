//! IPC server for garchomp control.

use garchomp_ipc::{Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IpcError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, IpcError>;

/// IPC server for compositor control.
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
}

impl IpcServer {
    /// Create a new IPC server.
    pub fn new() -> Result<Self> {
        let socket_path = Self::socket_path();

        // Remove existing socket
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)?;
        listener.set_nonblocking(true)?;

        tracing::info!("IPC server listening at {:?}", socket_path);

        Ok(Self {
            listener,
            socket_path,
        })
    }

    /// Get the socket path.
    fn socket_path() -> PathBuf {
        let runtime_dir =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(runtime_dir).join("garchomp.sock")
    }

    /// Poll for incoming connections (non-blocking).
    pub fn poll(&mut self) -> Option<ClientRequest> {
        match self.listener.accept() {
            Ok((stream, _)) => {
                if let Ok(request) = Self::read_request(&stream) {
                    Some(ClientRequest { stream, request })
                } else {
                    None
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
            Err(e) => {
                tracing::warn!("IPC accept error: {}", e);
                None
            }
        }
    }

    fn read_request(stream: &UnixStream) -> Result<Request> {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// A client request with the stream for responding.
pub struct ClientRequest {
    stream: UnixStream,
    pub request: Request,
}

impl ClientRequest {
    /// Send a response to the client.
    pub fn respond(mut self, response: Response) -> Result<()> {
        let mut msg = serde_json::to_string(&response)?;
        msg.push('\n');
        self.stream.write_all(msg.as_bytes())?;
        Ok(())
    }
}
