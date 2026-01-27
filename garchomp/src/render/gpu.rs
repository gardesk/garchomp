//! GPU context and wgpu integration for compositor rendering.

use super::xlib::{XlibDisplay, XlibError, XlibWindowHandle};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GpuError {
    #[error("Xlib error: {0}")]
    Xlib(#[from] XlibError),

    #[error("failed to create wgpu surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),

    #[error("no suitable GPU adapter found")]
    NoAdapter,

    #[error("failed to request GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),

    #[error("surface error: {0}")]
    Surface(#[from] wgpu::SurfaceError),

    #[error("failed to get image from pixmap {pixmap:#x} ({width}x{height})")]
    GetImageFailed { pixmap: u64, width: u32, height: u32 },
}

pub type Result<T> = std::result::Result<T, GpuError>;

/// GPU context for compositor rendering.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    // Keep Xlib display alive for surface lifetime
    xlib_display: XlibDisplay,
}

impl GpuContext {
    /// Create a new GPU context for the given overlay window.
    pub async fn new(window: u32, width: u32, height: u32) -> Result<Self> {
        // Open separate Xlib connection for GPU
        let xlib_display = XlibDisplay::open()?;
        let display = xlib_display.display_ptr();
        let screen = xlib_display.default_screen();

        tracing::info!("Opened Xlib display for GPU, screen {}", screen);

        // Create wgpu instance - prefer Vulkan for better performance
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });

        // Create window handle for surface creation
        let handle = XlibWindowHandle::new(window, display, screen);

        // Create surface from X11 window
        let surface = instance.create_surface(handle)?;

        // Request adapter compatible with surface
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or(GpuError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        tracing::info!(
            "GPU adapter: {} (backend: {:?})",
            adapter_info.name,
            adapter_info.backend
        );

        // Request device with default limits
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("garchomp"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);

        // Prefer sRGB format
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        tracing::info!("Surface format: {:?}", surface_format);

        // Prefer opaque alpha mode for compositor overlay
        let alpha_mode = if surface_caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::Opaque)
        {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            surface_caps.alpha_modes[0]
        };

        // Prefer Mailbox (low latency) or Fifo (vsync)
        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };

        tracing::info!("Present mode: {:?}, alpha: {:?}", present_mode, alpha_mode);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };

        surface.configure(&device, &surface_config);

        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
            xlib_display,
        })
    }

    /// Resize the surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);
            tracing::debug!("Surface resized to {}x{}", width, height);
        }
    }

    /// Get the surface format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    /// Get current surface dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    /// Begin a frame - get the current surface texture.
    pub fn begin_frame(&self) -> Result<wgpu::SurfaceTexture> {
        Ok(self.surface.get_current_texture()?)
    }

    /// Create a command encoder.
    pub fn create_encoder(&self) -> wgpu::CommandEncoder {
        self.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("garchomp_encoder"),
            })
    }

    /// Submit commands and present frame.
    pub fn end_frame(&self, encoder: wgpu::CommandEncoder, frame: wgpu::SurfaceTexture) {
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }

    /// Poll the device and sync Xlib display.
    pub fn poll_and_sync(&self) {
        self.device.poll(wgpu::Maintain::Wait);
        self.xlib_display.sync();
    }

    /// Get the Xlib display pointer for sharing with other Xlib operations.
    pub fn display_ptr(&self) -> *mut std::ffi::c_void {
        self.xlib_display.display_ptr()
    }
}
