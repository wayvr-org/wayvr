use std::{mem, ops::RangeInclusive, rc::Rc, sync::Arc};

use glam::{Affine2, Affine3A, Quat, Vec3, vec2, vec3};
use slotmap::Key;
use smithay::{
    desktop::PopupManager,
    input::pointer::CursorImageStatus,
    reexports::wayland_server::Resource,
    utils::{Logical, Size},
    wayland::{
        compositor::with_states,
        shell::xdg::{XdgPopupSurfaceData, XdgToplevelSurfaceData},
    },
};
use vulkano::{
    buffer::BufferUsage, image::view::ImageView, pipeline::graphics::color_blend::AttachmentBlend,
};
use wayvr_ipc::packet_client::PositionMode;
use wgui::{
    color::{WguiColor, WguiColorName},
    components::button::ComponentButton,
    event::{CallbackDataCommon, EventCallback},
    gfx::{
        cmd::WGfxClearMode,
        pipeline::{WGfxPipeline, WPipelineCreateInfo},
    },
    i18n::Translation,
    parser::Fetchable,
    widget::{EventResult, label::WidgetLabel, rectangle::WidgetRectangle},
};
use wlx_capture::frame::{MouseMeta, Transform};
use wlx_common::{
    overlays::{BackendAttrib, BackendAttribValue, StereoMode},
    windowing::{OverlayWindowState, Positioning},
};

use crate::{
    backend::{
        input::{self, HoverResult},
        task::{OverlayTask, TaskType},
        wayvr::{
            self, PointerFocusTarget, SurfaceBufWithImage, WvrServerState,
            hit_test::{
                PopupRoot, RenderedSurface, WvrHitContext, WvrHitTarget,
                collect_rendered_surface_tree, collect_rendered_surface_tree_at,
                rendered_surfaces_dirty,
            },
            process::KillSignal,
            window::WindowHandle,
        },
    },
    graphics::{ExtentExt, Vert2Uv, upload_quad_vertices},
    gui::panel::{
        GuiPanel, NewGuiPanelParams, OnCustomAttribFunc,
        button::{BUTTON_EVENT_SUFFIX, BUTTON_EVENTS},
    },
    overlays::screen::capture::ScreenPipeline,
    state::{self, AppState},
    subsystem::{hid::WheelDelta, input::InputFocus},
    windowing::{
        OverlayID, OverlaySelector,
        backend::{
            FrameMeta, OverlayBackend, OverlayEventData, RenderResources, ShouldRender,
            ui_transform,
        },
        overlay_scale_from_extent,
        window::{OverlayCategory, OverlayWindowConfig},
    },
};

#[derive(Clone)]
pub enum WvrCommand {
    ReloadTitle,
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
    let (scale, curve_scale) = overlay_scale_from_extent(size);

    let z_dist = if matches!(pos_mode, PositionMode::Anchor) {
        0.0
    } else {
        -0.95
    };

    let resizable = app
        .wvr_server
        .as_mut()
        .and_then(|wvr| wvr.wm.windows.get(window))
        .map(|w| w.resizable())
        .unwrap_or(false);

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
                Vec3::ONE * app.session.config.default_overlay_scale * scale,
                Quat::IDENTITY,
                vec3(0.0, 0.0, z_dist),
            ),
            ..OverlayWindowState::default()
        },
        input_focus: Some(InputFocus::WayVR),
        category: OverlayCategory::WayVR,
        show_on_spawn: true,
        ..OverlayWindowConfig::from_backend(Box::new(WvrWindowBackend::new(
            name,
            app,
            window,
            icon,
            (scale, curve_scale),
            resizable,
        )?))
    })
}

pub struct WvrWindowBackend {
    name: Arc<str>,
    icon: Arc<str>,
    pipeline: Option<ScreenPipeline>,
    subsurface_pipeline: Arc<WGfxPipeline<Vert2Uv>>,
    popup_outside_button: Option<wayvr::MouseIndex>,
    interaction_transform: Option<Affine2>,
    window: WindowHandle,
    popups: Rc<[RenderedSurface]>,
    surfaces: Rc<[RenderedSurface]>,
    hit_context: Option<WvrHitContext>,
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
    scrolling: bool,
    overlay_id: OverlayID,
    scale: (f32, f32),
    resizable: bool,
    had_focus: bool,
}

impl WvrWindowBackend {
    fn new(
        name: Arc<str>,
        app: &mut AppState,
        window: wayvr::window::WindowHandle,
        icon: Arc<str>,
        scale: (f32, f32),
        resizable: bool,
    ) -> anyhow::Result<Self> {
        let subsurface_pipeline = app.gfx.create_pipeline(
            app.gfx_extras.shaders.get("vert_quad").unwrap(), // want panic
            app.gfx_extras.shaders.get("frag_simple").unwrap(), // want panic
            WPipelineCreateInfo::new(app.gfx.surface_format).use_blend(AttachmentBlend::alpha()),
        )?;

        let on_custom_attrib: OnCustomAttribFunc =
            Box::new(move |layout, parser, attribs, _app| {
                let Ok(button) = parser.fetch_component_from_widget_id_as::<ComponentButton>(
                    &layout.state,
                    attribs.widget_id,
                ) else {
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
                extra_vars: [("title".into(), name.as_ref().into())].into(),
                ..Default::default()
            },
        )?;

        panel.update_layout(app)?;

        Ok(Self {
            name,
            icon,
            pipeline: None,
            window,
            popups: Default::default(),
            surfaces: Default::default(),
            hit_context: None,
            subsurface_pipeline,
            popup_outside_button: None,
            interaction_transform: None,
            just_resumed: false,
            meta: None,
            mouse: None,
            stereo: app.xr_backend.is_open_xr().then_some(StereoMode::None),
            stereo_full_frame: false,
            stereo_adjust_mouse: false,
            cur_image: None,
            inner_extent: [0, 0],
            panel,
            mouse_transform: Affine2::ZERO,
            uv_range: 0.0..=1.0,
            panel_hovered: false,
            scrolling: false,
            overlay_id: OverlayID::null(),
            scale,
            resizable,
            had_focus: false,
        })
    }

    fn apply_extent(&mut self, app: &mut AppState, meta: &FrameMeta) -> anyhow::Result<()> {
        let (old_scale, old_curve_scale) = self.scale;
        let (new_scale, curve_scale) = overlay_scale_from_extent(meta.extent);
        self.scale = (new_scale, curve_scale);

        if new_scale.abs() > f32::EPSILON {
            let scale_delta = new_scale / old_scale;
            let curve_scale_delta = curve_scale / old_curve_scale;

            if (scale_delta - 1.0).abs() > f32::EPSILON
                || (curve_scale_delta - 1.0).abs() > f32::EPSILON
            {
                self.resizable = app
                    .wvr_server
                    .as_mut()
                    .and_then(|wvr| wvr.wm.windows.get(self.window))
                    .map(|w| w.resizable())
                    .unwrap_or(false);

                app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                    OverlaySelector::Id(self.overlay_id),
                    Box::new(move |_app, owc| {
                        if let Some(state) = owc.active_state.as_mut() {
                            if let Some(curvature) = state.curvature.as_mut() {
                                *curvature = (curve_scale_delta * *curvature).clamp(0.0, 0.5);
                            }

                            if let Some(saved) = state.saved_transform.as_mut() {
                                saved.matrix3 = saved.matrix3.mul_scalar(scale_delta);
                            }

                            state.transform.matrix3 =
                                state.transform.matrix3.mul_scalar(scale_delta);
                        }
                    }),
                )));
            }
        }

        self.interaction_transform = Some(ui_transform(meta.extent));

        let mut scale = vec2(
            ((meta.extent[0] + BORDER_SIZE * 2) as f32) / meta.extent[0] as f32,
            ((meta.extent[1] + BORDER_SIZE * 2 + BAR_SIZE) as f32) / meta.extent[1] as f32,
        );

        let mut translation = vec2(
            -(BORDER_SIZE as f32) / meta.extent[0] as f32,
            -((BORDER_SIZE + BAR_SIZE) as f32) / meta.extent[1] as f32,
        );

        if self.stereo_adjust_mouse
            && let Some(stereo) = self.stereo
        {
            match stereo {
                StereoMode::LeftRight | StereoMode::RightLeft => {
                    scale.x *= 0.5;
                    translation.x *= 0.5;
                }
                StereoMode::TopBottom | StereoMode::BottomTop => {
                    scale.y *= 0.5;
                    translation.y *= 0.5;
                }
                _ => {}
            }
        }

        self.mouse_transform = Affine2::from_scale_angle_translation(scale, 0.0, translation);
        self.uv_range = translation[0]..=(1.0 - translation[0]);

        self.panel.max_size = vec2(
            (meta.extent[0]/*  + BORDER_SIZE * 2 (disabled for now) */) as _,
            BAR_SIZE as _,
        );
        self.panel.update_layout(app)?;

        Ok(())
    }

    fn update_title(&mut self, new_title: Rc<str>) {
        let mut common = CallbackDataCommon {
            state: &self.panel.layout.state,
            alterables: &mut self.panel.layout.alterables,
        };

        if let Ok(mut title) = self
            .panel
            .parser_state
            .fetch_widget_as::<WidgetLabel>(&self.panel.layout.state, "title")
        {
            title.set_text(&mut common, Translation::from_raw_text_rc(new_title));
        }
    }

    fn update_decor(&mut self, wvr_server: &mut WvrServerState) {
        let now_focused = wvr_server
            .get_focused_window()
            .is_some_and(|w| w == self.window);

        if now_focused == self.had_focus {
            return;
        }

        self.had_focus = now_focused;

        let mut common = CallbackDataCommon {
            state: &self.panel.layout.state,
            alterables: &mut self.panel.layout.alterables,
        };

        const COLORS: [(WguiColor, WguiColor); 2] = [
            (
                WguiColorName::Outline.to_wgui_color(),
                WguiColorName::OnBackground.to_wgui_color(),
            ),
            (
                WguiColorName::Tertiary.to_wgui_color(),
                WguiColorName::Tertiary.to_wgui_color(),
            ),
        ];

        let (rect_col, label_col) = COLORS[now_focused as usize];

        if let Ok(mut rect) = self
            .panel
            .parser_state
            .fetch_widget_as::<WidgetRectangle>(&self.panel.layout.state, "rect")
        {
            rect.set_border_color(&mut common, rect_col);
        }

        if let Ok(mut title) = self
            .panel
            .parser_state
            .fetch_widget_as::<WidgetLabel>(&self.panel.layout.state, "title")
        {
            title.set_color(&mut common, label_col, true);
        }
    }

    fn mouse_index_from_mode(mode: input::PointerMode) -> Option<wayvr::MouseIndex> {
        match mode {
            input::PointerMode::Left => Some(wayvr::MouseIndex::Left),
            input::PointerMode::Middle => Some(wayvr::MouseIndex::Center),
            input::PointerMode::Right => Some(wayvr::MouseIndex::Right),
            _ => None,
        }
    }

    fn render_subsurface(
        &self,
        app: &mut AppState,
        rdr: &mut RenderResources,
        s: &RenderedSurface,
    ) -> anyhow::Result<()> {
        let meta = self.meta.as_ref().unwrap();
        let extentf = [meta.extent[0] as f32, meta.extent[1] as f32];

        let mut buf_vert = app
            .gfx
            .empty_buffer(BufferUsage::TRANSFER_DST | BufferUsage::VERTEX_BUFFER, 4)?;

        upload_quad_vertices(
            &mut buf_vert,
            extentf[0],
            extentf[1],
            s.pos.x,
            s.pos.y,
            s.size.x,
            s.size.y,
        )?;

        let set0 =
            self.subsurface_pipeline
                .uniform_sampler(0, s.image.clone(), app.gfx.texture_filter)?;

        let pass = self.subsurface_pipeline.create_pass(
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

        Ok(())
    }

    fn sync_committed_toplevel_size(
        &mut self,
        app: &mut AppState,
        inner_extent: [u32; 2],
    ) -> anyhow::Result<()> {
        let Some(wvr_server) = app.wvr_server.as_mut() else {
            return Ok(());
        };

        let bounds = wvr_server.manager.state.output_logical_size();

        let committed = Size::new(inner_extent[0].max(1) as i32, inner_extent[1].max(1) as i32);

        let Some(window) = wvr_server.wm.windows.get_mut(self.window) else {
            return Ok(());
        };

        let clamped = window.clamp_configure_size(committed, bounds);

        if committed == clamped {
            window.remember_committed_size(committed);
        } else if window.pending_configure_size.is_none() {
            log::warn!("Client committed invalid size {committed:?}; requesting {clamped:?}");
            window.request_size(clamped, bounds);
        } else {
            log::trace!(
                "Client committed invalid size {committed:?}, but configure {:?} is already pending",
                window.pending_configure_size,
            );
        }

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
        let Some(toplevel) = app
            .wvr_server
            .as_ref()
            .and_then(|sv| sv.wm.windows.get(self.window))
            .map(|win| win.toplevel.clone())
        else {
            log::debug!(
                "{:?}: WayVR overlay without matching window entry",
                self.name
            );
            return Ok(ShouldRender::Unable);
        };

        let surface_id = toplevel.wl_surface().id();
        let surfaces = collect_rendered_surface_tree(toplevel.wl_surface());

        let mut popup_roots = Vec::new();
        let mut popups = Vec::new();

        for (popup, point) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
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
                surface_origin: vec2(popup_origin.x as f32, popup_origin.y as f32),
            });

            popups.extend(collect_rendered_surface_tree_at(
                popup.wl_surface(),
                popup_origin,
                true,
            ));
        }

        let mut tree_dirty = false;

        if let Some(wvr_server) = app.wvr_server.as_mut() {
            self.update_decor(wvr_server);

            let state = &mut wvr_server.manager.state;
            tree_dirty |= state.take_redraw_request(&surface_id);
            tree_dirty |= state.has_pending_frame_callbacks(&surface_id);
            for surface in surfaces.iter() {
                tree_dirty |= state.take_redraw_request(&surface.surface_id);
                tree_dirty |= state.has_pending_frame_callbacks(&surface.surface_id);
            }
            for popup in popups.iter() {
                tree_dirty |= state.take_redraw_request(&popup.surface_id);
                tree_dirty |= state.has_pending_frame_callbacks(&popup.surface_id);
            }
        }

        let should_render_panel = self.panel.should_render(app)?;
        let force_render = tree_dirty || mem::take(&mut self.just_resumed);

        let hit_surfaces = surfaces.clone();
        self.surfaces = surfaces;

        let Some(surf) = with_states(toplevel.wl_surface(), SurfaceBufWithImage::get_from_surface)
        else {
            log::trace!("{}: no buffer for wl_surface", self.name);
            return Ok(ShouldRender::Unable);
        };

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
        self.sync_committed_toplevel_size(app, inner_extent)?;

        let hit_context = WvrHitContext {
            surfaces: hit_surfaces,
            popup_roots: popup_roots.into(),
            mouse_transform: self.mouse_transform,
            uv_range: self.uv_range.clone(),
            inner_extent,
            panel_height: BORDER_SIZE * 2 + BAR_SIZE,
        };
        self.hit_context = Some(hit_context);

        meta.extent[0] += BORDER_SIZE * 2;
        meta.extent[1] += BORDER_SIZE * 2 + BAR_SIZE;

        if let Some(pipeline) = self.pipeline.as_mut() {
            if self.inner_extent != inner_extent {
                pipeline.set_layout(
                    app,
                    [inner_extent[0] as _, inner_extent[1] as _],
                    [BORDER_SIZE as _, (BAR_SIZE + BORDER_SIZE) as _],
                    Transform::Normal,
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
                Transform::Normal,
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
                x: (m.pos.x as f32) / (inner_extent[0] as f32),
                y: (m.pos.y as f32) / (inner_extent[1] as f32),
            });

        let dirty = self.mouse != mouse || rendered_surfaces_dirty(&self.popups, &popups);
        if !self.scrolling {
            self.mouse = mouse;
        }
        self.popups = popups.into();
        self.meta = Some(meta);

        if force_render {
            self.cur_image = Some(surf.image);
            return Ok(ShouldRender::Should);
        }

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
        let mut callback_surfaces = Vec::with_capacity(self.surfaces.len() + self.popups.len());

        self.pipeline
            .as_mut()
            .unwrap()
            .render_screen(image, app, rdr)?;

        for surface in self.surfaces.iter() {
            self.render_subsurface(app, rdr, surface)?;
            callback_surfaces.push(&surface.surface_id);
        }

        for popup in self.popups.iter() {
            self.render_subsurface(app, rdr, popup)?;
            callback_surfaces.push(&popup.surface_id);
        }

        // frame callbacks for toplevel + subsurf + popup
        if let Some(wvr_server) = app.wvr_server.as_mut() {
            let state = &mut wvr_server.manager.state;

            match state.cursor_image {
                CursorImageStatus::Hidden => {}
                CursorImageStatus::Named(_) | CursorImageStatus::Surface(_) => {
                    // TODO: properly render surface?
                    if let Some(mouse) = self.mouse.as_ref() {
                        self.pipeline.as_mut().unwrap().render_mouse(mouse, rdr)?;
                    }
                }
            }

            if let Some(window) = wvr_server.wm.windows.get(self.window) {
                let surface_id = window.toplevel.wl_surface().id();
                state.send_frame_callbacks_for_surface_id(&surface_id);
            }
            for surface in self.surfaces.iter() {
                state.send_frame_callbacks_for_surface_id(&surface.surface_id);
            }
            for popup in self.popups.iter() {
                state.send_frame_callbacks_for_surface_id(&popup.surface_id);
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
                self.overlay_id = oid;
                let wvr_server = app.wvr_server.as_mut().unwrap(); //never None
                wvr_server.overlay_added(oid, self.window);
            }
            OverlayEventData::WvrCommand(WvrCommand::ReloadTitle) => {
                let wvr_server = app.wvr_server.as_mut().unwrap(); //never None
                if let Some(window) = wvr_server.wm.windows.get(self.window) {
                    let title = with_states(window.toplevel.wl_surface(), |states| {
                        states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .unwrap()
                            .lock()
                            .unwrap()
                            .title
                            .clone()
                    });
                    if let Some(title) = title {
                        self.update_title(title.into());
                    }
                }
            }
            OverlayEventData::WvrCommand(WvrCommand::CloseWindow) => {
                app.wvr_server.as_mut().unwrap().close_window(self.window);
            }
            OverlayEventData::WvrCommand(WvrCommand::KillProcess(signal)) => {
                let wvr_server = app.wvr_server.as_mut().unwrap();
                let Some(p) = wvr_server.wm.windows.get(self.window) else {
                    return Ok(());
                };
                wvr_server.terminate_process(p.process, signal);
            }
            OverlayEventData::ResizeRequest(new_size) => {
                let wvr_server = app.wvr_server.as_mut().unwrap();
                let Some(win) = wvr_server.wm.windows.get_mut(self.window) else {
                    log::warn!("Could not process resize request: window not found");
                    return Ok(());
                };
                let size: Size<i32, Logical> = Size::new(new_size[0] as i32, new_size[1] as i32);
                win.checked_configure_size(size);
            }
            _ => {}
        }

        Ok(())
    }

    fn on_hover(&mut self, app: &mut state::AppState, hit: &input::PointerHit) -> HoverResult {
        if std::mem::take(&mut self.scrolling) {
            // we scrolled on previous frame so don't send mouse move events in case the user wants to scroll on this frame as well
            return HoverResult::consume();
        }

        let Some(ref ctx) = self.hit_context else {
            return HoverResult::default();
        };

        match ctx.hit_target(hit) {
            Some(WvrHitTarget::Panel(hit2)) => {
                self.panel_hovered = true;
                self.panel.on_hover(app, &hit2)
            }
            Some(WvrHitTarget::Popup {
                surface,
                global_pos,
                origin,
            })
            | Some(WvrHitTarget::Surface {
                surface,
                global_pos,
                origin,
            }) => {
                if self.panel_hovered {
                    self.panel.on_left(app, hit.pointer);
                    self.panel_hovered = false;
                }

                let pick_now = app.input_state.picking_focus.is_picking();
                if pick_now {
                    app.input_state.stop_picking();
                    app.hid_provider
                        .set_input_focus(app.wvr_server.as_mut(), InputFocus::WayVR);
                }

                let wvr_server = app.wvr_server.as_mut().unwrap();
                wvr_server.send_mouse_move(
                    PointerFocusTarget::Surface { surface, origin },
                    global_pos,
                    self.window,
                    pick_now,
                );

                HoverResult::consume()
            }
            Some(WvrHitTarget::Toplevel { pos }) => {
                if self.panel_hovered {
                    self.panel.on_left(app, hit.pointer);
                    self.panel_hovered = false;
                }

                let pick_now = app.input_state.picking_focus.is_picking();
                if pick_now {
                    app.input_state.stop_picking();
                    app.hid_provider
                        .set_input_focus(app.wvr_server.as_mut(), InputFocus::WayVR);
                }

                let wvr_server = app.wvr_server.as_mut().unwrap();
                wvr_server.send_mouse_move(
                    PointerFocusTarget::Toplevel,
                    pos,
                    self.window,
                    pick_now,
                );

                HoverResult::consume()
            }
            None => HoverResult::default(), // pass
        }
    }

    fn on_left(&mut self, app: &mut state::AppState, pointer: usize) {
        if self.panel_hovered {
            self.panel.on_left(app, pointer);
            self.panel_hovered = false;
        }
    }

    fn on_pointer(&mut self, app: &mut state::AppState, hit: &input::PointerHit, pressed: bool) {
        let Some(index) = Self::mouse_index_from_mode(hit.mode) else {
            return;
        };

        let Some(ref ctx) = self.hit_context else {
            return;
        };

        let target = ctx.hit_target(hit);
        let outside_pos = ctx.unclamped_client_pos_from_hit(hit);
        let click_freeze = app.session.config.click_freeze_time_ms;

        // if the press was consumed to dismiss a popup, consume the matching release too.
        if !pressed && self.popup_outside_button == Some(index) {
            self.popup_outside_button = None;

            app.wvr_server.as_mut().unwrap().send_mouse_button(
                PointerFocusTarget::None,
                outside_pos,
                self.window,
                index,
                false,
                click_freeze,
            );
            return;
        }

        let popup_grab_active = !self.popups.is_empty()
            && app
                .wvr_server
                .as_ref()
                .is_some_and(|server| server.pointer_is_grabbed());

        let outside_grabbed_popup =
            popup_grab_active && !matches!(&target, Some(WvrHitTarget::Popup { .. }));

        if outside_grabbed_popup {
            if pressed {
                self.popup_outside_button = Some(index);
            }

            let click_freeze = app.session.config.click_freeze_time_ms;
            app.wvr_server.as_mut().unwrap().send_mouse_button(
                PointerFocusTarget::None,
                outside_pos,
                self.window,
                index,
                pressed,
                click_freeze,
            );
            return;
        }

        match target {
            Some(WvrHitTarget::Panel(hit2)) => {
                self.panel_hovered = true;
                self.panel.on_pointer(app, &hit2, pressed);
            }

            Some(WvrHitTarget::Popup {
                surface,
                global_pos,
                origin,
            })
            | Some(WvrHitTarget::Surface {
                surface,
                global_pos,
                origin,
            }) => {
                let wvr_server = app.wvr_server.as_mut().unwrap();

                wvr_server.send_mouse_button(
                    PointerFocusTarget::Surface { surface, origin },
                    global_pos,
                    self.window,
                    index,
                    pressed,
                    click_freeze,
                );
            }

            Some(WvrHitTarget::Toplevel { pos }) => {
                let click_freeze = app.session.config.click_freeze_time_ms;
                let wvr_server = app.wvr_server.as_mut().unwrap();

                wvr_server.send_mouse_button(
                    PointerFocusTarget::Toplevel,
                    pos,
                    self.window,
                    index,
                    pressed,
                    click_freeze,
                );
            }

            None => {}
        }
    }

    fn on_scroll(&mut self, app: &mut state::AppState, hit: &input::PointerHit, delta: WheelDelta) {
        let Some(ref ctx) = self.hit_context else {
            return;
        };

        let target = ctx.hit_target(hit);
        self.scrolling = true;

        match target {
            Some(WvrHitTarget::Panel(hit2)) => {
                self.panel.on_scroll(app, &hit2, delta);
                let _ = hit2;
            }

            Some(WvrHitTarget::Popup { global_pos, .. })
            | Some(WvrHitTarget::Surface { global_pos, .. }) => {
                let wvr_server = app.wvr_server.as_mut().unwrap();
                wvr_server.send_mouse_scroll(self.window, global_pos, delta);
            }
            Some(WvrHitTarget::Toplevel { pos }) => {
                let wvr_server = app.wvr_server.as_mut().unwrap();
                wvr_server.send_mouse_scroll(self.window, pos, delta);
            }
            None => {}
        }
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
            BackendAttrib::Resizable => Some(BackendAttribValue::Resizable(self.resizable)),

            _ => None,
        }
    }
    fn set_attrib(&mut self, app: &mut AppState, value: BackendAttribValue) -> bool {
        match value {
            BackendAttribValue::Stereo(new) => {
                if let Some(stereo) = self.stereo.as_mut() {
                    log::debug!("{}: stereo: {stereo:?} → {new:?}", self.name);
                    *stereo = new;
                    if let Some(meta) = self.meta.clone() {
                        let _ = self.apply_extent(app, &meta);
                    }
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
                if let Some(pipeline) = self.pipeline.as_mut() {
                    pipeline.set_stereo_adjust_mouse(new);
                }
                true
            }
            _ => false,
        }
    }
}
