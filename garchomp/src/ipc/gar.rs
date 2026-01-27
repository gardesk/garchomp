//! Connection to gar window manager IPC.

use garchomp_ipc::GarEvent;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Connection state to gar window manager.
pub struct GarConnection {
    stream: Option<UnixStream>,
    reader: Option<BufReader<UnixStream>>,
    connected: bool,
    last_connect_attempt: Option<Instant>,
    reconnect_interval: Duration,
}

impl GarConnection {
    /// Create a new gar connection (initially disconnected).
    pub fn new() -> Self {
        Self {
            stream: None,
            reader: None,
            connected: false,
            last_connect_attempt: None,
            reconnect_interval: Duration::from_secs(5),
        }
    }

    /// Get the socket path for gar.
    fn socket_path() -> PathBuf {
        let runtime_dir =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(runtime_dir).join("gar.sock")
    }

    /// Attempt to connect to gar.
    pub fn connect(&mut self) -> bool {
        if self.connected {
            return true;
        }

        // Rate limit connection attempts
        if let Some(last) = self.last_connect_attempt {
            if last.elapsed() < self.reconnect_interval {
                return false;
            }
        }

        self.last_connect_attempt = Some(Instant::now());

        let path = Self::socket_path();
        match UnixStream::connect(&path) {
            Ok(stream) => {
                if let Err(e) = stream.set_nonblocking(true) {
                    tracing::warn!("Failed to set gar socket non-blocking: {}", e);
                    return false;
                }

                tracing::info!("Connected to gar at {:?}", path);

                // Clone stream for reader
                let reader_stream = match stream.try_clone() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Failed to clone gar socket: {}", e);
                        return false;
                    }
                };

                self.reader = Some(BufReader::new(reader_stream));
                self.stream = Some(stream);
                self.connected = true;

                // Subscribe to events
                if let Err(e) = self.subscribe() {
                    tracing::warn!("Failed to subscribe to gar events: {}", e);
                    self.disconnect();
                    return false;
                }

                true
            }
            Err(e) => {
                tracing::debug!("Could not connect to gar: {} (will retry)", e);
                false
            }
        }
    }

    /// Subscribe to gar events.
    fn subscribe(&mut self) -> std::io::Result<()> {
        if let Some(ref mut stream) = self.stream {
            // Send subscription request (gar i3 IPC format)
            let msg = r#"{"type":"subscribe","payload":["window","workspace"]}"#;
            stream.write_all(msg.as_bytes())?;
            stream.write_all(b"\n")?;
            stream.flush()?;
            tracing::debug!("Subscribed to gar events");
        }
        Ok(())
    }

    /// Disconnect from gar.
    pub fn disconnect(&mut self) {
        if self.connected {
            tracing::info!("Disconnected from gar");
        }
        self.stream = None;
        self.reader = None;
        self.connected = false;
    }

    /// Check if connected to gar.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Get raw fd for polling (if connected).
    pub fn as_raw_fd(&self) -> Option<RawFd> {
        self.stream.as_ref().map(|s| s.as_raw_fd())
    }

    /// Poll for events from gar (non-blocking).
    pub fn poll(&mut self) -> Option<GarEvent> {
        if !self.connected {
            return None;
        }

        let reader = self.reader.as_mut()?;
        let mut line = String::new();

        match reader.read_line(&mut line) {
            Ok(0) => {
                // EOF - gar disconnected
                tracing::info!("gar connection closed");
                self.disconnect();
                None
            }
            Ok(_) => {
                // Parse event
                match serde_json::from_str::<GarEvent>(&line) {
                    Ok(event) => {
                        tracing::debug!("Received gar event: {:?}", event);
                        Some(event)
                    }
                    Err(e) => {
                        tracing::trace!("Failed to parse gar event: {} (line: {})", e, line.trim());
                        None
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
            Err(e) => {
                tracing::warn!("Error reading from gar: {}", e);
                self.disconnect();
                None
            }
        }
    }

    /// Try to reconnect if disconnected.
    pub fn try_reconnect(&mut self) -> bool {
        if !self.connected {
            self.connect()
        } else {
            true
        }
    }

    /// Send a message to gar.
    pub fn send(&mut self, msg: &str) -> std::io::Result<()> {
        if let Some(ref mut stream) = self.stream {
            stream.write_all(msg.as_bytes())?;
            stream.write_all(b"\n")?;
            stream.flush()?;
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Not connected to gar",
            ))
        }
    }
}

impl Default for GarConnection {
    fn default() -> Self {
        Self::new()
    }
}
