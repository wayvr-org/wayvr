use smithay::{
    reexports::wayland_server::{
        Resource,
        backend::ObjectId,
        protocol::{wl_shm, wl_surface::WlSurface},
    },
    wayland::{
        compositor::with_states, shm::with_buffer_contents,
        xdg_toplevel_icon::ToplevelIconCachedState,
    },
};
use std::{collections::HashMap, sync::Arc};
use wlx_common::desktop_finder::DesktopFinder;

#[derive(Debug, Default)]
pub struct ToplevelIconSnapshot {
    name: Option<Arc<str>>,
    png_path: Option<Arc<str>>,
}

impl ToplevelIconSnapshot {
    pub fn resolve(&self, finder: &DesktopFinder) -> Option<Arc<str>> {
        self.png_path.clone().or_else(|| {
            self.name
                .as_deref()
                .and_then(|name| finder.resolve_icon(name))
                .map(Into::into)
        })
    }

    pub const fn is_empty(&self) -> bool {
        self.name.is_none() && self.png_path.is_none()
    }
}

pub fn toplevel_icon(
    surface: &WlSurface,
    icons: &HashMap<ObjectId, ToplevelIconSnapshot>,
    finder: &DesktopFinder,
) -> Option<Arc<str>> {
    icons
        .get(&surface.id())
        .and_then(|icon| icon.resolve(finder))
}

pub fn snapshot_pending_icon(surface: &WlSurface) -> ToplevelIconSnapshot {
    let (name, buffers) = with_states(surface, |states| {
        let mut icon = states.cached_state.get::<ToplevelIconCachedState>();
        let pending = icon.pending();
        (
            pending.icon_name().map(Arc::<str>::from),
            pending.buffers().to_vec(),
        )
    });

    for (buffer, _scale) in buffers {
        let mut png_path = None;
        let _ = with_buffer_contents(&buffer, |data, size, info| {
            let data = unsafe { std::slice::from_raw_parts(data, size) };
            let (w, h) = (info.width as usize, info.height as usize);
            if info.width <= 0
                || info.height <= 0
                || info.stride <= 0
                || info.offset < 0
                || w != h
                || w > 1024
            {
                return;
            }
            let stride = info.stride as usize;
            let offset = info.offset as usize;
            if stride < w * 4
                || offset
                    .checked_add((h - 1).saturating_mul(stride) + w * 4)
                    .is_none_or(|end| end > size)
            {
                return;
            }
            let has_alpha = match info.format {
                wl_shm::Format::Argb8888 => true,
                wl_shm::Format::Xrgb8888 => false,
                _ => return,
            };
            let mut rgba = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    let pos = offset + y * stride + x * 4;
                    let dst = (y * w + x) * 4;
                    rgba[dst] = data[pos + 2];
                    rgba[dst + 1] = data[pos + 1];
                    rgba[dst + 2] = data[pos];
                    rgba[dst + 3] = if has_alpha { data[pos + 3] } else { 255 };
                }
            }
            let path = wlx_common::cache_dir::get_path(&format!(
                "icons/wayland-{}.png",
                uuid::Uuid::new_v4()
            ));
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if image::save_buffer_with_format(
                &path,
                &rgba,
                w as u32,
                h as u32,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .is_ok()
            {
                png_path = Some(path.to_string_lossy().into_owned().into());
            }
        });
        if let Some(png_path) = png_path {
            return ToplevelIconSnapshot {
                name,
                png_path: Some(png_path),
            };
        }
    }
    ToplevelIconSnapshot {
        name,
        png_path: None,
    }
}
