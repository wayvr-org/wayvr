use glam::{Affine2, Affine3A, Quat, Vec2, Vec3, vec2, vec3};
use smithay::{
    desktop::PopupManager,
    wayland::{compositor::with_states, shell::xdg::XdgPopupSurfaceData},
};
use std::{ops::RangeInclusive, sync::Arc};
use vulkano::{
    buffer::BufferUsage, image::view::ImageView, pipeline::graphics::color_blend::AttachmentBlend,
};
use wayvr_ipc::packet_client::PositionMode;
use wgui::{
    components::button::ComponentButton,
    event::EventCallback,
    gfx::{
        cmd::WGfxClearMode,
        pipeline::{WGfxPipeline, WPipelineCreateInfo},
    },
    i18n::Translation,
    parser::Fetchable,
    widget::{EventResult, label::WidgetLabel},
};
use wlx_capture::frame::MouseMeta;
use wlx_common::{
    overlays::{BackendAttrib, BackendAttribValue, StereoMode},
    windowing::{OverlayWindowState, Positioning},
};

use crate::{
    backend::{
        XrBackend,
        input::{self, HoverResult},
        wayvr::{self, SurfaceBufWithImage, process::KillSignal, window::WindowHandle},
    },
    graphics::{ExtentExt, Vert2Uv, upload_quad_vertices},
    gui::panel::{
        GuiPanel, NewGuiPanelParams, OnCustomAttribFunc,
        button::{BUTTON_EVENT_SUFFIX, BUTTON_EVENTS},
    },
    overlays::screen::capture::ScreenPipeline,
    state::{self, AppState},
    subsystem::{hid::WheelDelta, input::KeyboardFocus},
    windowing::{
        backend::{
            FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender,
            ui_transform,
        },
        window::{OverlayCategory, OverlayWindowConfig},
    },
};

pub enum WvrCommand {
    CloseWindow,
    KillProcess(KillSignal),
}

const BORDER_SIZE: u32 = 5;
const BAR_SIZE: u32 = 48;

pub fn create_wl_window_overlay(
    name: Arc<str>,
    app: &mut AppState,
    window: wayvr::window::WindowHandle,
    icon: Arc<str>,
    size: [u32; 2],
    pos_mode: PositionMode,
) -> anyhow::Result<OverlayWindowConfig> {
    let scale = size[0].max(size[1]) as f32 / 1920.0;
    let curve_scale = size[0] as f32 / 1920.0;

    let z_dist = if matches!(pos_mode, PositionMode::Anchor) {
        0.0
    } else {
        -0.95
    };

    Ok(OverlayWindowConfig {
        name: name.clone(),
        default_state: OverlayWindowState {
            grabbable: true,
            interactable: true,
            positioning: match pos_mode {
                PositionMode::Float => Positioning::Floating,
                PositionMode::Anchor => Positioning::Anchored,
                PositionMode::Static => Positioning::Static,
            },
            curvature: Some(0.15 * curve_scale),
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::ONE * scale,
                Quat::IDENTITY,
                vec3(0.0, 0.0, z_dist),
            ),
            ..OverlayWindowState::default()
        },
        keyboard_focus: Some(KeyboardFocus::WayVR),
        category: OverlayCategory::WayVR,
        show_on_spawn: true,
        ..OverlayWindowConfig::from_backend(Box::new(WvrWindowBackend::new(
            name, app, window, icon,
        )?))
    })
}

pub struct WvrWindowBackend {
    name: Arc<str>,
    icon: Arc<str>,
    pipeline: Option<ScreenPipeline>,
    popups_pipeline: Arc<WGfxPipeline<Vert2Uv>>,
    interaction_transform: Option<Affine2>,
    window: WindowHandle,
    popups: Vec<(Arc<ImageView>, Vec2)>,
    just_resumed: bool,
    meta: Option<FrameMeta>,
    mouse: Option<MouseMeta>,
    stereo: Option<StereoMode>,
    stereo_full_frame: bool,
    stereo_adjust_mouse: bool,
    cur_image: Option<Arc<ImageView>>,
    panel: GuiPanel<WindowHandle>,
    inner_extent: [u32; 2],
    mouse_transform: Affine2,
    uv_range: RangeInclusive<f32>,
    panel_hovered: bool,
}

impl WvrWindowBackend {
    fn new(
        name: Arc<str>,
        app: &mut AppState,
        window: wayvr::window::WindowHandle,
        icon: Arc<str>,
    ) -> anyhow::Result<Self> {
        let popups_pipeline = app.gfx.create_pipeline(
            app.gfx_extras.shaders.get("vert_quad").unwrap(), // want panic
            app.gfx_extras.shaders.get("frag_screen").unwrap(), // want panic
            WPipelineCreateInfo::new(app.gfx.surface_format).use_blend(AttachmentBlend::default()),
        )?;

        let on_custom_attrib: OnCustomAttribFunc =
            Box::new(move |layout, parser, attribs, _app| {
                let Ok(button) =
                    parser.fetch_component_from_widget_id_as::<ComponentButton>(attribs.widget_id)
                else {
                    return;
                };

                for (name, kind, test_button, test_duration) in &BUTTON_EVENTS {
                    for suffix in BUTTON_EVENT_SUFFIX {
                        let name = &format!("{name}{suffix}");
                        let Some(action) = attribs.get_value(name) else {
                            break;
                        };

                        let mut args = action.split_whitespace();
                        let Some(command) = args.next() else {
                            continue;
                        };

                        let button = button.clone();

                        let callback: EventCallback<AppState, WindowHandle> = match command {
                            "::DecorCloseWindow" => Box::new(move |_common, data, app, state| {
                                if !test_button(data) || !test_duration(&button, app) {
                                    return Ok(EventResult::Pass);
                                }

                                app.wvr_server.as_mut().unwrap().close_window(*state);

                                Ok(EventResult::Consumed)
                            }),
                            _ => return,
                        };

                        let id = layout.add_event_listener(attribs.widget_id, *kind, callback);
                        log::debug!("Registered {action} on {:?} as {id:?}", attribs.widget_id);
                    }
                }
            });

        let mut panel = GuiPanel::new_from_template(
            app,
            "gui/decor.xml",
            window,
            NewGuiPanelParams {
                resize_to_parent: true,
                on_custom_attrib: Some(on_custom_attrib),
                ..Default::default()
            },
        )?;

        {
            let mut title = panel
                .parser_state
                .fetch_widget_as::<WidgetLabel>(&panel.layout.state, "label_title")?;
            title.set_text_simple(
                &mut app.wgui_globals.get(),
                Translation::from_raw_text(&name),
            );
        }

        panel.update_layout(app)?;

        Ok(Self {
            name,
            icon,
            pipeline: None,
            window,
            popups: vec![],
            popups_pipeline,
            interaction_transform: None,
            just_resumed: false,
            meta: None,
            mouse: None,
            stereo: if matches!(app.xr_backend, XrBackend::OpenXR) {
                Some(StereoMode::None)
            } else {
                None
            },
            stereo_full_frame: false,
            stereo_adjust_mouse: false,
            cur_image: None,
            inner_extent: [0, 0],
            panel,
            mouse_transform: Affine2::ZERO,
            uv_range: 0.0..=1.0,
            panel_hovered: false,
        })
    }

    fn apply_extent(&mut self, app: &mut AppState, meta: &FrameMeta) -> anyhow::Result<()> {
        self.interaction_transform = Some(ui_transform(meta.extent));

        let mut scale = vec2(
            ((meta.extent[0] + BORDER_SIZE * 2) as f32) / (meta.extent[0] as f32),
            ((meta.extent[1] + BORDER_SIZE * 2 + BAR_SIZE) as f32) / (meta.extent[1] as f32),
        );

        if self.stereo_adjust_mouse
            && let Some(stereo) = self.stereo
        {
            match stereo {
                StereoMode::LeftRight | StereoMode::RightLeft => scale.x *= 0.5,
                StereoMode::TopBottom | StereoMode::BottomTop => scale.y *= 0.5,
                _ => {}
            }
        }

        let translation = vec2(
            -(BORDER_SIZE as f32) / (meta.extent[0] as f32),
            -((BORDER_SIZE + BAR_SIZE) as f32) / (meta.extent[1] as f32),
        );

        self.mouse_transform = Affine2::from_scale_angle_translation(scale, 0.0, translation);
        self.uv_range = translation[0]..=(1.0 - translation[0]);

        self.panel.max_size = vec2(
            (meta.extent[0]/*  + BORDER_SIZE * 2 (disabled for now) */) as _,
            BAR_SIZE as _,
        );
        self.panel.update_layout(app)?;

        Ok(())
    }
}

impl OverlayBackend for WvrWindowBackend {
    fn init(&mut self, app: &mut state::AppState) -> anyhow::Result<()> {
        self.panel.init(app)
    }

    fn pause(&mut self, app: &mut state::AppState) -> anyhow::Result<()> {
        self.panel.pause(app)
    }

    fn resume(&mut self, app: &mut state::AppState) -> anyhow::Result<()> {
        self.just_resumed = true;
        self.panel.resume(app)
    }

    #[allow(clippy::too_many_lines)]
    fn should_render(&mut self, app: &mut AppState) -> anyhow::Result<ShouldRender> {
        let should_render_panel = self.panel.should_render(app)?;

        let Some(toplevel) = app
            .wvr_server
            .as_ref()
            .and_then(|sv| sv.wm.windows.get(&self.window))
            .map(|win| win.toplevel.clone())
        else {
            log::debug!(
                "{:?}: WayVR overlay without matching window entry",
                self.name
            );
            return Ok(ShouldRender::Unable);
        };

        let popups = PopupManager::popups_for_surface(toplevel.wl_surface())
            .filter_map(|(popup, point)| {
                with_states(popup.wl_surface(), |states| {
                    if !states
                        .data_map
                        .get::<XdgPopupSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .configured
                    {
                        // not yet configured
                        return None;
                    }

                    if let Some(surf) = SurfaceBufWithImage::get_from_surface(states) {
                        Some((surf.image, vec2(point.x as _, point.y as _)))
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();

        with_states(toplevel.wl_surface(), |states| {
            if let Some(surf) = SurfaceBufWithImage::get_from_surface(states) {
                let mut meta = FrameMeta {
                    extent: surf.image.extent_u32arr(),
                    format: surf.image.format(),
                    clear: WGfxClearMode::Clear([0.0, 0.0, 0.0, 0.0]),
                    stereo: self.stereo.unwrap_or(StereoMode::None),
                    ..Default::default()
                };

                if let Some(stereo) = self.stereo {
                    // Apply stereo full frame logic
                    if self.stereo_full_frame {
                        match stereo {
                            StereoMode::LeftRight | StereoMode::RightLeft => {
                                meta.extent[0] /= 2;
                            }
                            StereoMode::TopBottom | StereoMode::BottomTop => {
                                meta.extent[1] /= 2;
                            }
                            _ => {}
                        }
                    }
                }

                let inner_extent = meta.extent;
                meta.extent[0] += BORDER_SIZE * 2;
                meta.extent[1] += BORDER_SIZE * 2 + BAR_SIZE;

                if let Some(pipeline) = self.pipeline.as_mut() {
                    if self.inner_extent != inner_extent {
                        pipeline.set_extent(
                            app,
                            [inner_extent[0] as _, inner_extent[1] as _],
                            [BORDER_SIZE as _, (BAR_SIZE + BORDER_SIZE) as _],
                        )?;
                        self.apply_extent(app, &meta)?;
                        self.inner_extent = inner_extent;
                    }
                } else {
                    let pipeline = ScreenPipeline::new(
                        &meta,
                        app,
                        self.stereo.unwrap_or(StereoMode::None),
                        [BORDER_SIZE as _, (BAR_SIZE + BORDER_SIZE) as _],
                    )?;
                    self.apply_extent(app, &meta)?;
                    self.pipeline = Some(pipeline);
                }

                let mouse = app
                    .wvr_server
                    .as_ref()
                    .unwrap()
                    .wm
                    .mouse
                    .as_ref()
                    .filter(|m| m.hover_window == self.window)
                    .map(|m| MouseMeta {
                        x: (m.x as f32) / (inner_extent[0] as f32),
                        y: (m.y as f32) / (inner_extent[1] as f32),
                    });

                let dirty = self.mouse != mouse || self.popups != popups;
                self.mouse = mouse;
                self.popups = popups;
                self.meta = Some(meta);
                if self
                    .cur_image
                    .as_ref()
                    .is_none_or(|i| *i.image() != *surf.image.image())
                {
                    log::trace!(
                        "{}: new {} image",
                        self.name,
                        if surf.dmabuf { "DMA-buf" } else { "SHM" }
                    );
                    self.cur_image = Some(surf.image);
                    Ok(ShouldRender::Should)
                } else if dirty {
                    Ok(ShouldRender::Should)
                } else {
                    Ok(should_render_panel)
                }
            } else {
                log::trace!("{}: no buffer for wl_surface", self.name);
                Ok(ShouldRender::Unable)
            }
        })
    }

    fn render(
        &mut self,
        app: &mut state::AppState,
        rdr: &mut RenderResources,
    ) -> anyhow::Result<()> {
        self.panel.render(app, rdr)?;
        // `GuiPanel` is not stereo-aware, so just render the same pass twice
        if rdr.cmd_bufs.len() > 1 {
            rdr.cmd_bufs.reverse();
            self.panel.render(app, rdr)?;
            rdr.cmd_bufs.reverse();
        }

        let image = self.cur_image.as_ref().unwrap().clone();

        self.pipeline
            .as_mut()
            .unwrap()
            .render(image, self.mouse.as_ref(), app, rdr)?;

        for (popup_img, point) in &self.popups {
            let meta = self.meta.as_ref().unwrap();
            let extentf = [meta.extent[0] as f32, meta.extent[1] as f32];
            let popup_extentf = popup_img.extent_f32();
            let mut buf_vert = app
                .gfx
                .empty_buffer(BufferUsage::TRANSFER_DST | BufferUsage::VERTEX_BUFFER, 4)?;

            upload_quad_vertices(
                &mut buf_vert,
                extentf[0],
                extentf[1],
                point.x,
                point.y,
                popup_extentf[0],
                popup_extentf[1],
            )?;

            let set0 = self.popups_pipeline.uniform_sampler(
                0,
                popup_img.clone(),
                app.gfx.texture_filter,
            )?;
            let pass = self.popups_pipeline.create_pass(
                extentf,
                [BORDER_SIZE as _, (BAR_SIZE + BORDER_SIZE) as _],
                buf_vert,
                0..4,
                0..1,
                vec![set0],
                &Default::default(),
            )?;

            for buf in &mut rdr.cmd_bufs {
                buf.run_ref(&pass)?;
            }
        }

        Ok(())
    }

    fn frame_meta(&mut self) -> Option<FrameMeta> {
        self.meta
    }

    fn notify(
        &mut self,
        app: &mut state::AppState,
        event_data: OverlayEventData,
    ) -> anyhow::Result<()> {
        match event_data {
            OverlayEventData::IdAssigned(oid) => {
                let wvr_server = app.wvr_server.as_mut().unwrap(); //never None
                wvr_server.overlay_added(oid, self.window);
            }
            OverlayEventData::WvrCommand(WvrCommand::CloseWindow) => {
                app.wvr_server.as_mut().unwrap().close_window(self.window);
            }
            OverlayEventData::WvrCommand(WvrCommand::KillProcess(signal)) => {
                let wvr_server = app.wvr_server.as_mut().unwrap();
                let Some(p) = wvr_server.wm.windows.get(&self.window) else {
                    return Ok(());
                };
                wvr_server.terminate_process(p.process, signal);
            }
            _ => {}
        }

        Ok(())
    }

    fn on_hover(&mut self, app: &mut state::AppState, hit: &input::PointerHit) -> HoverResult {
        let transformed = self.mouse_transform.transform_point2(hit.uv);

        if !self.uv_range.contains(&transformed.x) || !self.uv_range.contains(&transformed.y) {
            let Some(meta) = self.meta.as_ref() else {
                return HoverResult::default();
            };

            let mut hit2 = *hit;
            hit2.uv.y *= meta.extent[1] as f32 / (meta.extent[1] - self.inner_extent[1]) as f32;
            self.panel_hovered = true;
            return self.panel.on_hover(app, &hit2);
        } else if self.panel_hovered {
            self.panel.on_left(app, hit.pointer);
            self.panel_hovered = false;
        }

        let clamped = transformed.clamp(Vec2::ZERO, Vec2::ONE);

        let x = (clamped.x * (self.inner_extent[0] as f32)) as u32;
        let y = (clamped.y * (self.inner_extent[1] as f32)) as u32;

        let wvr_server = app.wvr_server.as_mut().unwrap(); //never None
        wvr_server.send_mouse_move(self.window, x, y);

        HoverResult {
            haptics: None, // haptics are handled via task
            consume: true,
        }
    }

    fn on_left(&mut self, app: &mut state::AppState, pointer: usize) {
        if self.panel_hovered {
            self.panel.on_left(app, pointer);
            self.panel_hovered = false;
        }
    }

    fn on_pointer(&mut self, app: &mut state::AppState, hit: &input::PointerHit, pressed: bool) {
        let transformed = self.mouse_transform.transform_point2(hit.uv);

        if !self.uv_range.contains(&transformed.x) || !self.uv_range.contains(&transformed.y) {
            let Some(meta) = self.meta.as_ref() else {
                return;
            };

            let mut hit2 = *hit;
            hit2.uv.y *= meta.extent[1] as f32 / (meta.extent[1] - self.inner_extent[1]) as f32;
            self.panel_hovered = true;
            return self.panel.on_pointer(app, &hit2, pressed);
        }

        if let Some(index) = match hit.mode {
            input::PointerMode::Left => Some(wayvr::MouseIndex::Left),
            input::PointerMode::Middle => Some(wayvr::MouseIndex::Center),
            input::PointerMode::Right => Some(wayvr::MouseIndex::Right),
            _ => {
                // Unknown pointer event, ignore
                None
            }
        } {
            let click_freeze = app.session.config.click_freeze_time_ms;
            let wvr_server = app.wvr_server.as_mut().unwrap(); //never None
            if pressed {
                wvr_server.send_mouse_down(click_freeze, self.window, index);
            } else {
                wvr_server.send_mouse_up(index);
            }
        }
    }

    fn on_scroll(
        &mut self,
        app: &mut state::AppState,
        _hit: &input::PointerHit,
        delta: WheelDelta,
    ) {
        let wvr_server = app.wvr_server.as_mut().unwrap(); //never None
        wvr_server.send_mouse_scroll(delta);
    }

    fn get_interaction_transform(&mut self) -> Option<Affine2> {
        self.interaction_transform
    }

    fn get_attrib(&self, attrib: BackendAttrib) -> Option<BackendAttribValue> {
        match attrib {
            BackendAttrib::Stereo => self.stereo.map(BackendAttribValue::Stereo),
            BackendAttrib::Icon => Some(BackendAttribValue::Icon(self.icon.clone())),
            BackendAttrib::StereoFullFrame => {
                Some(BackendAttribValue::StereoFullFrame(self.stereo_full_frame))
            }
            BackendAttrib::StereoAdjustMouse => Some(BackendAttribValue::StereoAdjustMouse(
                self.stereo_adjust_mouse,
            )),
            _ => None,
        }
    }
    fn set_attrib(&mut self, app: &mut AppState, value: BackendAttribValue) -> bool {
        match value {
            BackendAttribValue::Stereo(new) => {
                if let Some(stereo) = self.stereo.as_mut() {
                    log::debug!("{}: stereo: {stereo:?} → {new:?}", self.name);
                    *stereo = new;
                    if let Some(pipeline) = self.pipeline.as_mut() {
                        pipeline.ensure_stereo(new);
                    }
                    true
                } else {
                    false
                }
            }
            BackendAttribValue::StereoFullFrame(new) => {
                self.stereo_full_frame = new;
                true
            }
            BackendAttribValue::StereoAdjustMouse(new) => {
                self.stereo_adjust_mouse = new;
                if let Some(meta) = self.meta.take() {
                    let _ = self.apply_extent(app, &meta);
                    self.meta = Some(meta);
                }
                true
            }
            _ => false,
        }
    }
}
