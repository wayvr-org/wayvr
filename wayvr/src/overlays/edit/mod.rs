use std::{
    any::Any,
    cell::RefCell,
    mem::{self, ManuallyDrop},
    rc::Rc,
    time::{Duration, Instant},
};

use glam::vec2;
use slotmap::Key;
use wgui::{
    components::{button::ComponentButton, checkbox::ComponentCheckbox, slider::ComponentSlider},
    event::EventCallback,
    i18n::Translation,
    parser::Fetchable,
    widget::EventResult,
};
use wlx_common::overlays::{BackendAttrib, BackendAttribValue, MouseTransform, StereoMode};

use crate::{
    attrib_value,
    backend::{
        input::HoverResult,
        task::{OverlayTask, TaskContainer, TaskType},
    },
    gui::panel::{
        GuiPanel, NewGuiPanelParams, OnCustomAttribFunc,
        button::{BUTTON_EVENT_SUFFIX, BUTTON_EVENTS},
    },
    overlays::edit::{
        lock::InteractLockHandler,
        mouse::new_mouse_tab_handler,
        pos::{PosTabState, new_pos_tab_handler},
        sprite_tab::SpriteTabHandler,
        stereo::{get_stereo_full_frame_translation, new_stereo_tab_handler},
        tab::ButtonPaneTabSwitcher,
    },
    state::AppState,
    subsystem::hid::WheelDelta,
    windowing::{
        OverlayID, OverlaySelector,
        backend::{DummyBackend, OverlayBackend, OverlayEventData, RenderResources, ShouldRender},
        window::OverlayWindowConfig,
    },
};

mod lock;
mod mouse;
mod pos;
mod sprite_tab;
mod stereo;
pub mod tab;

struct EditModeState {
    tasks: Rc<RefCell<TaskContainer>>,
    id: Rc<RefCell<OverlayID>>,
    tabs: ButtonPaneTabSwitcher,
    lock: InteractLockHandler,
    pos: SpriteTabHandler<PosTabState>,
    stereo: SpriteTabHandler<StereoMode>,
    mouse: SpriteTabHandler<MouseTransform>,
}

type EditModeWrapPanel = GuiPanel<EditModeState>;

#[derive(Default)]
pub struct EditWrapperManager {
    #[allow(dead_code)]
    edit_mode: bool,
    panel_pool: Vec<EditModeWrapPanel>,
}

impl EditWrapperManager {
    pub fn wrap_edit_mode(
        &mut self,
        id: OverlayID,
        owc: &mut OverlayWindowConfig,
        app: &mut AppState,
    ) -> anyhow::Result<()> {
        if owc.editing {
            return Ok(());
        }

        log::debug!("EditMode wrap on {}", owc.name);
        let mut panel = self.panel_pool.pop();
        if panel.is_none() {
            panel = Some(make_edit_panel(app)?);
        }
        let mut panel = panel.unwrap();
        reset_panel(&mut panel, id, owc)?;

        let inner = mem::replace(&mut owc.backend, Box::new(DummyBackend {}));

        owc.backend = Box::new(EditModeBackendWrapper {
            inner: ManuallyDrop::new(inner),
            panel: ManuallyDrop::new(panel),
            last_render: Instant::now(),
            can_render_inner: false,
        });
        owc.editing = true;

        Ok(())
    }

    pub fn unwrap_edit_mode(
        &mut self,
        owc: &mut OverlayWindowConfig,
        app: &mut AppState,
    ) -> anyhow::Result<()> {
        if !owc.editing {
            return Ok(());
        }

        log::debug!("EditMode unwrap on {}", owc.name);
        let wrapper = mem::replace(&mut owc.backend, Box::new(DummyBackend {}));
        let mut wrapper: Box<dyn Any> = wrapper;
        let wrapper = wrapper
            .downcast_mut::<EditModeBackendWrapper>()
            .expect("Wrong type to unwrap");

        let panel = unsafe { ManuallyDrop::take(&mut wrapper.panel) };
        self.panel_pool.push(panel);

        let inner = unsafe { ManuallyDrop::take(&mut wrapper.inner) };
        owc.backend = inner;
        owc.editing = false;

        owc.backend.resume(app)

        // wrapper is destroyed with nothing left inside
    }
}

pub struct EditModeBackendWrapper {
    panel: ManuallyDrop<EditModeWrapPanel>,
    inner: ManuallyDrop<Box<dyn OverlayBackend>>,

    last_render: Instant,
    can_render_inner: bool,
}

impl OverlayBackend for EditModeBackendWrapper {
    fn init(&mut self, app: &mut crate::state::AppState) -> anyhow::Result<()> {
        self.inner.init(app)?;
        self.panel.init(app)
    }
    fn pause(&mut self, app: &mut crate::state::AppState) -> anyhow::Result<()> {
        self.inner.pause(app)?;
        self.panel.pause(app)
    }
    fn resume(&mut self, app: &mut crate::state::AppState) -> anyhow::Result<()> {
        self.inner.resume(app)?;
        self.panel.resume(app)
    }
    fn should_render(&mut self, app: &mut crate::state::AppState) -> anyhow::Result<ShouldRender> {
        {
            let mut local_tasks = self.panel.state.tasks.borrow_mut();
            app.tasks.transfer_from(&mut local_tasks);
        }

        let inner = self.inner.should_render(app)?;

        if !matches!(inner, ShouldRender::Unable)
            && let Some(ref frame_meta) = self.inner.frame_meta()
        {
            let (width_px, height_px) = (frame_meta.extent[0], frame_meta.extent[1]);

            let new_size = vec2(width_px as _, height_px as _);
            if !self.panel.max_size.abs_diff_eq(new_size, 0.1) {
                log::debug!("EditWrapperGui size {} → {new_size}", self.panel.max_size);
                self.panel.max_size = new_size;

                let gui_scale = (new_size.x / 750.0).min(new_size.y / 300.0);
                self.panel.gui_scale = (gui_scale * 4.0).round() / 4.0;
                self.panel.update_layout(app)?;
            }

            self.can_render_inner = true;
        } else {
            self.can_render_inner = false;
            if self.last_render.elapsed() < Duration::from_millis(100) {
                return Ok(ShouldRender::Unable);
            }
        }

        let panel = self.panel.should_render(app)?;

        #[allow(clippy::match_same_arms)]
        Ok(match (inner, panel) {
            (ShouldRender::Should, ShouldRender::Should) => ShouldRender::Should,
            (ShouldRender::Should, ShouldRender::Can) => ShouldRender::Should,
            (ShouldRender::Can, ShouldRender::Should) => ShouldRender::Should,
            (ShouldRender::Can, ShouldRender::Can) => ShouldRender::Can,
            // only if last render was more than 100ms ago.
            (ShouldRender::Unable, ShouldRender::Should) => ShouldRender::Should,
            _ => ShouldRender::Unable,
        })
    }
    fn render(
        &mut self,
        app: &mut crate::state::AppState,
        rdr: &mut RenderResources,
    ) -> anyhow::Result<()> {
        if self.can_render_inner {
            self.inner.render(app, rdr)?;
        }

        self.panel.render(app, rdr)?;
        // `GuiPanel` is not stereo-aware, so just render the same pass twice
        if rdr.cmd_bufs.len() > 1 {
            rdr.cmd_bufs.reverse();
            self.panel.render(app, rdr)?;
            rdr.cmd_bufs.reverse();
        }

        Ok(())
    }
    fn frame_meta(&mut self) -> Option<crate::windowing::backend::FrameMeta> {
        self.inner.frame_meta()
    }
    fn on_hover(
        &mut self,
        app: &mut crate::state::AppState,
        hit: &crate::backend::input::PointerHit,
    ) -> HoverResult {
        // pass through hover events to force pipewire to capture frames for us
        let _ = self.inner.on_hover(app, hit);
        self.panel.on_hover(app, hit)
    }
    fn on_left(&mut self, app: &mut crate::state::AppState, pointer: usize) {
        self.inner.on_left(app, pointer);
        self.panel.on_left(app, pointer);
    }
    fn on_pointer(
        &mut self,
        app: &mut crate::state::AppState,
        hit: &crate::backend::input::PointerHit,
        pressed: bool,
    ) {
        self.panel.on_pointer(app, hit, pressed);
    }
    fn on_scroll(
        &mut self,
        app: &mut crate::state::AppState,
        hit: &crate::backend::input::PointerHit,
        delta: WheelDelta,
    ) {
        self.panel.on_scroll(app, hit, delta);
    }
    fn notify(&mut self, app: &mut AppState, event_data: OverlayEventData) -> anyhow::Result<()> {
        self.panel.notify(app, event_data)
    }
    fn get_interaction_transform(&mut self) -> Option<glam::Affine2> {
        self.inner.get_interaction_transform()
    }
    fn get_attrib(&self, attrib: BackendAttrib) -> Option<BackendAttribValue> {
        self.inner.get_attrib(attrib)
    }
    fn set_attrib(&mut self, app: &mut AppState, value: BackendAttribValue) -> bool {
        self.inner.set_attrib(app, value)
    }
}

#[allow(clippy::too_many_lines)]
fn make_edit_panel(app: &mut AppState) -> anyhow::Result<EditModeWrapPanel> {
    let state = EditModeState {
        id: Rc::new(RefCell::new(OverlayID::null())),
        tasks: Rc::new(RefCell::new(TaskContainer::new())),
        tabs: ButtonPaneTabSwitcher::default(),
        lock: InteractLockHandler::default(),
        pos: SpriteTabHandler::default(),
        stereo: SpriteTabHandler::default(),
        mouse: SpriteTabHandler::default(),
    };

    let anim_mult = app.wgui_theme.animation_mult;

    let on_custom_attrib: OnCustomAttribFunc = Box::new(move |layout, parser, attribs, _app| {
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

                let callback: EventCallback<AppState, EditModeState> = match command {
                    "::EditModeToggleLock" => Box::new(move |common, data, app, state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        let sel = OverlaySelector::Id(*state.id.borrow());
                        let task = state.lock.toggle(common, anim_mult);
                        app.tasks
                            .enqueue(TaskType::Overlay(OverlayTask::Modify(sel, task)));
                        Ok(EventResult::Consumed)
                    }),
                    "::EditModeToggleGrab" => Box::new(move |_common, data, app, state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        let sel = OverlaySelector::Id(*state.id.borrow());
                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                            sel,
                            Box::new(|_app, owc| {
                                let state = owc.active_state.as_mut().unwrap(); //want panic
                                state.grabbable = !state.grabbable;
                            }),
                        )));
                        Ok(EventResult::Consumed)
                    }),
                    "::EditModeTab" => {
                        let tab_name = args.next().unwrap().to_owned();
                        Box::new(move |common, data, app, state| {
                            if !test_button(data) || !test_duration(&button, app) {
                                return Ok(EventResult::Pass);
                            }

                            state.tabs.tab_button_clicked(common, &tab_name);
                            Ok(EventResult::Consumed)
                        })
                    }
                    "::EditModeSetPos" => {
                        let key = args.next().unwrap().to_owned();
                        Box::new(move |common, data, app, state| {
                            if !test_button(data) || !test_duration(&button, app) {
                                return Ok(EventResult::Pass);
                            }

                            let sel = OverlaySelector::Id(*state.id.borrow());
                            let task = state.pos.button_clicked(common, &key);
                            app.tasks
                                .enqueue(TaskType::Overlay(OverlayTask::Modify(sel, task)));
                            Ok(EventResult::Consumed)
                        })
                    }
                    "::EditModeSetStereo" => {
                        let key = args.next().unwrap().to_owned();
                        Box::new(move |common, data, app, state| {
                            if !test_button(data) || !test_duration(&button, app) {
                                return Ok(EventResult::Pass);
                            }

                            let sel = OverlaySelector::Id(*state.id.borrow());
                            let task = state.stereo.button_clicked(common, &key);
                            app.tasks
                                .enqueue(TaskType::Overlay(OverlayTask::Modify(sel, task)));
                            Ok(EventResult::Consumed)
                        })
                    }
                    "::EditModeSetStereoFullFrame" => {
                        let full_frame = args.next().unwrap().parse::<bool>().unwrap();
                        Box::new(move |_common, data, app, state| {
                            if !test_button(data) || !test_duration(&button, app) {
                                return Ok(EventResult::Pass);
                            }

                            let sel = OverlaySelector::Id(*state.id.borrow());
                            app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                                sel,
                                Box::new(move |app, owc| {
                                    let attrib = BackendAttribValue::StereoFullFrame(full_frame);
                                    owc.backend.set_attrib(app, attrib);
                                }),
                            )));
                            Ok(EventResult::Consumed)
                        })
                    }
                    "::EditModeSetMouse" => {
                        let key = args.next().unwrap().to_owned();
                        Box::new(move |common, data, app, state| {
                            if !test_button(data) || !test_duration(&button, app) {
                                return Ok(EventResult::Pass);
                            }

                            let sel = OverlaySelector::Id(*state.id.borrow());
                            let task = state.mouse.button_clicked(common, &key);
                            app.tasks
                                .enqueue(TaskType::Overlay(OverlayTask::Modify(sel, task)));
                            Ok(EventResult::Consumed)
                        })
                    }
                    "::EditModeDelete" => Box::new(move |_common, data, app, state| {
                        if !test_button(data) || !test_duration(&button, app) {
                            return Ok(EventResult::Pass);
                        }

                        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
                            OverlaySelector::Id(*state.id.borrow()),
                            Box::new(move |_app, owc| {
                                owc.active_state = None;
                            }),
                        )));
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
        "gui/edit.xml",
        state,
        NewGuiPanelParams {
            on_custom_attrib: Some(on_custom_attrib),
            resize_to_parent: true,
            ..Default::default()
        },
    )?;

    panel.state.pos = new_pos_tab_handler(&mut panel)?;
    panel.state.stereo = new_stereo_tab_handler(&mut panel)?;
    panel.state.mouse = new_mouse_tab_handler(&mut panel)?;
    panel.state.lock = InteractLockHandler::new(&mut panel)?;
    panel.state.tabs = ButtonPaneTabSwitcher::new(
        &mut panel,
        &["none", "pos", "alpha", "curve", "stereo", "mouse"],
    )?;

    set_up_checkbox(&mut panel, "additive_box", cb_assign_additive)?;
    set_up_checkbox(&mut panel, "align_box", cb_assign_align)?;
    set_up_checkbox(&mut panel, "global_box", cb_assign_global)?;
    set_up_checkbox(&mut panel, "angle_fade_box", cb_assign_angle_fade)?;
    set_up_checkbox(&mut panel, "block_input_box", cb_assign_block_input)?;
    set_up_checkbox(
        &mut panel,
        "stereo_full_frame_box",
        cb_assign_stereo_full_frame,
    )?;
    set_up_checkbox(
        &mut panel,
        "stereo_adjust_mouse_box",
        cb_assign_stereo_adjust_mouse,
    )?;
    set_up_slider(&mut panel, "lerp_slider", cb_assign_lerp)?;
    set_up_slider(&mut panel, "alpha_slider", cb_assign_alpha)?;
    set_up_slider(&mut panel, "curve_slider", cb_assign_curve)?;

    Ok(panel)
}

fn reset_panel(
    panel: &mut EditModeWrapPanel,
    id: OverlayID,
    owc: &mut OverlayWindowConfig,
) -> anyhow::Result<()> {
    *panel.state.id.borrow_mut() = id;
    let state = owc.active_state.as_mut().unwrap();

    let mut com = panel.layout.common();

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentButton>("top_grab")?;
    c.set_sticky_state(&mut com, !state.grabbable);

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentSlider>("lerp_slider")?;
    c.set_value(&mut com, state.positioning.get_lerp().unwrap_or(1.0));

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentSlider>("alpha_slider")?;
    c.set_value(&mut com, state.alpha);

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentSlider>("curve_slider")?;
    c.set_value(&mut com, state.curvature.unwrap_or(0.0));

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentCheckbox>("additive_box")?;
    c.set_checked(&mut com, state.additive);

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentCheckbox>("align_box")?;
    c.set_checked(&mut com, state.positioning.get_align().unwrap_or(false));

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentCheckbox>("global_box")?;
    c.set_checked(&mut com, owc.global);

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentCheckbox>("angle_fade_box")?;
    c.set_checked(&mut com, state.angle_fade);

    let c = panel
        .parser_state
        .fetch_component_as::<ComponentCheckbox>("block_input_box")?;
    c.set_checked(&mut com, state.block_input);

    panel.state.pos.reset(&mut com, &state.positioning.into());
    panel.state.lock.reset(&mut com, state.interactable);
    panel.state.tabs.reset(&mut com);

    if let Some(stereo) = attrib_value!(
        owc.backend.get_attrib(BackendAttrib::Stereo),
        BackendAttribValue::Stereo
    ) {
        panel.state.tabs.set_tab_visible(&mut com, "stereo", true);
        panel.state.stereo.reset(&mut com, &stereo);

        // Set the checkbox label based on stereo mode
        let translation = get_stereo_full_frame_translation(stereo);
        let c = panel
            .parser_state
            .fetch_component_as::<ComponentCheckbox>("stereo_full_frame_box")?;
        c.set_text(&mut com, Translation::from_translation_key(translation));
    } else {
        panel.state.tabs.set_tab_visible(&mut com, "stereo", false);
    }

    if let Some(mouse) = attrib_value!(
        owc.backend.get_attrib(BackendAttrib::MouseTransform),
        BackendAttribValue::MouseTransform
    ) {
        panel.state.tabs.set_tab_visible(&mut com, "mouse", true);
        panel.state.mouse.reset(&mut com, &mouse);
    } else {
        panel.state.tabs.set_tab_visible(&mut com, "mouse", false);
    }

    if let Some(full_frame) = attrib_value!(
        owc.backend.get_attrib(BackendAttrib::StereoFullFrame),
        BackendAttribValue::StereoFullFrame
    ) {
        let c = panel
            .parser_state
            .fetch_component_as::<ComponentCheckbox>("stereo_full_frame_box")?;
        c.set_checked(&mut com, full_frame);
    }

    if let Some(adjust_mouse) = attrib_value!(
        owc.backend.get_attrib(BackendAttrib::StereoAdjustMouse),
        BackendAttribValue::StereoAdjustMouse
    ) {
        let c = panel
            .parser_state
            .fetch_component_as::<ComponentCheckbox>("stereo_adjust_mouse_box")?;
        c.set_checked(&mut com, adjust_mouse);
    }

    Ok(())
}

const fn cb_assign_lerp(_app: &mut AppState, owc: &mut OverlayWindowConfig, lerp: f32) {
    owc.dirty = true;
    let active_state = owc.active_state.as_mut().unwrap();
    active_state.positioning = active_state.positioning.with_lerp(lerp);
}

const fn cb_assign_alpha(_app: &mut AppState, owc: &mut OverlayWindowConfig, alpha: f32) {
    owc.dirty = true;
    owc.active_state.as_mut().unwrap().alpha = alpha;
}

fn cb_assign_curve(_app: &mut AppState, owc: &mut OverlayWindowConfig, curvature: f32) {
    owc.dirty = true;
    owc.active_state.as_mut().unwrap().curvature = if curvature < 0.005 {
        None
    } else {
        Some(curvature)
    };
}

const fn cb_assign_additive(_app: &mut AppState, owc: &mut OverlayWindowConfig, additive: bool) {
    owc.dirty = true;
    owc.active_state.as_mut().unwrap().additive = additive;
}

const fn cb_assign_align(_app: &mut AppState, owc: &mut OverlayWindowConfig, align: bool) {
    owc.dirty = true;
    let active_state = owc.active_state.as_mut().unwrap();
    active_state.positioning = active_state.positioning.with_align(align);
}

const fn cb_assign_global(_app: &mut AppState, owc: &mut OverlayWindowConfig, global: bool) {
    owc.global = global;
}

const fn cb_assign_angle_fade(
    _app: &mut AppState,
    owc: &mut OverlayWindowConfig,
    angle_fade: bool,
) {
    owc.active_state.as_mut().unwrap().angle_fade = angle_fade;
}

const fn cb_assign_block_input(
    _app: &mut AppState,
    owc: &mut OverlayWindowConfig,
    block_input: bool,
) {
    owc.active_state.as_mut().unwrap().block_input = block_input;
}

fn cb_assign_stereo_full_frame(
    app: &mut AppState,
    owc: &mut OverlayWindowConfig,
    full_frame: bool,
) {
    owc.dirty = true;
    let attrib = BackendAttribValue::StereoFullFrame(full_frame);
    owc.backend.set_attrib(app, attrib);
}

fn cb_assign_stereo_adjust_mouse(
    app: &mut AppState,
    owc: &mut OverlayWindowConfig,
    adjust_mouse: bool,
) {
    owc.dirty = true;
    let attrib = BackendAttribValue::StereoAdjustMouse(adjust_mouse);
    owc.backend.set_attrib(app, attrib);
}

fn set_up_slider(
    panel: &mut EditModeWrapPanel,
    id: &str,
    callback: fn(&mut AppState, &mut OverlayWindowConfig, f32),
) -> anyhow::Result<()> {
    let slider = panel
        .parser_state
        .fetch_component_as::<ComponentSlider>(id)?;
    let tasks = panel.state.tasks.clone();
    let overlay_id = panel.state.id.clone();
    slider.on_value_changed(Box::new(move |_common, e| {
        let mut tasks = tasks.borrow_mut();
        let e_value = e.value;

        tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
            OverlaySelector::Id(*overlay_id.borrow()),
            Box::new(move |app, owc| callback(app, owc, e_value)),
        )));
    }));

    Ok(())
}

fn set_up_checkbox(
    panel: &mut EditModeWrapPanel,
    id: &str,
    callback: fn(&mut AppState, &mut OverlayWindowConfig, bool),
) -> anyhow::Result<()> {
    let checkbox = panel
        .parser_state
        .fetch_component_as::<ComponentCheckbox>(id)?;
    let tasks = panel.state.tasks.clone();
    let overlay_id = panel.state.id.clone();
    checkbox.on_toggle(Box::new(move |_common, e| {
        let mut tasks = tasks.borrow_mut();
        let e_checked = e.checked;

        tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
            OverlaySelector::Id(*overlay_id.borrow()),
            Box::new(move |app, owc| callback(app, owc, e_checked)),
        )));
        Ok(())
    }));

    Ok(())
}
