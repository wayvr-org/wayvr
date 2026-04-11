use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
    sync::atomic::Ordering,
};

use anyhow::Context;
use glam::{Affine3A, Quat, Vec3, Vec3A};
use slotmap::{Key, SecondaryMap, SlotMap};
use wgui::animation::AnimationEasing;
use wgui::log::LogErr;
use wlx_common::{
    astr_containers::{AStrMap, AStrMapExt},
    config::SerializedWindowSet,
    overlays::{BackendAttrib, BackendAttribValue, ToastTopic},
    timestep::get_micros,
};

use crate::{
    FRAME_COUNTER,
    backend::task::{OverlayTask, ToggleMode},
    config::save_state,
    overlays::{
        anchor::{create_anchor, create_grab_help},
        custom::create_custom,
        dashboard::{DASH_NAME, create_dash_frontend},
        edit::EditWrapperManager,
        keyboard::create_keyboard,
        screen::create_screens,
        toast::Toast,
        watch::create_watch,
    },
    state::AppState,
    windowing::{
        OverlayID, OverlaySelector,
        backend::{OverlayEventData, OverlayMeta},
        set::OverlayWindowSet,
        snap_upright,
        window::{OverlayCategory, OverlayWindowData},
    },
};

use wlx_common::windowing::OverlayWindowState;

pub const MAX_OVERLAY_SETS: usize = 6;

struct FocusAnimation {
    oid: OverlayID,
    from: OverlayWindowState,
    to: OverlayWindowState,
    start_us: u64,
    duration_us: u64,
    easing: AnimationEasing,
}

#[derive(Clone)]
struct FocusedScreenState {
    name: Arc<str>,
    oid: OverlayID,
    saved_state: OverlayWindowState,
    saved_crop_rect: [f32; 4],
    focus_anchor: Affine3A,
    target_x: f32,
    target_y: f32,
    crop_rect: [f32; 4],
}

pub struct OverlayWindowManager<T> {
    wrappers: EditWrapperManager,
    overlays: SlotMap<OverlayID, OverlayWindowData<T>>,
    sets: Vec<OverlayWindowSet>,
    global_set: OverlayWindowSet,
    /// The set that is currently visible.
    current_set: Option<usize>,
    /// The set that will be restored by show_hide.
    /// Usually the same as current_set, except it keeps its value when current_set is hidden.
    restore_set: usize,
    anchor_local: Affine3A,
    watch_id: OverlayID,
    keyboard_id: OverlayID,
    edit_mode: bool,
    dropped_overlays: VecDeque<OverlayWindowData<T>>,
    initialized: bool,
    focused_screen: Option<FocusedScreenState>,
    focus_animation: Option<FocusAnimation>,
}

impl<T> OverlayWindowManager<T>
where
    T: Default,
{
    pub fn new(app: &mut AppState, headless: bool) -> anyhow::Result<Self> {
        let mut me = Self {
            wrappers: EditWrapperManager::default(),
            overlays: SlotMap::<OverlayID, OverlayWindowData<T>>::with_key(),
            current_set: Some(0),
            restore_set: 0,
            sets: vec![OverlayWindowSet::default()],
            global_set: OverlayWindowSet::default(),
            anchor_local: Affine3A::from_translation(Vec3::NEG_Z),
            watch_id: OverlayID::null(),
            keyboard_id: OverlayID::null(),
            edit_mode: false,
            dropped_overlays: VecDeque::with_capacity(8),
            initialized: false,
            focused_screen: None,
            focus_animation: None,
        };

        let mut wayland = false;

        if headless {
            log::info!("Running in headless mode; keyboard will be en-US");
        } else {
            // create one window set for each screen.
            // this is the default and would be overwritten by
            // OverlayWindowManager::restore_layout down below
            match create_screens(app) {
                Ok((data, is_wayland)) => {
                    let last_idx = data.screens.len() - 1;
                    for (idx, (meta, mut config)) in data.screens.into_iter().enumerate() {
                        config.show_on_spawn = true;
                        me.add(OverlayWindowData::from_config(config), app);

                        if idx < last_idx {
                            me.sets.push(OverlayWindowSet::default());
                            me.switch_to_set(app, Some(me.current_set.unwrap() + 1), false);
                        }
                        app.screens.push(meta);
                    }

                    wayland = is_wayland;
                }
                Err(e) => log::error!("Unable to initialize screens: {e:?}"),
            }
        }

        let mut keyboard = OverlayWindowData::from_config(create_keyboard(app, wayland)?);
        keyboard.config.show_on_spawn = true;
        me.keyboard_id = me.add(keyboard, app);

        // is this needed?
        me.switch_to_set(app, None, false);

        // copy keyboard to all sets
        let kbd_state = me
            .sets
            .last()
            .and_then(|s| s.overlays.get(me.keyboard_id))
            .unwrap()
            .clone();
        for set in &mut me.sets {
            set.overlays.insert(me.keyboard_id, kbd_state.clone());
        }

        let anchor = OverlayWindowData::from_config(create_anchor(app)?);
        me.add(anchor, app);

        let watch = OverlayWindowData::from_config(create_watch(app)?);
        me.watch_id = me.add(watch, app);

        let dash_frontend = OverlayWindowData::from_config(create_dash_frontend(app)?);
        me.add(dash_frontend, app);

        let grab_help = OverlayWindowData::from_config(create_grab_help(app)?);
        me.add(grab_help, app);

        let custom_panels = app.session.config.custom_panels.clone();
        for name in custom_panels {
            let Some(panel) = create_custom(app, name) else {
                continue;
            };
            log::info!("Loaded custom panel '{}'", panel.name);
            me.add(OverlayWindowData::from_config(panel), app);
        }

        // overwrite default layout with saved layout, if exists
        me.restore_layout(app);
        me.overlays_changed(app)?;

        for id in [me.watch_id, me.keyboard_id] {
            for ev in [
                OverlayEventData::NumSetsChanged(me.sets.len()),
                OverlayEventData::EditModeChanged(false),
                OverlayEventData::DevicesChanged,
            ] {
                me.mut_by_id(id).unwrap().config.backend.notify(app, ev)?;
            }
        }

        me.initialized = true;

        Ok(me)
    }

    #[allow(clippy::too_many_lines)]
    pub fn handle_task(&mut self, app: &mut AppState, task: OverlayTask) -> anyhow::Result<()> {
        match task {
            OverlayTask::ShowHide => self.show_hide(app),
            OverlayTask::ToggleSet(set) => {
                self.switch_or_toggle_set(app, set);
            }
            OverlayTask::SwitchSet(maybe_set) => {
                self.switch_to_set(app, maybe_set, false);
            }
            OverlayTask::ResetOverlay(sel) => {
                if let Some(o) = self.mut_by_selector(&sel) {
                    let was_active = o.config.is_active();
                    o.config.activate(app);
                    if !was_active {
                        self.visible_overlays_changed(app)?;
                    }
                }
            }
            OverlayTask::ToggleOverlay(sel, mode) => {
                let Some(id) = self.id_by_selector(&sel) else {
                    log::warn!("Overlay not found for task: {sel:?}");
                    return Ok(());
                };

                let o = &mut self.overlays[id];

                match mode {
                    ToggleMode::EnsureOn if o.config.is_active() => return Ok(()),
                    ToggleMode::EnsureOff if !o.config.is_active() => return Ok(()),
                    _ => {}
                }

                let parent_set = if o.config.global {
                    &mut self.global_set
                } else {
                    &mut self.sets[self.restore_set]
                };

                if let Some(active_state) = o.config.active_state.take() {
                    log::debug!("{}: toggle off", o.config.name);

                    parent_set
                        .hidden_overlays
                        .arc_set(o.config.name.clone(), active_state);
                } else if let Some(state) = parent_set.hidden_overlays.arc_rm(&o.config.name) {
                    let o = &mut self.overlays[id];
                    log::debug!("{}: toggle on", o.config.name);
                    o.config.dirty = true;
                    o.config.active_state = Some(state);
                    o.config.reset(app, false);
                } else {
                    // no saved state
                    o.config.activate(app);
                }
                self.visible_overlays_changed(app)?;

                return Ok(());
            }
            OverlayTask::ToggleEditMode => {
                self.set_edit_mode(!self.edit_mode, app)?;
            }
            OverlayTask::ToggleDashboard => {
                if let Some(overlay) =
                    self.mut_by_selector(&OverlaySelector::Name(DASH_NAME.into()))
                {
                    if overlay.config.active_state.is_none() {
                        overlay.config.activate(app);
                    } else {
                        overlay.config.deactivate();
                    }
                    self.visible_overlays_changed(app)?;
                }
            }
            OverlayTask::AddSet => {
                let new_idx = self.sets.len();
                if new_idx >= MAX_OVERLAY_SETS {
                    Toast::new(
                        ToastTopic::System,
                        "TOAST.CANNOT_ADD_SET".into(),
                        "TOAST.MAXIMUM_SETS_REACHED".into(),
                    )
                    .with_timeout(5.)
                    .with_sound(true)
                    .submit(app);
                    return Ok(());
                }
                self.sets.push(OverlayWindowSet::default());
                self.switch_to_set(app, Some(new_idx), false);
                self.overlays[self.keyboard_id].config.activate(app);
                self.sets_changed(app);
                self.visible_overlays_changed(app)?;
            }
            OverlayTask::DeleteActiveSet => {
                let Some(set) = self.current_set else {
                    Toast::new(
                        ToastTopic::System,
                        "TOAST.CANNOT_REMOVE_SET".into(),
                        "TOAST.NO_SET_SELECTED".into(),
                    )
                    .with_timeout(5.)
                    .with_sound(true)
                    .submit(app);
                    return Ok(());
                };

                if self.sets.len() <= 1 {
                    Toast::new(
                        ToastTopic::System,
                        "TOAST.CANNOT_REMOVE_SET".into(),
                        "TOAST.LAST_EXISTING_SET".into(),
                    )
                    .with_timeout(5.)
                    .with_sound(true)
                    .submit(app);
                    return Ok(());
                }

                self.switch_to_set(app, None, false);
                self.sets.remove(set);
                self.restore_set = 0;
                self.sets_changed(app);
            }
            OverlayTask::SettingsChanged => {
                for o in self.overlays.values_mut() {
                    let _ = o
                        .config
                        .backend
                        .notify(app, OverlayEventData::SettingsChanged)
                        .log_err("Could not notify SettingsChanged");
                }
            }
            OverlayTask::KeyboardChanged => {
                self.overlays_changed(app)?;
                self.sets_changed(app);
            }
            OverlayTask::CleanupMirrors => {
                let mut ids_to_remove = vec![];
                for (oid, o) in &self.overlays {
                    if !matches!(o.config.category, OverlayCategory::Mirror) {
                        continue;
                    }
                    if o.config.active_state.is_some() {
                        continue;
                    }
                    ids_to_remove.push(oid);
                }

                for oid in ids_to_remove {
                    self.remove_by_selector(&OverlaySelector::Id(oid), app);
                }
            }
            OverlayTask::Modify(sel, f) => {
                if let Some(o) = self.mut_by_selector(&sel) {
                    let was_visible = o.config.is_active();
                    f(app, &mut o.config);

                    if was_visible != o.config.is_active() {
                        let _ = self.visible_overlays_changed(app);
                    }
                } else {
                    log::warn!("Overlay not found for task: {sel:?}");
                }
            }
            OverlayTask::Create(sel, f) => {
                let None = self.mut_by_selector(&sel) else {
                    log::debug!("Could not create {sel:?}: exists");
                    return Ok(());
                };

                let Some(overlay_config) = f(app) else {
                    log::debug!("Could not create {sel:?}: empty config");
                    return Ok(());
                };

                self.add(
                    OverlayWindowData {
                        birthframe: FRAME_COUNTER.load(Ordering::Relaxed),
                        ..OverlayWindowData::from_config(overlay_config)
                    },
                    app,
                );
            }
            OverlayTask::Drop(sel) => {
                if let Some(o) = self.mut_by_selector(&sel)
                    && o.birthframe < FRAME_COUNTER.load(Ordering::Relaxed)
                    && let Some(o) = self.remove_by_selector(&sel, app)
                {
                    log::debug!("Dropping overlay {}", o.config.name);
                    self.dropped_overlays.push_back(o);
                }
            }
            OverlayTask::ModifyPanel(task) => {
                if let Some(oid) = self.lookup(&task.overlay)
                    && let Some(o) = self.mut_by_id(oid)
                {
                    if !matches!(
                        o.config.category,
                        OverlayCategory::Panel
                            | OverlayCategory::Keyboard
                            | OverlayCategory::Internal
                    ) {
                        log::warn!(
                            "Received command for '{}', but this overlay does not support commands",
                            &task.overlay
                        );
                        return Ok(());
                    }

                    o.config.backend.notify(
                        app,
                        OverlayEventData::CustomCommand {
                            element: task.element,
                            command: task.command,
                        },
                    )?;
                }
            }
            OverlayTask::ScreenFocusToggle(screen_focus) => {
                self.handle_screen_focus_toggle(app, screen_focus)?;
            }
        }
        Ok(())
    }
}

const SAVED_ATTRIBS: [BackendAttrib; 3] = [
    BackendAttrib::Stereo,
    BackendAttrib::StereoFullFrame,
    BackendAttrib::MouseTransform,
];

impl<T> OverlayWindowManager<T> {
    pub fn animate_focus_transitions(&mut self, app: &mut AppState) {
        if let Some(anim) = self.focus_animation.as_ref() {
            let raw = if anim.duration_us == 0 {
                1.0
            } else {
                ((get_micros().saturating_sub(anim.start_us)) as f32 / anim.duration_us as f32)
                    .clamp(0.0, 1.0)
            };
            let pos = anim.easing.interpolate(raw);
            let is_done = raw >= 1.0;
            let oid = anim.oid;
            let state = interpolate_overlay_state(&anim.from, &anim.to, pos);

            if let Some(overlay) = self.mut_by_id(oid) {
                overlay.config.active_state = Some(state);
                overlay.config.dirty = true;
            }

            if is_done {
                self.focus_animation = None;
            }
            return;
        }

        if let Some(focused) = self.focused_screen.clone() {
            if let Some(overlay) = self.mut_by_id(focused.oid) {
                let aspect_ratio = overlay
                    .frame_meta()
                    .map(|meta| meta.extent[0] as f32 / meta.extent[1] as f32)
                    .unwrap_or(1.0)
                    .max(0.01);
                let refreshed_focus_state = build_focused_screen_state(
                    &focused.saved_state,
                    app,
                    focused.focus_anchor,
                    aspect_ratio,
                    focused.target_x,
                    focused.target_y,
                    focused.crop_rect,
                );
                let assisted_state = apply_focus_look_assist(&refreshed_focus_state, app);
                overlay.config.active_state = Some(assisted_state);
                overlay.config.dirty = true;
            }
        }
    }

    pub fn pop_dropped(&mut self) -> Option<OverlayWindowData<T>> {
        self.dropped_overlays.pop_front()
    }

    pub fn persist_layout(&mut self, app: &mut AppState) {
        app.session.config.global_set.clear();
        app.session.config.sets.clear();
        app.session.config.sets.reserve(self.sets.len());
        app.session.config.last_set = self.restore_set as _;

        // only safe to save when current_set is None
        let restore_after = if self.current_set.is_some() {
            self.switch_to_set(app, None, true);
            true
        } else {
            false
        };

        for set in &self.sets {
            let mut overlays: HashMap<_, _> = set
                .overlays
                .iter()
                .filter_map(|(k, v)| {
                    let n = self.overlays.get(k).map(|o| o.config.name.clone())?;
                    Some((n, v.clone()))
                })
                .collect();

            // overlays that we haven't seen since startup (e.g. wayvr apps)
            for (k, o) in &set.inactive_overlays {
                if !overlays.contains_key(k) {
                    overlays.insert(k.clone(), o.clone());
                }
            }

            let hidden_overlays: HashMap<_, _> = set.hidden_overlays.iter().cloned().collect();

            let serialized = SerializedWindowSet {
                name: set.name.clone(),
                overlays,
                hidden_overlays,
            };
            app.session.config.sets.push(serialized);
        }

        // global overlays; watch, toast
        for oid in &[self.watch_id] {
            let Some(o) = self.get_by_id(*oid) else {
                break;
            };
            let Some(state) = o.config.active_state.clone() else {
                break;
            };
            app.session
                .config
                .global_set
                .insert(o.config.name.clone(), state.clone());
        }

        // BackendAttrib
        for o in self.overlays.values() {
            app.session.config.attribs.arc_set(
                o.config.name.clone(),
                SAVED_ATTRIBS
                    .iter()
                    .filter_map(|a| o.config.backend.get_attrib(*a))
                    .filter(|val| !val.is_default())
                    .collect(),
            );
        }

        if restore_after {
            self.switch_to_set(app, Some(self.restore_set), true);
        }
    }

    pub fn restore_layout(&mut self, app: &mut AppState) {
        if app.session.config.sets.is_empty() {
            // keep defaults
            return;
        }

        // only safe to load when current_set is None
        if self.current_set.is_some() {
            self.switch_to_set(app, None, false);
        }

        self.sets.clear();
        self.sets.reserve(app.session.config.sets.len());

        for (i, s) in app.session.config.sets.iter().enumerate() {
            let mut overlays = SecondaryMap::new();
            let mut inactive_overlays = AStrMap::new();

            for (name, o) in &s.overlays {
                if let Some(id) = self.lookup(name) {
                    log::debug!("set {i}: loaded state for {name}");
                    overlays.insert(id, o.clone());
                } else {
                    log::debug!(
                        "set {i} has saved state for {name} which doesn't exist. will apply state once added."
                    );
                    inactive_overlays.arc_set(name.clone(), o.clone());
                }
            }

            let hidden_overlays: AStrMap<_> = s
                .hidden_overlays
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            self.sets.push(OverlayWindowSet {
                name: s.name.clone(),
                overlays,
                inactive_overlays,
                hidden_overlays,
            });
        }

        // global overlays
        for (name, ows) in app.session.config.global_set.clone() {
            let ows = ows.clone();

            if let Some(oid) = self.lookup(&name)
                && let Some(o) = self.mut_by_id(oid)
            {
                o.config.global = true;
                if o.config.active_state.is_none() {
                    self.global_set.hidden_overlays.arc_set(name.clone(), ows);
                } else {
                    o.config.active_state = Some(ows);
                    o.config.reset(app, false);
                }
                log::debug!("global set: loaded state for {name}");
            } else {
                log::debug!(
                    "global set has saved state for {name} which doesn't exist. will apply state once added."
                );
                self.global_set
                    .inactive_overlays
                    .arc_set(name.clone(), ows.clone());
            }
        }

        for (name, attribs) in &app.session.config.attribs.clone() {
            let Some(oid) = self.lookup(name) else {
                continue;
            };
            let Some(o) = self.mut_by_id(oid) else {
                continue;
            };

            for value in attribs {
                o.config.backend.set_attrib(app, value.clone());
            }
        }

        self.restore_set = (app.session.config.last_set as usize).min(self.sets.len() - 1);
    }

    pub const fn get_edit_mode(&self) -> bool {
        self.edit_mode
    }

    pub const fn get_current_set(&self) -> Option<usize> {
        self.current_set
    }

    pub const fn get_total_sets(&self) -> usize {
        self.sets.len()
    }

    pub fn set_edit_mode(&mut self, enabled: bool, app: &mut AppState) -> anyhow::Result<()> {
        let changed = enabled != self.edit_mode;
        self.edit_mode = enabled;
        if !enabled {
            for o in self.overlays.values_mut() {
                self.wrappers.unwrap_edit_mode(&mut o.config, app)?;
            }

            if changed {
                self.persist_layout(app);
                if let Err(e) = save_state(&app.session.config) {
                    log::error!("Could not save state: {e:?}");
                }
            }
        }
        if changed && let Some(watch) = self.mut_by_id(self.watch_id) {
            watch
                .config
                .active_state
                .iter_mut()
                .for_each(|f| f.grabbable = enabled);
            watch
                .config
                .backend
                .notify(app, OverlayEventData::EditModeChanged(enabled))?;
        }
        Ok(())
    }

    pub fn edit_overlay(&mut self, id: OverlayID, enabled: bool, app: &mut AppState) {
        if !self.edit_mode {
            return;
        }

        let Some(overlay) = self.overlays.get_mut(id) else {
            return;
        };

        if matches!(
            overlay.config.category,
            OverlayCategory::Internal | OverlayCategory::Dashboard
        ) {
            // watch, anchor, toast, dashboard
            return;
        }

        if enabled {
            self.wrappers
                .wrap_edit_mode(id, &mut overlay.config, app)
                .inspect_err(|e| log::error!("{e:?}"))
                .unwrap(); // FIXME: unwrap
        } else {
            self.wrappers
                .unwrap_edit_mode(&mut overlay.config, app)
                .inspect_err(|e| log::error!("{e:?}"))
                .unwrap(); // FIXME: unwrap
        }
    }

    pub fn id_by_selector(&self, selector: &OverlaySelector) -> Option<OverlayID> {
        match selector {
            OverlaySelector::Id(id) => Some(*id),
            OverlaySelector::Name(name) => self.lookup(name),
            _ => None,
        }
    }

    pub fn mut_by_selector(
        &mut self,
        selector: &OverlaySelector,
    ) -> Option<&mut OverlayWindowData<T>> {
        self.id_by_selector(selector)
            .and_then(|id| self.mut_by_id(id))
    }

    fn remove_by_selector(
        &mut self,
        selector: &OverlaySelector,
        app: &mut AppState,
    ) -> Option<OverlayWindowData<T>> {
        let id = match selector {
            OverlaySelector::Id(id) => *id,
            OverlaySelector::Name(name) => self.lookup(name)?,
            _ => return None,
        };

        let ret_val = self.overlays.remove(id);
        let internal = ret_val.as_ref().is_some_and(|o| {
            matches!(
                o.config.category,
                OverlayCategory::Internal | OverlayCategory::Keyboard | OverlayCategory::Dashboard
            )
        });

        if !internal && let Err(e) = self.overlays_changed(app) {
            log::error!("Error while removing overlay: {e:?}");
        }

        ret_val
    }

    pub fn get_by_id(&mut self, id: OverlayID) -> Option<&OverlayWindowData<T>> {
        self.overlays.get(id)
    }

    pub fn mut_by_id(&mut self, id: OverlayID) -> Option<&mut OverlayWindowData<T>> {
        self.overlays.get_mut(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (OverlayID, &'_ OverlayWindowData<T>)> {
        self.overlays.iter()
    }

    #[allow(dead_code)]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (OverlayID, &'_ mut OverlayWindowData<T>)> {
        self.overlays.iter_mut()
    }

    pub fn values(&self) -> impl Iterator<Item = &'_ OverlayWindowData<T>> {
        self.overlays.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &'_ mut OverlayWindowData<T>> {
        self.overlays.values_mut()
    }

    pub fn lookup(&self, name: &str) -> Option<OverlayID> {
        self.overlays
            .iter()
            .find(|(_, v)| v.config.name.as_ref() == name)
            .map(|(k, _)| k)
    }

    pub fn add(&mut self, mut overlay: OverlayWindowData<T>, app: &mut AppState) -> OverlayID {
        while self.lookup(&overlay.config.name).is_some() {
            log::error!(
                "An overlay with name {} already exists. Deduplicating, but things may break!",
                overlay.config.name
            );
            overlay.config.name = format!("{}_2", overlay.config.name).into();
        }

        let name = overlay.config.name.clone();
        let global = overlay.config.global;
        let internal = matches!(overlay.config.category, OverlayCategory::Internal);
        let show_on_spawn = overlay.config.show_on_spawn;

        let oid = self.overlays.insert(overlay);
        let mut shown = false;

        if !global {
            for (i, set) in self.sets.iter_mut().enumerate() {
                let Some(state) = set.inactive_overlays.arc_rm(&name) else {
                    continue;
                };
                if self.current_set == Some(i) {
                    let o = &mut self.overlays[oid];
                    o.config.active_state = Some(state);
                    o.config.reset(app, false);
                    shown = true;
                    log::debug!("loaded state for {name} to active set!");
                } else {
                    set.overlays.insert(oid, state);
                    log::debug!("loaded state for {name} to set {i}");
                }
            }
        }

        self.overlays[oid]
            .config
            .backend
            .notify(app, OverlayEventData::IdAssigned(oid))
            .unwrap(); // IdAssigned not expected to fail

        if !shown && show_on_spawn {
            log::debug!("activating {name} due to show_on_spawn");
            self.overlays[oid].config.activate(app);
        }
        if !internal && let Err(e) = self.overlays_changed(app) {
            log::error!("Error while adding overlay: {e:?}");
        }
        if !internal && let Err(e) = self.visible_overlays_changed(app) {
            log::error!("Error while adding overlay: {e:?}");
        }
        oid
    }

    pub fn switch_or_toggle_set(&mut self, app: &mut AppState, set: usize) {
        let new_set = if self.current_set.iter().any(|cur| *cur == set) {
            None
        } else {
            Some(set)
        };

        self.switch_to_set(app, new_set, false);
    }

    pub fn switch_to_set(
        &mut self,
        app: &mut AppState,
        new_set: Option<usize>,
        keep_transforms: bool,
    ) {
        if new_set == self.current_set || new_set.is_some_and(|x| x >= self.sets.len()) {
            return;
        }

        if let Some(current_set) = self.current_set.as_ref() {
            let ws = &mut self.sets[*current_set];
            for (id, data) in self.overlays.iter_mut().filter(|(_, d)| !d.config.global) {
                if let Some(state) = data.config.active_state.take() {
                    log::debug!("{}: active_state → ws{}", data.config.name, current_set);
                    ws.overlays.insert(id, state);
                }
            }
        }

        if let Some(new_set) = new_set {
            let mut num_overlays = 0;
            let ws = &mut self.sets[new_set];
            for (id, data) in self.overlays.iter_mut().filter(|(_, d)| !d.config.global) {
                if let Some(state) = ws.overlays.remove(id) {
                    log::debug!("{}: ws{} → active_state", data.config.name, new_set);
                    data.config.active_state = Some(state);
                    if !keep_transforms {
                        data.config.reset(app, false);
                    }
                    if !matches!(
                        data.config.category,
                        OverlayCategory::Internal
                            | OverlayCategory::Keyboard
                            | OverlayCategory::Dashboard
                    ) {
                        num_overlays += 1;
                    }
                }
            }
            ws.overlays.clear();
            self.restore_set = new_set;

            if !self.edit_mode && self.initialized && num_overlays < 1 {
                Toast::new(
                    ToastTopic::System,
                    "TOAST.EMPTY_SET".into(),
                    "TOAST.LETS_ADD_OVERLAYS".into(),
                )
                .with_timeout(3.)
                .submit(app);
            }
        }
        self.current_set = new_set;

        for id in [self.watch_id, self.keyboard_id] {
            let _ = self.mut_by_id(id).context("Missing overlay").and_then(|o| {
                o.config
                    .backend
                    .notify(app, OverlayEventData::ActiveSetChanged(new_set))
            });

            let _ = self
                .visible_overlays_changed(app)
                .inspect_err(|e| log::error!("VisibleOverlaysChanged: {e:?}"));
        }
    }

    pub fn show_hide(&mut self, app: &mut AppState) {
        if self.current_set.is_none() {
            let hmd = snap_upright(app.input_state.hmd, Vec3A::Y);
            app.anchor = hmd * self.anchor_local;

            self.switch_to_set(app, Some(self.restore_set), false);
        } else {
            self.switch_to_set(app, None, false);
        }

        let _ = self
            .visible_overlays_changed(app)
            .inspect_err(|e| log::error!("VisibleOverlaysChanged: {e:?}"));
    }

    #[allow(clippy::unnecessary_wraps)]
    fn overlays_changed(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        let mut meta = Vec::with_capacity(self.overlays.len());
        for (id, data) in &self.overlays {
            if matches!(data.config.category, OverlayCategory::Internal) {
                continue;
            }
            let icon = if let Some(BackendAttribValue::Icon(icon)) =
                data.config.backend.get_attrib(BackendAttrib::Icon)
            {
                Some(icon)
            } else {
                None
            };

            meta.push(OverlayMeta {
                id,
                name: data.config.name.clone(),
                category: data.config.category,
                visible: data.config.is_active(),
                icon,
            });
        }

        let meta: Rc<[OverlayMeta]> = meta.into();
        for id in [self.watch_id, self.keyboard_id] {
            let _ = self.mut_by_id(id).context("Missing overlay").and_then(|o| {
                o.config
                    .backend
                    .notify(app, OverlayEventData::OverlaysChanged(meta.clone()))
            });
        }

        Ok(())
    }

    #[allow(clippy::unnecessary_wraps)]
    fn visible_overlays_changed(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        let mut vis = Vec::with_capacity(self.overlays.len());

        for (id, data) in &self.overlays {
            if data.config.active_state.is_none()
                || matches!(data.config.category, OverlayCategory::Internal)
            {
                continue;
            }
            vis.push(id);
        }

        let vis: Rc<[OverlayID]> = vis.into();
        for id in [self.watch_id, self.keyboard_id] {
            let _ = self.mut_by_id(id).context("Missing overlay").and_then(|o| {
                o.config
                    .backend
                    .notify(app, OverlayEventData::VisibleOverlaysChanged(vis.clone()))
            });
        }

        Ok(())
    }

    fn sets_changed(&mut self, app: &mut AppState) {
        let len = self.sets.len();
        for id in [self.watch_id, self.keyboard_id] {
            if let Some(o) = self.mut_by_id(id) {
                let _ = o
                    .config
                    .backend
                    .notify(app, OverlayEventData::NumSetsChanged(len))
                    .log_err("Could not notify NumSetsChanged");
            }
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn devices_changed(&mut self, app: &mut AppState) -> anyhow::Result<()> {
        if let Some(watch) = self.mut_by_id(self.watch_id) {
            let _ = watch
                .config
                .backend
                .notify(app, OverlayEventData::DevicesChanged);
        }

        Ok(())
    }

    fn handle_screen_focus_toggle(
        &mut self,
        app: &mut AppState,
        screen_focus: crate::backend::task::ScreenFocusTask,
    ) -> anyhow::Result<()> {
        let screen_name: Arc<str> = screen_focus.screen_name.into();
        let mut carry_saved_state: Option<OverlayWindowState> = None;
        let mut carry_saved_crop_rect: Option<[f32; 4]> = None;
        let mut carry_current_state: Option<OverlayWindowState> = None;

        if let Some(focused_screen) = self.focused_screen.take() {
            let focused_name = focused_screen.name;
            let focused_oid = focused_screen.oid;
            let saved_state = focused_screen.saved_state;
            let saved_crop_rect = focused_screen.saved_crop_rect;
            let current_state = self
                .mut_by_id(focused_oid)
                .and_then(|overlay| overlay.config.active_state.clone())
                .unwrap_or_else(|| saved_state.clone());

            if screen_focus.refresh_only && focused_name == screen_name {
                carry_saved_state = Some(saved_state);
                carry_saved_crop_rect = Some(saved_crop_rect);
                carry_current_state = Some(current_state);
            } else {
                if let Some(overlay) = self.mut_by_id(focused_oid) {
                    overlay
                        .config
                        .backend
                        .set_attrib(app, BackendAttribValue::CropRect(saved_crop_rect));
                    overlay.config.active_state = Some(saved_state.clone());
                    overlay.config.dirty = true;
                }

                self.focus_animation = Some(FocusAnimation {
                    oid: focused_oid,
                    from: current_state,
                    to: saved_state.clone(),
                    start_us: get_micros(),
                    duration_us: 260_000,
                    easing: AnimationEasing::OutCubic,
                });

                if focused_name == screen_name {
                    log::info!("Screen focus: restored previous state for {}", screen_name);
                    return Ok(());
                }
            }
        }

        if screen_focus.refresh_only && carry_saved_state.is_none() {
            return Ok(());
        }

        let Some(target_oid) = self.lookup(&screen_name) else {
            log::warn!("Screen focus: no overlay found for screen {}", screen_name);
            return Ok(());
        };

        let Some(overlay) = self.mut_by_id(target_oid) else {
            log::warn!("Screen focus: overlay {:?} not found", target_oid);
            return Ok(());
        };

        if !matches!(overlay.config.category, OverlayCategory::Screen) {
            log::warn!("Screen focus: overlay {} is not a screen", screen_name);
            return Ok(());
        }

        let saved_state = carry_saved_state.unwrap_or_else(|| {
            overlay
                .config
                .active_state
                .clone()
                .unwrap_or_else(OverlayWindowState::default)
        });
        let saved_crop_rect = carry_saved_crop_rect.unwrap_or_else(|| {
            overlay
                .config
                .backend
                .get_attrib(BackendAttrib::CropRect)
                .and_then(|value| match value {
                    BackendAttribValue::CropRect(crop_rect) => Some(crop_rect),
                    _ => None,
                })
                .unwrap_or([0.0, 0.0, 1.0, 1.0])
        });

        let frame_meta = overlay.frame_meta();
        let aspect_ratio = frame_meta
            .map(|meta| meta.extent[0] as f32 / meta.extent[1] as f32)
            .unwrap_or(1.0)
            .max(0.01);

        let current_state = carry_current_state.unwrap_or_else(|| {
            overlay
                .config
                .active_state
                .clone()
                .unwrap_or_else(|| saved_state.clone())
        });

        let crop_rect = screen_focus.crop_rect.unwrap_or([0.0, 0.0, 1.0, 1.0]);
        overlay
            .config
            .backend
            .set_attrib(app, BackendAttribValue::CropRect(crop_rect));

        let focus_anchor = snap_upright(app.input_state.hmd, Vec3A::Y);
        let focused_state = build_focused_screen_state(
            &saved_state,
            app,
            focus_anchor,
            aspect_ratio,
            screen_focus.target_x,
            screen_focus.target_y,
            crop_rect,
        );

        overlay.config.active_state = Some(focused_state.clone());
        overlay.config.dirty = true;

        self.focused_screen = Some(FocusedScreenState {
            name: screen_name.clone(),
            oid: target_oid,
            saved_state,
            saved_crop_rect,
            focus_anchor,
            target_x: screen_focus.target_x,
            target_y: screen_focus.target_y,
            crop_rect,
        });
        self.focus_animation = Some(FocusAnimation {
            oid: target_oid,
            from: current_state,
            to: focused_state,
            start_us: get_micros(),
            duration_us: 320_000,
            easing: AnimationEasing::OutBack,
        });

        log::info!(
            "Screen focus: focused screen {} on overlay {:?}",
            screen_name,
            target_oid
        );
        Ok(())
    }
}

fn interpolate_overlay_state(
    from: &OverlayWindowState,
    to: &OverlayWindowState,
    t: f32,
) -> OverlayWindowState {
    let (from_scale, from_rot, from_trans) = from.transform.to_scale_rotation_translation();
    let (to_scale, to_rot, to_trans) = to.transform.to_scale_rotation_translation();

    let mut state = to.clone();
    state.transform = Affine3A::from_scale_rotation_translation(
        from_scale.lerp(to_scale, t),
        from_rot.slerp(to_rot, t),
        from_trans.lerp(to_trans, t),
    );
    state.alpha = from.alpha + (to.alpha - from.alpha) * t;
    state
}

fn build_focused_screen_state(
    saved_state: &OverlayWindowState,
    app: &AppState,
    focus_anchor: Affine3A,
    aspect_ratio: f32,
    target_x: f32,
    target_y: f32,
    crop_rect: [f32; 4],
) -> OverlayWindowState {
    use wlx_common::windowing::Positioning;

    let focus_scale = saved_state.transform.matrix3.y_axis.length().max(0.01)
        * app.session.config.focused_screen_scale.max(0.01);
    let mut focus_transform = focus_anchor
        * Affine3A::from_scale_rotation_translation(
            Vec3::splat(focus_scale),
            Quat::IDENTITY,
            Vec3::new(
                0.0,
                0.0,
                -app.session.config.focused_screen_distance.max(0.05),
            ),
        );

    let (_, focus_rotation, _) = focus_transform.to_scale_rotation_translation();
    let width = focus_scale;
    let height = focus_scale / aspect_ratio.max(0.01);
    let offset_strength = if crop_rect != [0.0, 0.0, 1.0, 1.0] {
        0.0
    } else {
        0.35
    };
    let local_x = (0.5 - target_x) * width * offset_strength;
    let local_y = (target_y - 0.5) * height * offset_strength;
    let offset_world = Vec3A::from(focus_rotation.mul_vec3(Vec3::new(local_x, local_y, 0.0)));
    focus_transform.translation += offset_world;
    let mut focused_state = saved_state.clone();
    let curve_x = resolve_focused_screen_curvature(
        saved_state.curvature,
        app.session.config.focused_screen_curve_x,
    );
    focused_state.transform = focus_transform;
    focused_state.positioning = Positioning::Static;
    focused_state.interactable = true;
    focused_state.grabbable = true;
    focused_state.curvature = Some(curve_x);
    focused_state
}

fn resolve_focused_screen_curvature(saved_curvature: Option<f32>, configured_curve_x: f32) -> f32 {
    saved_curvature
        .unwrap_or(0.15)
        .max(configured_curve_x.max(0.0))
}

fn apply_focus_look_assist(base: &OverlayWindowState, app: &AppState) -> OverlayWindowState {
    let mut state = base.clone();
    let (scale, rotation, translation) = base.transform.to_scale_rotation_translation();

    let hmd_forward = app
        .input_state
        .hmd
        .transform_vector3a(Vec3A::NEG_Z)
        .normalize();
    let local_forward = rotation.inverse().mul_vec3a(hmd_forward);

    let aspect_ratio = (scale.x / scale.y).max(0.01);
    let width = scale.x;
    let height = width / aspect_ratio;

    let (assist_offset, assist_rotation) = resolve_focus_look_assist(
        local_forward,
        width,
        height,
        app.session.config.focused_screen_assist_x.max(0.0),
        app.session.config.focused_screen_assist_y.max(0.0),
        app.session.config.focused_screen_rotate_assist_x.max(0.0),
        app.session.config.focused_screen_rotate_assist_y.max(0.0),
    );
    let assist_world = rotation.mul_vec3(assist_offset);

    state.transform = Affine3A::from_scale_rotation_translation(
        scale,
        rotation * assist_rotation,
        translation + assist_world,
    );
    state
}

fn resolve_focus_look_assist(
    local_forward: Vec3A,
    width: f32,
    height: f32,
    translate_assist_x: f32,
    translate_assist_y: f32,
    rotate_assist_x: f32,
    rotate_assist_y: f32,
) -> (Vec3, Quat) {
    let clamped_x = (-local_forward.x).clamp(-0.55, 0.55);
    let clamped_y = (-local_forward.y).clamp(-0.5, 0.5);

    let assist_offset = Vec3::new(
        clamped_x * width * translate_assist_x,
        clamped_y * height * translate_assist_y,
        0.0,
    );

    let assist_rotation = Quat::from_rotation_y(clamped_x * rotate_assist_x)
        * Quat::from_rotation_x(-clamped_y * rotate_assist_y);

    (assist_offset, assist_rotation)
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3, Vec3A};

    use super::{resolve_focus_look_assist, resolve_focused_screen_curvature};

    #[test]
    fn focused_screen_curvature_uses_saved_or_configured_curve() {
        let curve_x = resolve_focused_screen_curvature(Some(0.2), 0.3);
        assert!((curve_x - 0.3).abs() < f32::EPSILON);

        let curve_x = resolve_focused_screen_curvature(None, 0.32);
        assert!((curve_x - 0.32).abs() < f32::EPSILON);
    }

    #[test]
    fn focus_look_assist_is_neutral_when_forward_is_centered() {
        let (offset, rotation) =
            resolve_focus_look_assist(Vec3A::new(0.0, 0.0, -1.0), 1.2, 0.8, 0.13, 0.18, 0.13, 0.12);

        assert!(offset.length() < f32::EPSILON);
        assert!(rotation.abs_diff_eq(Quat::IDENTITY, f32::EPSILON));
    }

    #[test]
    fn focus_look_assist_adds_yaw_pitch_without_roll() {
        let (offset, rotation) = resolve_focus_look_assist(
            Vec3A::new(-0.4, 0.3, -1.0),
            1.0,
            0.75,
            0.13,
            0.18,
            0.2,
            0.15,
        );

        assert!(offset.x > 0.0);
        assert!(offset.y < 0.0);

        let rotated_right = rotation.mul_vec3(Vec3::X);
        assert!(rotated_right.y.abs() < 1e-5);

        let rotated_up = rotation.mul_vec3(Vec3::Y);
        assert!(rotated_up.z > 0.0);
    }
}
