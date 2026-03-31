use glam::{Affine2, Affine3A, Vec2};
use smallvec::SmallVec;
use std::{any::Any, rc::Rc, sync::Arc};
use vulkano::{command_buffer::CommandBufferUsage, format::Format, image::view::ImageView};
use wgui::gfx::{
    WGfx,
    cmd::{GfxCommandBuffer, WGfxClearMode},
};
use wlx_common::{
    overlays::{BackendAttrib, BackendAttribValue, StereoMode},
    windowing::Positioning,
};

use crate::{
    backend::{
        input::{HoverResult, PointerHit},
        task::ModifyPanelCommand,
    },
    graphics::RenderResult,
    overlays::wayvr::WvrCommand,
    state::AppState,
    subsystem::hid::WheelDelta,
    windowing::{OverlayID, window::OverlayCategory},
};

#[derive(Default, Clone, Copy)]
pub struct FrameMeta {
    pub extent: [u32; 2],
    pub transform: Affine3A,
    #[allow(dead_code)]
    pub format: Format,
    pub clear: WGfxClearMode,
    pub stereo: StereoMode,
}

#[derive(Debug, Clone, Copy)]
pub enum ShouldRender {
    /// The overlay is dirty and needs to be rendered.
    Should,
    /// The overlay is not dirty but is ready to be rendered.
    Can,
    /// The overlay is not ready to be rendered.
    Unable,
}

pub struct RenderTarget {
    pub views: SmallVec<[Arc<ImageView>; 2]>,
}

pub struct RenderResources {
    pub cmd_bufs: SmallVec<[GfxCommandBuffer; 2]>,
    pub extent: [u32; 2],
}

impl RenderResources {
    pub fn new(gfx: Arc<WGfx>, target: RenderTarget, meta: &FrameMeta) -> anyhow::Result<Self> {
        let mut cmd_bufs = SmallVec::new_const();

        for tgt in target.views {
            let mut cmd_buf = gfx.create_gfx_command_buffer(CommandBufferUsage::OneTimeSubmit)?;

            cmd_buf.begin_rendering(tgt, meta.clear)?;
            cmd_bufs.push(cmd_buf);
        }

        Ok(Self {
            cmd_bufs,
            extent: meta.extent,
        })
    }

    pub fn cmd_buf_single(&mut self) -> &mut GfxCommandBuffer {
        self.cmd_bufs.first_mut().unwrap() // first must always be populated
    }

    pub fn end(self) -> anyhow::Result<SmallVec<[RenderResult; 2]>> {
        let mut ret_val = SmallVec::new_const();

        for mut buf in self.cmd_bufs {
            buf.end_rendering()?;
            ret_val.push(RenderResult {
                queue: buf.queue.clone(),
                cmd_buf: buf.build()?,
            });
        }

        Ok(ret_val)
    }
}

#[macro_export]
macro_rules! attrib_value {
    ($opt:expr, $variant:path) => {
        $opt.and_then(|e| match e {
            $variant(inner) => Some(inner),
            _ => None,
        })
    };
}

#[derive(Clone)]
pub struct OverlayMeta {
    pub id: OverlayID,
    pub name: Arc<str>,
    pub category: OverlayCategory,
    pub visible: bool,
    pub icon: Option<Arc<str>>,
}

#[allow(clippy::enum_variant_names)]
pub enum OverlayEventData {
    /// Notifies a newly added overlay of its ID, even before the overlay is shown.
    IdAssigned(OverlayID),
    ActiveSetChanged(Option<usize>),
    NumSetsChanged(usize),
    EditModeChanged(bool),
    OverlaysChanged(Rc<[OverlayMeta]>),
    VisibleOverlaysChanged(Rc<[OverlayID]>),
    DevicesChanged,
    SettingsChanged,
    OverlayGrabbed {
        name: Arc<str>,
        pos: Positioning,
        editing: bool,
    },
    CustomCommand {
        element: String,
        command: ModifyPanelCommand,
    },
    WvrCommand(WvrCommand),
}

pub trait OverlayBackend: Any {
    /// Called once, before the first frame is rendered
    fn init(&mut self, app: &mut AppState) -> anyhow::Result<()>;
    fn pause(&mut self, app: &mut AppState) -> anyhow::Result<()>;
    fn resume(&mut self, app: &mut AppState) -> anyhow::Result<()>;

    /// Called when the presentation layer is ready to present a new frame
    fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender>;

    /// Called when the contents need to be rendered to the swapchain
    fn render(&mut self, app: &mut AppState, rdr: &mut RenderResources) -> anyhow::Result<()>;

    /// Called to retrieve the effective extent of the image
    /// Used for creating swapchains.
    ///
    /// Must be Some if should_render was Should or Can on the same frame.
    fn frame_meta(&mut self) -> Option<FrameMeta>;

    fn notify(&mut self, app: &mut AppState, event_data: OverlayEventData) -> anyhow::Result<()>;

    fn on_hover(&mut self, app: &mut AppState, hit: &PointerHit) -> HoverResult;
    fn on_left(&mut self, app: &mut AppState, pointer: usize);
    fn on_pointer(&mut self, app: &mut AppState, hit: &PointerHit, pressed: bool);
    fn on_scroll(&mut self, app: &mut AppState, hit: &PointerHit, delta: WheelDelta);
    fn get_interaction_transform(&mut self) -> Option<Affine2>;
    fn get_attrib(&self, attrib: BackendAttrib) -> Option<BackendAttribValue>;
    fn set_attrib(&mut self, app: &mut AppState, value: BackendAttribValue) -> bool;
}

pub fn ui_transform(extent: [u32; 2]) -> Affine2 {
    let aspect = extent[0] as f32 / extent[1] as f32;
    let scale = if aspect < 1.0 {
        Vec2 {
            x: 1.0 / aspect,
            y: -1.0,
        }
    } else {
        Vec2 { x: 1.0, y: -aspect }
    };
    let center = Vec2 { x: 0.5, y: 0.5 };
    Affine2::from_scale_angle_translation(scale, 0.0, center)
}

pub struct DummyBackend {}

impl OverlayBackend for DummyBackend {
    fn init(&mut self, _: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn pause(&mut self, _: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn resume(&mut self, _: &mut AppState) -> anyhow::Result<()> {
        Ok(())
    }
    fn should_render(&mut self, _: &mut AppState) -> anyhow::Result<ShouldRender> {
        Ok(ShouldRender::Unable)
    }
    fn render(&mut self, _: &mut AppState, _: &mut RenderResources) -> anyhow::Result<()> {
        unreachable!()
    }
    fn frame_meta(&mut self) -> Option<FrameMeta> {
        unreachable!()
    }

    fn notify(&mut self, _: &mut AppState, _event_data: OverlayEventData) -> anyhow::Result<()> {
        unreachable!()
    }

    fn on_hover(&mut self, _: &mut AppState, _: &PointerHit) -> HoverResult {
        HoverResult::default()
    }
    fn on_left(&mut self, _: &mut AppState, _: usize) {}
    fn on_pointer(&mut self, _: &mut AppState, _: &PointerHit, _: bool) {}
    fn on_scroll(&mut self, _: &mut AppState, _: &PointerHit, _: WheelDelta) {}
    fn get_interaction_transform(&mut self) -> Option<glam::Affine2> {
        None
    }
    fn get_attrib(&self, _attrib: BackendAttrib) -> Option<BackendAttribValue> {
        None
    }
    fn set_attrib(&mut self, _: &mut AppState, _value: BackendAttribValue) -> bool {
        false
    }
}
