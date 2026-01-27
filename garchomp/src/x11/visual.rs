//! X11 visual selection for DeepColor/HDR support.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ColormapAlloc, ConnectionExt, Screen, Visualid, VisualClass, Colormap,
};

/// Visual configuration for the compositor.
#[derive(Debug, Clone)]
pub struct VisualConfig {
    pub visual_id: Visualid,
    pub depth: u8,
    pub hdr_capable: bool,
    pub colormap: Option<Colormap>,
}

impl VisualConfig {
    /// Check if this visual supports 10-bit color (30-bit depth).
    pub fn is_deepcolor(&self) -> bool {
        self.depth >= 30
    }
}

/// Find the best visual for HDR/DeepColor rendering.
///
/// Priority:
/// 1. 30-bit depth (10-bit per channel) TrueColor visual
/// 2. 24-bit depth (8-bit per channel) TrueColor visual (fallback)
pub fn find_best_visual<C: Connection>(
    conn: &C,
    screen: &Screen,
) -> Option<VisualConfig> {
    let mut best_visual: Option<(Visualid, u8)> = None;
    let mut deepcolor_visual: Option<Visualid> = None;

    for depth_info in &screen.allowed_depths {
        for visual in &depth_info.visuals {
            // Only consider TrueColor visuals
            if visual.class != VisualClass::TRUE_COLOR {
                continue;
            }

            // Prefer 30-bit depth (DeepColor)
            if depth_info.depth >= 30 {
                deepcolor_visual = Some(visual.visual_id);
                tracing::info!(
                    "Found DeepColor visual: id={:#x} depth={} rgb_bits={}/{}/{}",
                    visual.visual_id,
                    depth_info.depth,
                    visual.red_mask.count_ones(),
                    visual.green_mask.count_ones(),
                    visual.blue_mask.count_ones()
                );
            }

            // Track best visual (highest depth)
            if best_visual.map_or(true, |(_, d)| depth_info.depth > d) {
                best_visual = Some((visual.visual_id, depth_info.depth));
            }
        }
    }

    // Prefer DeepColor, fall back to best available
    let (visual_id, depth) = if let Some(dc) = deepcolor_visual {
        (dc, 30)
    } else if let Some((v, d)) = best_visual {
        (v, d)
    } else {
        // Last resort: use root visual
        return Some(VisualConfig {
            visual_id: screen.root_visual,
            depth: screen.root_depth,
            hdr_capable: false,
            colormap: None,
        });
    };

    let hdr_capable = depth >= 30;

    tracing::info!(
        "Selected visual: id={:#x} depth={} hdr_capable={}",
        visual_id,
        depth,
        hdr_capable
    );

    Some(VisualConfig {
        visual_id,
        depth,
        hdr_capable,
        colormap: None,
    })
}

/// Create a colormap for a non-default visual.
pub fn create_colormap<C: Connection>(
    conn: &C,
    screen: &Screen,
    visual_id: Visualid,
) -> Result<Colormap, x11rb::errors::ReplyOrIdError> {
    // Only need a new colormap if visual differs from root
    if visual_id == screen.root_visual {
        return Ok(screen.default_colormap);
    }

    let colormap = conn.generate_id()?;
    conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, visual_id)?;

    tracing::debug!("Created colormap {:#x} for visual {:#x}", colormap, visual_id);

    Ok(colormap)
}

/// Query the depth of a specific visual.
pub fn get_visual_depth(screen: &Screen, visual_id: Visualid) -> Option<u8> {
    for depth_info in &screen.allowed_depths {
        for visual in &depth_info.visuals {
            if visual.visual_id == visual_id {
                return Some(depth_info.depth);
            }
        }
    }
    None
}

/// Check if the display supports DeepColor (30-bit) visuals.
pub fn has_deepcolor_support(screen: &Screen) -> bool {
    for depth_info in &screen.allowed_depths {
        if depth_info.depth >= 30 {
            for visual in &depth_info.visuals {
                if visual.class == VisualClass::TRUE_COLOR {
                    return true;
                }
            }
        }
    }
    false
}
