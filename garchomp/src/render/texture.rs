//! Window texture management - imports X11 pixmaps to wgpu textures.

use super::GpuError;
use std::collections::HashMap;
use std::ffi::c_void;
use x11_dl::xlib::{Display, Pixmap, Visual, XImage, Xlib, ZPixmap};

/// Manages textures for compositor windows.
pub struct TextureManager {
    xlib: Xlib,
    display: *mut Display,
    textures: HashMap<u32, WindowTexture>,
}

/// Texture data for a single window.
pub struct WindowTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl TextureManager {
    /// Create a new texture manager.
    pub fn new(display: *mut c_void) -> Result<Self, GpuError> {
        let xlib = Xlib::open().map_err(|_| GpuError::Xlib(super::xlib::XlibError::LoadXlib))?;

        Ok(Self {
            xlib,
            display: display as *mut Display,
            textures: HashMap::new(),
        })
    }

    /// Get or create a texture for a window.
    pub fn get_texture(&self, window_id: u32) -> Option<&WindowTexture> {
        self.textures.get(&window_id)
    }

    /// Update a window's texture from its pixmap.
    ///
    /// `depth` should be the window's depth (e.g., 24 or 32).
    /// For 32-bit depth, alpha channel is preserved; for 24-bit, alpha is set to opaque.
    pub fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        window_id: u32,
        pixmap: u64,
        _width: u32,
        _height: u32,
    ) -> Result<&WindowTexture, GpuError> {
        // Query actual pixmap geometry - this gives us the true dimensions
        // The window geometry might include borders, but the pixmap is just content
        let (width, height, depth) = self.get_pixmap_geometry(pixmap);

        if width == 0 || height == 0 {
            return Err(GpuError::GetImageFailed { pixmap, width, height });
        }

        // Get pixel data from pixmap using actual dimensions
        let pixels = self.get_pixmap_data(pixmap, width, height, depth)?;

        // Check if texture exists and has the right size
        let needs_recreate = match self.textures.get(&window_id) {
            Some(tex) => tex.width != width || tex.height != height,
            None => true,
        };

        if needs_recreate {
            // Create new texture
            // Use sRGB format since X11 pixmap data is gamma-encoded (sRGB)
            // Use Rgba8UnormSrgb for GL backend compatibility
            tracing::debug!(
                "Creating texture for window {:#x}: {}x{}",
                window_id, width, height
            );

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("window_texture_{}", window_id)),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Use RGBA format for GL backend compatibility
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            tracing::trace!("Texture created for window {:#x}", window_id);

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            self.textures.insert(
                window_id,
                WindowTexture {
                    texture,
                    view,
                    width,
                    height,
                },
            );
        }

        // Upload pixel data
        let tex = self.textures.get(&window_id).unwrap();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        Ok(self.textures.get(&window_id).unwrap())
    }

    /// Get the geometry of a pixmap via XGetGeometry.
    /// Returns (width, height, depth). Returns (0, 0, 24) on failure.
    fn get_pixmap_geometry(&self, pixmap: u64) -> (u32, u32, u8) {
        let mut root: u64 = 0;
        let mut x: i32 = 0;
        let mut y: i32 = 0;
        let mut width: u32 = 0;
        let mut height: u32 = 0;
        let mut border_width: u32 = 0;
        let mut depth: u32 = 0;

        let status = unsafe {
            (self.xlib.XGetGeometry)(
                self.display,
                pixmap,
                &mut root,
                &mut x,
                &mut y,
                &mut width,
                &mut height,
                &mut border_width,
                &mut depth,
            )
        };

        if status == 0 {
            // Failed
            (0, 0, 24)
        } else {
            (width, height, depth as u8)
        }
    }

    /// Get pixel data from an X11 pixmap.
    fn get_pixmap_data(&self, pixmap: u64, width: u32, height: u32, depth: u8) -> Result<Vec<u8>, GpuError> {
        // Get the default visual for depth/color info
        let screen = unsafe { (self.xlib.XDefaultScreen)(self.display) };
        let visual = unsafe { (self.xlib.XDefaultVisual)(self.display, screen) };

        // Sync display to ensure pixmap is ready
        unsafe { (self.xlib.XSync)(self.display, 0) };

        // Get the image data from the pixmap
        let image = unsafe {
            (self.xlib.XGetImage)(
                self.display,
                pixmap,
                0,
                0,
                width,
                height,
                !0, // AllPlanes
                ZPixmap,
            )
        };

        if image.is_null() {
            tracing::warn!("XGetImage failed for pixmap {:#x} ({}x{}, depth={})", pixmap, width, height, depth);
            return Err(GpuError::GetImageFailed { pixmap, width, height });
        }

        // Convert to RGBA pixel data, using depth to determine alpha handling
        let pixels = unsafe { self.convert_ximage_to_bgra(image, width, height, visual, depth) };

        // Free the XImage
        unsafe {
            (self.xlib.XDestroyImage)(image);
        }

        Ok(pixels)
    }

    /// Convert XImage data to BGRA8 format.
    ///
    /// `depth` is the pixmap depth (from XGetGeometry), which tells us whether
    /// to preserve alpha (32-bit) or treat alpha as opaque (24-bit).
    unsafe fn convert_ximage_to_bgra(
        &self,
        image: *mut XImage,
        width: u32,
        height: u32,
        _visual: *mut Visual,
        depth: u8,
    ) -> Vec<u8> {
        // SAFETY: image pointer is valid, checked by caller
        let img = unsafe { &*image };
        let mut pixels = vec![0u8; (width * height * 4) as usize];

        let bytes_per_line = img.bytes_per_line as usize;
        let bits_per_pixel = img.bits_per_pixel;
        let data = img.data as *const u8;
        let byte_order = img.byte_order;
        let bitmap_bit_order = img.bitmap_bit_order;

        tracing::trace!(
            "XImage: {}x{}, depth={}, bpp={}, bytes_per_line={}",
            width, height, depth, bits_per_pixel, bytes_per_line
        );

        // True ARGB windows have depth 32; depth 24 windows have no real alpha
        let has_real_alpha = depth >= 32;

        // X11 byte order constants
        const LSBFIRST: i32 = 0;
        const MSBFIRST: i32 = 1;

        for y in 0..height {
            for x in 0..width {
                let src_offset = (y as usize) * bytes_per_line + (x as usize) * (bits_per_pixel / 8) as usize;
                let dst_offset = ((y * width + x) * 4) as usize;

                match bits_per_pixel {
                    32 => {
                        // Read raw bytes
                        // SAFETY: src_offset is within image bounds
                        let byte0 = unsafe { *data.add(src_offset) };
                        let byte1 = unsafe { *data.add(src_offset + 1) };
                        let byte2 = unsafe { *data.add(src_offset + 2) };
                        let byte3 = unsafe { *data.add(src_offset + 3) };

                        // X11 byte order determines how pixel is stored:
                        // LSBFirst (little-endian): memory = [B, G, R, A/X]
                        // MSBFirst (big-endian): memory = [A/X, R, G, B]
                        let (b, g, r, a) = if byte_order == LSBFIRST {
                            (byte0, byte1, byte2, byte3)
                        } else {
                            // MSBFirst: memory is [A, R, G, B]
                            (byte3, byte2, byte1, byte0)
                        };

                        // Output in RGBA format for wgpu texture (GL backend compatibility)
                        pixels[dst_offset] = r;
                        pixels[dst_offset + 1] = g;
                        pixels[dst_offset + 2] = b;

                        // For true 32-bit depth windows, preserve alpha exactly.
                        // For 24-bit windows displayed as 32bpp, alpha byte is garbage/0,
                        // so treat as opaque.
                        pixels[dst_offset + 3] = if has_real_alpha { a } else { 255 };
                    }
                    24 => {
                        // BGR or RGB format depending on byte order
                        // SAFETY: src_offset is within image bounds
                        let byte0 = unsafe { *data.add(src_offset) };
                        let byte1 = unsafe { *data.add(src_offset + 1) };
                        let byte2 = unsafe { *data.add(src_offset + 2) };

                        let (b, g, r) = if byte_order == LSBFIRST {
                            (byte0, byte1, byte2)
                        } else {
                            (byte2, byte1, byte0)
                        };

                        // Output in RGBA format
                        pixels[dst_offset] = r;
                        pixels[dst_offset + 1] = g;
                        pixels[dst_offset + 2] = b;
                        pixels[dst_offset + 3] = 255;
                    }
                    16 => {
                        // RGB565 format
                        // SAFETY: src_offset is within image bounds
                        let lo = unsafe { *data.add(src_offset) as u16 };
                        let hi = unsafe { *data.add(src_offset + 1) as u16 };
                        let pixel = if byte_order == LSBFIRST {
                            lo | (hi << 8)
                        } else {
                            (lo << 8) | hi
                        };

                        let r = ((pixel >> 11) & 0x1F) as u8;
                        let g = ((pixel >> 5) & 0x3F) as u8;
                        let b = (pixel & 0x1F) as u8;

                        pixels[dst_offset] = (b << 3) | (b >> 2);
                        pixels[dst_offset + 1] = (g << 2) | (g >> 4);
                        pixels[dst_offset + 2] = (r << 3) | (r >> 2);
                        pixels[dst_offset + 3] = 255;
                    }
                    _ => {
                        // Unsupported format - fill with magenta for visibility
                        pixels[dst_offset] = 255;
                        pixels[dst_offset + 1] = 0;
                        pixels[dst_offset + 2] = 255;
                        pixels[dst_offset + 3] = 255;
                    }
                }
            }
        }

        pixels
    }

    /// Remove a window's texture.
    pub fn remove_texture(&mut self, window_id: u32) {
        self.textures.remove(&window_id);
    }

    /// Clear all textures.
    pub fn clear(&mut self) {
        self.textures.clear();
    }
}

// The texture manager is Send because we carefully manage the Xlib display pointer
unsafe impl Send for TextureManager {}
