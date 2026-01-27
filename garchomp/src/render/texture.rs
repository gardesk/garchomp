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
    pub fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        window_id: u32,
        pixmap: u64,
        width: u32,
        height: u32,
    ) -> Result<&WindowTexture, GpuError> {
        // Get pixel data from pixmap
        let pixels = self.get_pixmap_data(pixmap, width, height)?;

        // Check if texture exists and has the right size
        let needs_recreate = match self.textures.get(&window_id) {
            Some(tex) => tex.width != width || tex.height != height,
            None => true,
        };

        if needs_recreate {
            // Create new texture
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
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

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

    /// Get pixel data from an X11 pixmap.
    fn get_pixmap_data(&self, pixmap: u64, width: u32, height: u32) -> Result<Vec<u8>, GpuError> {
        // Get the default visual for depth/color info
        let screen = unsafe { (self.xlib.XDefaultScreen)(self.display) };
        let visual = unsafe { (self.xlib.XDefaultVisual)(self.display, screen) };

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
            return Err(GpuError::GetImageFailed { pixmap, width, height });
        }

        // Convert to BGRA8 pixel data
        let pixels = unsafe { self.convert_ximage_to_bgra(image, width, height, visual) };

        // Free the XImage
        unsafe {
            (self.xlib.XDestroyImage)(image);
        }

        Ok(pixels)
    }

    /// Convert XImage data to BGRA8 format.
    unsafe fn convert_ximage_to_bgra(
        &self,
        image: *mut XImage,
        width: u32,
        height: u32,
        _visual: *mut Visual,
    ) -> Vec<u8> {
        // SAFETY: image pointer is valid, checked by caller
        let img = unsafe { &*image };
        let mut pixels = vec![0u8; (width * height * 4) as usize];

        let bytes_per_line = img.bytes_per_line as usize;
        let bits_per_pixel = img.bits_per_pixel;
        let data = img.data as *const u8;

        for y in 0..height {
            for x in 0..width {
                let src_offset = (y as usize) * bytes_per_line + (x as usize) * (bits_per_pixel / 8) as usize;
                let dst_offset = ((y * width + x) * 4) as usize;

                match bits_per_pixel {
                    32 => {
                        // Assume BGRA or BGRX format (common for 32-bit depth)
                        // SAFETY: src_offset is within image bounds
                        let b = unsafe { *data.add(src_offset) };
                        let g = unsafe { *data.add(src_offset + 1) };
                        let r = unsafe { *data.add(src_offset + 2) };
                        let a = unsafe { *data.add(src_offset + 3) };

                        pixels[dst_offset] = b;
                        pixels[dst_offset + 1] = g;
                        pixels[dst_offset + 2] = r;
                        pixels[dst_offset + 3] = if a == 0 { 255 } else { a };
                    }
                    24 => {
                        // BGR format
                        // SAFETY: src_offset is within image bounds
                        let b = unsafe { *data.add(src_offset) };
                        let g = unsafe { *data.add(src_offset + 1) };
                        let r = unsafe { *data.add(src_offset + 2) };

                        pixels[dst_offset] = b;
                        pixels[dst_offset + 1] = g;
                        pixels[dst_offset + 2] = r;
                        pixels[dst_offset + 3] = 255;
                    }
                    16 => {
                        // Assume RGB565
                        // SAFETY: src_offset is within image bounds
                        let lo = unsafe { *data.add(src_offset) as u16 };
                        let hi = unsafe { *data.add(src_offset + 1) as u16 };
                        let pixel = lo | (hi << 8);

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
