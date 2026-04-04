use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::atomic::Ordering,
};

use anyhow::Context;
use glam::{Affine3A, Vec3, Vec3A};
use slotmap::{Key, SecondaryMap, SlotMap};
use wgui::log::LogErr;
use wlx_common::{
    astr_containers::{AStrMap, AStrMapExt},
    config::SerializedWindowSet,
    overlays::{BackendAttrib, BackendAttribValue, ToastTopic},
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
        watch::{WATCH_NAME, create_watch},
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

pub const MAX_OVERLAY_SETS: usize = 6;

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
            watch_id: OverlayID::null(),    // set down below
            keyboard_id: OverlayID::null(), // set down below
            edit_mode: false,
            dropped_overlays: VecDeque::with_capacity(8),
            initialized: false,
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
            let mut ows = ows.clone();

            // fix angle_fade missing on watch if loading older state
            if name.as_ref() == WATCH_NAME {
                ows.angle_fade = true;
            }

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
}
