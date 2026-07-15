use glam::{Affine2, Vec2};
use smithay::{
    desktop::PopupManager,
    reexports::wayland_server::{Resource, backend::ObjectId, protocol::wl_surface::WlSurface},
    utils::{Logical, Point},
    wayland::{
        compositor::{
            SUBSURFACE_ROLE, SubsurfaceCachedState, SurfaceAttributes, TraversalAction,
            with_states, with_surface_tree_upward,
        },
        shell::xdg::XdgPopupSurfaceData,
    },
};
use std::{ops::RangeInclusive, rc::Rc, sync::Arc};
use vulkano::image::view::ImageView;

use crate::graphics::ExtentExt;

use crate::backend::input::PointerHit;

use super::SurfaceBufWithImage;

pub const BORDER_SIZE: u32 = 5;
pub const BAR_SIZE: u32 = 48;

#[derive(Clone)]
pub struct PopupRoot {
    pub surface: WlSurface,
    pub surface_origin: Vec2,
}

#[derive(Clone)]
pub struct RenderedSurface {
    pub surface: WlSurface,
    pub surface_id: ObjectId,
    pub image: Arc<ImageView>,
    pub pos: Vec2,
    pub size: Vec2,
}

#[derive(Clone)]
pub enum WvrHitTarget {
    Panel(PointerHit),
    Toplevel {
        pos: Vec2,
    },
    Surface {
        surface: WlSurface,
        global_pos: Vec2,
        origin: Vec2,
    },
    Popup {
        surface: WlSurface,
        global_pos: Vec2,
        origin: Vec2,
    },
}

pub struct WvrHitContext {
    pub surfaces: Rc<[RenderedSurface]>,
    pub popup_roots: Rc<[PopupRoot]>,
    pub mouse_transform: Affine2,
    pub uv_range: RangeInclusive<f32>,
    pub inner_extent: [u32; 2],
    pub panel_height: u32,
}

impl WvrHitContext {
    pub fn popup_hit_at_client_pos(&self, pos: Vec2) -> Option<(WlSurface, Vec2)> {
        self.popup_roots.iter().find_map(|popup| {
            surface_tree_input_hit_test(&popup.surface, popup.surface_origin, pos, true)
        })
    }

    pub fn is_inside_client_area(&self, transformed: Vec2) -> bool {
        self.uv_range.contains(&transformed.x) && self.uv_range.contains(&transformed.y)
    }

    pub fn transformed_uv_from_hit(&self, hit: &PointerHit) -> Vec2 {
        self.mouse_transform.transform_point2(hit.uv)
    }

    pub fn client_pos_from_transformed_uv(&self, transformed: Vec2) -> Vec2 {
        Vec2::new(
            transformed.x * self.inner_extent[0] as f32,
            transformed.y * self.inner_extent[1] as f32,
        )
    }

    pub fn unclamped_client_pos_from_hit(&self, hit: &PointerHit) -> Vec2 {
        let transformed = self.transformed_uv_from_hit(hit);
        self.client_pos_from_transformed_uv(transformed)
    }

    pub fn panel_hit_from_hit(&self, hit: &PointerHit) -> Option<PointerHit> {
        if self.panel_height == 0 {
            return None;
        }
        let mut hit2 = *hit;
        let total_height = self.inner_extent[1] + self.panel_height;
        hit2.uv.y *= total_height as f32 / self.panel_height as f32;
        Some(hit2)
    }

    pub fn hit_target_at(&self, client_pos: Vec2) -> Option<WvrHitTarget> {
        let transformed = self.mouse_transform.inverse().transform_point2(Vec2::new(
            client_pos.x / self.inner_extent[0] as f32,
            client_pos.y / self.inner_extent[1] as f32,
        ));
        if let Some((surface, surface_origin)) = self.popup_hit_at_client_pos(client_pos) {
            return Some(WvrHitTarget::Popup {
                surface,
                global_pos: client_pos,
                origin: surface_origin,
            });
        }

        if !self.is_inside_client_area(transformed) {
            return None;
        }

        let hit_surface = self
            .surfaces
            .iter()
            .rev()
            .find(|s| surface_accepts_input(s, client_pos));

        if let Some(surface) = hit_surface {
            return Some(WvrHitTarget::Surface {
                surface: surface.surface.clone(),
                global_pos: client_pos,
                origin: surface.pos,
            });
        }

        Some(WvrHitTarget::Toplevel { pos: client_pos })
    }

    pub fn hit_target(&self, hit: &PointerHit) -> Option<WvrHitTarget> {
        let transformed = self.transformed_uv_from_hit(hit);
        let client_pos = self.client_pos_from_transformed_uv(transformed);

        // popups are checked before the panel/client bounds.
        if let Some((surface, surface_origin)) = self.popup_hit_at_client_pos(client_pos) {
            return Some(WvrHitTarget::Popup {
                surface,
                global_pos: client_pos,
                origin: surface_origin,
            });
        }

        if !self.is_inside_client_area(transformed) {
            return self.panel_hit_from_hit(hit).map(WvrHitTarget::Panel);
        }

        let hit_surface = self
            .surfaces
            .iter()
            .rev()
            .find(|s| surface_accepts_input(s, client_pos));

        if let Some(surface) = hit_surface {
            return Some(WvrHitTarget::Surface {
                surface: surface.surface.clone(),
                global_pos: client_pos,
                origin: surface.pos,
            });
        }

        let clamped = transformed.clamp(Vec2::ZERO, Vec2::ONE);
        let pos = self.client_pos_from_transformed_uv(clamped);

        Some(WvrHitTarget::Toplevel { pos })
    }
}

fn surface_location(states: &smithay::wayland::compositor::SurfaceData) -> Point<i32, Logical> {
    if states.role == Some(SUBSURFACE_ROLE) {
        let mut guard = states.cached_state.get::<SubsurfaceCachedState>();
        guard.current().location
    } else {
        (0, 0).into()
    }
}

pub fn collect_rendered_surface_tree_at(
    root: &WlSurface,
    initial_pos: Point<i32, Logical>,
    include_root: bool,
) -> Vec<RenderedSurface> {
    let mut out = Vec::new();
    let root_id = root.id();

    with_surface_tree_upward(
        root,
        initial_pos,
        |_, states, parent_pos| {
            let pos = *parent_pos + surface_location(states);

            // do not skip even if this surface has no buffer; children may still have buffers
            TraversalAction::DoChildren(pos)
        },
        |surface, states, parent_pos| {
            if !include_root && surface.id() == root_id {
                return;
            }

            let pos = *parent_pos + surface_location(states);

            if let Some(surf) = SurfaceBufWithImage::get_from_surface(states) {
                let extent = surf.image.extent_f32();
                let scale = surf.scale.max(1) as f32;

                out.push(RenderedSurface {
                    surface: surface.clone(),
                    surface_id: surface.id(),
                    image: surf.image,
                    pos: Vec2::new(pos.x as f32, pos.y as f32),
                    size: Vec2::new(extent[0] / scale, extent[1] / scale),
                });
            }
        },
        |_, _, _| true,
    );

    out
}

pub fn collect_rendered_surface_tree(root: &WlSurface) -> Rc<[RenderedSurface]> {
    collect_rendered_surface_tree_at(root, Point::<i32, Logical>::from((0, 0)), false).into()
}

fn surface_accepts_input(surface: &RenderedSurface, global_pos: Vec2) -> bool {
    let local = global_pos - surface.pos;

    if local.x < 0.0 || local.y < 0.0 || local.x >= surface.size.x || local.y >= surface.size.y {
        return false;
    }

    with_states(&surface.surface, |states| {
        let mut guard = states.cached_state.get::<SurfaceAttributes>();
        let attrs = guard.current();

        match attrs.input_region.as_ref() {
            None => true,
            Some(region) => {
                let point =
                    Point::<i32, Logical>::from((local.x.floor() as i32, local.y.floor() as i32));
                region.contains(point)
            }
        }
    })
}

fn surface_accepts_input_states(
    states: &smithay::wayland::compositor::SurfaceData,
    local: Vec2,
) -> bool {
    if local.x < 0.0 || local.y < 0.0 {
        return false;
    }

    let mut guard = states.cached_state.get::<SurfaceAttributes>();
    let attrs = guard.current();

    let point = Point::<i32, Logical>::from((local.x.floor() as i32, local.y.floor() as i32));

    // explicit input region wins, even if no render buffer
    if let Some(region) = attrs.input_region.as_ref() {
        return region.contains(point);
    }

    // fallback for normal rendered surfaces
    if let Some(surf) = SurfaceBufWithImage::get_from_surface(states) {
        let extent = surf.image.extent_f32();
        let scale = surf.scale.max(1) as f32;

        return local.x < extent[0] / scale && local.y < extent[1] / scale;
    }

    false
}

pub fn surface_tree_input_hit_test(
    root: &WlSurface,
    root_origin: Vec2,
    global_pos: Vec2,
    include_root: bool,
) -> Option<(WlSurface, Vec2)> {
    let mut hit = None;
    let root_id = root.id();

    with_surface_tree_upward(
        root,
        Point::<i32, Logical>::from((root_origin.x as i32, root_origin.y as i32)),
        |_, states, parent_pos| {
            let pos = *parent_pos + surface_location(states);
            TraversalAction::DoChildren(pos)
        },
        |surface, states, parent_pos| {
            if !include_root && surface.id() == root_id {
                return;
            }

            let pos = *parent_pos + surface_location(states);
            let surface_origin = Vec2::new(pos.x as f32, pos.y as f32);
            let local = global_pos - surface_origin;

            if surface_accepts_input_states(states, local) {
                // with_surface_tree_upward visits bottom-to-top, so later hits win
                hit = Some((surface.clone(), surface_origin));
            }
        },
        |_, _, _| true,
    );

    hit
}

pub fn rendered_surfaces_dirty(old: &[RenderedSurface], new: &[RenderedSurface]) -> bool {
    if old.len() != new.len() {
        return true;
    }

    old.iter().zip(new).any(|(a, b)| {
        a.surface_id != b.surface_id
            || a.pos != b.pos
            || a.size != b.size
            || *a.image.image() != *b.image.image()
    })
}

pub fn compute_transforms(inner_extent: [u32; 2]) -> (Affine2, RangeInclusive<f32>) {
    let ix = inner_extent[0].max(1) as f32;
    let iy = inner_extent[1].max(1) as f32;

    let extent_x = ix + BORDER_SIZE as f32 * 2.0;
    let extent_y = iy + BORDER_SIZE as f32 * 2.0 + BAR_SIZE as f32;

    let scale_x = (extent_x + BORDER_SIZE as f32 * 2.0) / extent_x;
    let scale_y = (extent_y + BORDER_SIZE as f32 * 2.0 + BAR_SIZE as f32) / extent_y;

    let translation_x = -(BORDER_SIZE as f32) / ix;
    let translation_y = -((BORDER_SIZE + BAR_SIZE) as f32) / iy;

    let mouse_transform = Affine2::from_scale_angle_translation(
        Vec2::new(scale_x, scale_y),
        0.0,
        Vec2::new(translation_x, translation_y),
    );
    let uv_range = translation_x..=(1.0 - translation_x);

    (mouse_transform, uv_range)
}

pub fn build_hit_context(
    toplevel: &WlSurface,
    _popup_manager: &PopupManager,
    inner_extent: [u32; 2],
) -> Option<WvrHitContext> {
    let (mouse_transform, uv_range) = compute_transforms(inner_extent);
    let panel_height = BORDER_SIZE * 2 + BAR_SIZE;

    let surfaces = collect_rendered_surface_tree(toplevel);

    let mut popup_roots = Vec::new();
    let mut popups = Vec::new();

    for (popup, point) in PopupManager::popups_for_surface(toplevel) {
        let configured = with_states(popup.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgPopupSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .configured
        });

        if !configured {
            continue;
        }

        let popup_origin = point - popup.geometry().loc;

        popup_roots.push(PopupRoot {
            surface: popup.wl_surface().clone(),
            surface_origin: Vec2::new(popup_origin.x as f32, popup_origin.y as f32),
        });

        popups.extend(collect_rendered_surface_tree_at(
            popup.wl_surface(),
            popup_origin,
            true,
        ));
    }

    Some(WvrHitContext {
        surfaces,
        popup_roots: popup_roots.into(),
        mouse_transform,
        uv_range,
        inner_extent,
        panel_height,
    })
}
