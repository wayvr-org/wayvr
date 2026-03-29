use std::f32::consts::PI;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Affine3A, Vec2, Vec3A, Vec3Swizzles};

use idmap_derive::IntegerId;
use smallvec::{SmallVec, smallvec};
use strum::AsRefStr;
use wlx_common::common::LeftRight;
use wlx_common::windowing::{OverlayWindowState, Positioning};

use crate::backend::task::{InputTask, OverlayTask};
use crate::overlays::anchor::{ANCHOR_NAME, GRAB_HELP_NAME};
use crate::overlays::keyboard::KEYBOARD_NAME;
use crate::overlays::watch::WATCH_NAME;
use crate::state::{AppSession, AppState};
use crate::subsystem::hid::WheelDelta;
use crate::subsystem::input::KeyboardFocus;
use crate::windowing::backend::OverlayEventData;
use crate::windowing::manager::OverlayWindowManager;
use crate::windowing::window::{self, OverlayWindowData, realign};
use crate::windowing::{OverlayID, OverlaySelector};

use super::task::TaskType;

#[derive(Clone, Default)]
pub struct HoverResult {
    pub haptics: Option<Haptics>,
    /// If true, the laser shows at this position and no further raycasting will be done.
    pub consume: bool,
}

pub struct TrackedDevice {
    pub soc: Option<f32>,
    pub charging: bool,
    pub role: TrackedDeviceRole,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntegerId, AsRefStr)]
pub enum TrackedDeviceRole {
    None,
    Hmd,
    LeftHand,
    RightHand,
    Tracker,
}

pub struct InputState {
    pub hmd: Affine3A,
    pub ipd: f32,
    pub pointers: [Pointer; 2],
    pub devices: Vec<TrackedDevice>,
    processes: Vec<Child>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            hmd: Affine3A::IDENTITY,
            ipd: 0.0,
            pointers: [Pointer::new(0), Pointer::new(1)],
            devices: Vec::new(),
            processes: Vec::new(),
        }
    }

    pub fn handle_task(&mut self, task: InputTask) {
        match task {
            InputTask::Haptics { device, haptics } => {
                if let Some(pointer) = self.pointers.get_mut(device) {
                    pointer.pending_haptics = Some(haptics);
                } else {
                    log::warn!("Can't trigger haptics on non-existing device: {device}");
                }
            }
        }
    }

    pub const fn pre_update(&mut self) {
        self.pointers[0].before = self.pointers[0].now;
        self.pointers[1].before = self.pointers[1].now;
    }

    pub fn post_update(&mut self, session: &AppSession) {
        for hand in &mut self.pointers {
            #[cfg(debug_assertions)]
            debug_print_hand(hand);

            if hand.now.click {
                hand.last_click = Instant::now();

                if !hand.before.click {
                    hand.click_press_start = Some(Instant::now());
                    hand.interaction.long_press_click_sent = false;
                }
            } else {
                hand.click_press_start = None;
                hand.interaction.long_press_click_sent = false;
            }

            let long_press_threshold = Duration::from_secs_f32(session.config.long_press_duration);
            let is_long_press = hand
                .click_press_start
                .is_some_and(|start| start.elapsed() >= long_press_threshold);

            // Prevent the mode from changing during a click (except for long press)
            if !hand.before.click || is_long_press {
                if hand.now.click_modifier_right {
                    hand.interaction.mode = PointerMode::Right;
                } else if is_long_press {
                    hand.interaction.mode = PointerMode::Right;
                } else if hand.now.click_modifier_middle {
                    hand.interaction.mode = PointerMode::Middle;
                } else if !hand.before.click {
                    let hmd_up = self.hmd.transform_vector3a(Vec3A::Y);
                    let dot = hmd_up.dot(hand.pose.transform_vector3a(Vec3A::X))
                        * 2.0f32.mul_add(-(hand.idx as f32), 1.0);

                    hand.interaction.mode = if dot < -0.85 {
                        PointerMode::Right
                    } else if dot > 0.7 {
                        PointerMode::Middle
                    } else {
                        PointerMode::Left
                    };

                    let middle_click_orientation = false;
                    let right_click_orientation = false;
                    match hand.interaction.mode {
                        PointerMode::Middle => {
                            if !middle_click_orientation {
                                hand.interaction.mode = PointerMode::Left;
                            }
                        }
                        PointerMode::Right => {
                            if !right_click_orientation {
                                hand.interaction.mode = PointerMode::Left;
                            }
                        }
                        _ => {}
                    }
                }
            }

            if hand.now.alt_click != hand.before.alt_click {
                // Reap previous processes
                self.processes
                    .retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));

                let mut args = if hand.now.alt_click {
                    session.config.alt_click_down.iter()
                } else {
                    session.config.alt_click_up.iter()
                };

                if let Some(program) = args.next()
                    && let Ok(child) = Command::new(program).args(args).spawn()
                {
                    self.processes.push(child);
                }
            }
        }
    }
}

#[cfg(debug_assertions)]
fn debug_print_hand(hand: &Pointer) {
    {
        if hand.now.click != hand.before.click {
            log::debug!("Hand {}: click {}", hand.idx, hand.now.click);
        }
        if hand.now.grab != hand.before.grab {
            log::debug!("Hand {}: grab {}", hand.idx, hand.now.grab);
        }
        if hand.now.alt_click != hand.before.alt_click {
            log::debug!("Hand {}: alt_click {}", hand.idx, hand.now.alt_click);
        }
        if hand.now.show_hide != hand.before.show_hide {
            log::debug!("Hand {}: show_hide {}", hand.idx, hand.now.show_hide);
        }
        if hand.now.toggle_dashboard != hand.before.toggle_dashboard {
            log::debug!(
                "Hand {}: toggle_dashboard {}",
                hand.idx,
                hand.now.toggle_dashboard
            );
        }
        if hand.now.space_drag != hand.before.space_drag {
            log::debug!("Hand {}: space_drag {}", hand.idx, hand.now.space_drag);
        }
        if hand.now.space_rotate != hand.before.space_rotate {
            log::debug!("Hand {}: space_rotate {}", hand.idx, hand.now.space_rotate);
        }
        if hand.now.space_reset != hand.before.space_reset {
            log::debug!("Hand {}: space_reset {}", hand.idx, hand.now.space_reset);
        }
        if hand.now.click_modifier_right != hand.before.click_modifier_right {
            log::debug!(
                "Hand {}: click_modifier_right {}",
                hand.idx,
                hand.now.click_modifier_right
            );
        }
        if hand.now.click_modifier_middle != hand.before.click_modifier_middle {
            log::debug!(
                "Hand {}: click_modifier_middle {}",
                hand.idx,
                hand.now.click_modifier_middle
            );
        }
    }
}

pub struct InteractionState {
    pub mode: PointerMode,
    pub grabbed: Option<GrabData>,
    pub clicked_id: Option<OverlayID>,
    pub hovered_id: Option<OverlayID>,
    pub next_push: Instant,
    pub haptics: Option<f32>,
    pub should_block_input: bool,
    pub should_block_poses: bool,
    pub click_sent: bool,
    pub click_mode: PointerMode,
    pub long_press_click_sent: bool,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            mode: PointerMode::Left,
            grabbed: None,
            clicked_id: None,
            hovered_id: None,
            next_push: Instant::now(),
            haptics: None,
            should_block_input: false,
            should_block_poses: false,
            click_sent: false,
            click_mode: PointerMode::Left,
            long_press_click_sent: false,
        }
    }
}

pub struct Pointer {
    pub idx: usize,
    pub pose: Affine3A,
    pub raw_pose: Affine3A,
    pub now: PointerState,
    pub before: PointerState,
    pub last_click: Instant,
    pub click_press_start: Option<Instant>,
    pub pending_haptics: Option<Haptics>,
    pub(super) interaction: InteractionState,
    pub tracked: bool,
    pub handsfree: bool,
}

impl Pointer {
    pub fn new(idx: usize) -> Self {
        debug_assert!(idx == 0 || idx == 1);
        Self {
            idx,
            pose: Affine3A::IDENTITY,
            raw_pose: Affine3A::IDENTITY,
            now: PointerState::default(),
            before: PointerState::default(),
            last_click: Instant::now(),
            click_press_start: None,
            pending_haptics: None,
            interaction: InteractionState::default(),
            tracked: false,
            handsfree: false,
        }
    }

    pub const fn hand(&self) -> Option<LeftRight> {
        match self.idx {
            0 => Some(LeftRight::Left),
            1 => Some(LeftRight::Right),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct PointerState {
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub click: bool,
    pub grab: bool,
    pub alt_click: bool,
    pub show_hide: bool,
    pub toggle_dashboard: bool,
    pub space_drag: bool,
    pub space_rotate: bool,
    pub space_reset: bool,
    pub click_modifier_right: bool,
    pub click_modifier_middle: bool,
    pub move_mouse: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PointerHit {
    pub pointer: usize,
    pub overlay: OverlayID,
    pub mode: PointerMode,
    pub primary: bool,
    pub uv: Vec2,
    pub dist: f32,
}

#[derive(Clone, Copy)]
pub struct Haptics {
    pub intensity: f32,
    pub duration: f32,
    pub frequency: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct RayHit {
    overlay: OverlayID,
    global_pos: Vec3A,
    local_pos: Vec2,
    dist: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GrabData {
    pub offset: Affine3A,
    pub grabbed_id: OverlayID,
    pub grab_anchor: bool,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum PointerMode {
    #[default]
    Left,
    Right,
    Middle,
    Special,
}

pub struct PointerLine {
    pub mode: PointerMode,
    pub a: Vec3A,
    pub b: Vec3A,
}

fn populate_lines(
    lines: &mut Vec<PointerLine>,
    pointer: &Pointer,
    maybe_hit: Option<(PointerHit, RayHit)>,
    hmd: &Affine3A,
) {
    let Some((hit, raw_hit)) = maybe_hit else {
        return;
    };

    if pointer.handsfree {
        const HALF_SIZE: f32 = 0.01;

        // horizontal arm
        lines.push(PointerLine {
            mode: PointerMode::Left,
            a: raw_hit.global_pos - (hmd.x_axis * HALF_SIZE),
            b: raw_hit.global_pos + (hmd.x_axis * HALF_SIZE),
        });

        // vertical arm
        lines.push(PointerLine {
            mode: PointerMode::Special,
            a: raw_hit.global_pos - (hmd.y_axis * HALF_SIZE),
            b: raw_hit.global_pos + (hmd.y_axis * HALF_SIZE),
        });
    } else {
        lines.push(PointerLine {
            mode: hit.mode,
            a: pointer.pose.translation,
            b: raw_hit.global_pos,
        });
    }
}

fn update_focus(focus: &mut KeyboardFocus, overlay_keyboard_focus: Option<KeyboardFocus>) {
    if let Some(f) = &overlay_keyboard_focus
        && *focus != *f
    {
        log::debug!("Setting keyboard focus to {:?}", *f);
        *focus = *f;
    }
}

pub fn interact<O>(
    overlays: &mut OverlayWindowManager<O>,
    app: &mut AppState,
    lines: &mut Vec<PointerLine>,
) -> [Option<Haptics>; 2]
where
    O: Default,
{
    let hits = if app.input_state.pointers[1].last_click > app.input_state.pointers[0].last_click {
        let right = interact_hand(1, overlays, app);
        let left = interact_hand(0, overlays, app);
        [left, right]
    } else {
        let left = interact_hand(0, overlays, app);
        let right = interact_hand(1, overlays, app);
        [left, right]
    };

    for (idx, hit) in hits.iter().enumerate() {
        populate_lines(
            lines,
            &app.input_state.pointers[idx],
            hit.0,
            &app.input_state.hmd,
        );
    }

    [hits[0].1, hits[1].1]
}

fn interact_hand<O>(
    idx: usize,
    overlays: &mut OverlayWindowManager<O>,
    app: &mut AppState,
) -> (Option<(PointerHit, RayHit)>, Option<Haptics>)
where
    O: Default,
{
    let mut pointer = &mut app.input_state.pointers[idx];
    let pending_haptics = pointer.pending_haptics.take();

    if !pointer.tracked {
        return (None, pending_haptics); // no hit
    }

    // already grabbing, ignore everything else
    if let Some(grab_data) = pointer.interaction.grabbed {
        if let Some(grabbed) = overlays.mut_by_id(grab_data.grabbed_id) {
            handle_grabbed(idx, grabbed, app);
        } else {
            log::warn!("Grabbed overlay {:?} does not exist", grab_data.grabbed_id);
            pointer.interaction.grabbed = None;
        }
        return (None, pending_haptics);
    }

    let hovered_id = pointer.interaction.hovered_id.take();
    let (Some((mut hit, raw_hit)), haptics) = get_nearest_hit(idx, overlays, app) else {
        handle_no_hit(idx, hovered_id, overlays, app);
        return (None, pending_haptics); // no hit
    };

    // focus change
    if let Some(hovered_id) = hovered_id
        && hovered_id != hit.overlay
        && let Some(old_hovered) = overlays.mut_by_id(hovered_id)
    {
        if old_hovered.primary_pointer.is_some_and(|i| i == idx) {
            old_hovered.primary_pointer = None;
        }
        log::debug!("{} on_left (focus changed)", old_hovered.config.name);
        old_hovered.config.backend.on_left(app, idx);
        old_hovered.hover_pointers[idx] = false;
        if !old_hovered.hover_pointers.iter().any(|x| *x) {
            overlays.edit_overlay(hovered_id, false, app);
        }
    }

    overlays.edit_overlay(hit.overlay, true, app);
    let edit_mode = overlays.get_edit_mode();

    let Some(hovered) = overlays.mut_by_id(hit.overlay) else {
        log::warn!("Hit overlay {:?} does not exist", hit.overlay);
        return (None, pending_haptics); // no hit
    };
    pointer = &mut app.input_state.pointers[idx];
    pointer.interaction.hovered_id = Some(hit.overlay);
    hovered.hover_pointers[idx] = true;

    if let Some(primary_pointer) = hovered.primary_pointer {
        if hit.pointer <= primary_pointer {
            hovered.primary_pointer = Some(hit.pointer);
            hit.primary = true;
        }
    } else {
        hovered.primary_pointer = Some(hit.pointer);
        hit.primary = true;
    }

    if let Some(state) = hovered.config.active_state.as_ref() {
        pointer.interaction.should_block_input = state.block_input
            && (hovered.config.name.as_ref() != WATCH_NAME
                || !app.session.config.block_game_input_ignore_watch);

        pointer.interaction.should_block_poses = state.block_input
            && app.session.config.block_poses_on_kbd_interaction
            && hovered.config.name.as_ref() == KEYBOARD_NAME;
    } else {
        pointer.interaction.should_block_input = false;
        pointer.interaction.should_block_poses = false;
    }

    #[cfg(debug_assertions)]
    log::trace!("Hit: {} {:?}", hovered.config.name, hit);

    let hovered_state = hovered.config.active_state.as_mut().unwrap();

    // grab
    if pointer.now.grab && !pointer.before.grab && hovered_state.grabbable {
        update_focus(
            &mut app.hid_provider.keyboard_focus,
            hovered.config.keyboard_focus,
        );
        start_grab(
            idx,
            hit.overlay,
            hovered.config.name.clone(),
            hovered.config.editing,
            hovered_state,
            app,
            edit_mode,
        );
        log::debug!("Hand {}: grabbed {}", hit.pointer, hovered.config.name);
        return (
            Some((hit, raw_hit)),
            Some(Haptics {
                intensity: 0.25,
                duration: 0.1,
                frequency: 0.1,
            }),
        );
    }

    handle_scroll(&hit, hovered, app);

    // click / release - normal behavior
    let pointer = &mut app.input_state.pointers[hit.pointer];

    // Check if we should send a long-press click immediately
    let long_press_threshold = Duration::from_secs_f32(app.session.config.long_press_duration);
    let is_long_press = pointer
        .click_press_start
        .is_some_and(|start| start.elapsed() >= long_press_threshold);

    if is_long_press && pointer.now.click && !pointer.interaction.long_press_click_sent {
        // Immediately send right-click (press + release)
        pointer.interaction.long_press_click_sent = true;
        let mut hit_right = hit;
        hit_right.mode = PointerMode::Right;
        hovered.config.backend.on_pointer(app, &hit_right, true);
        hovered.config.backend.on_pointer(app, &hit_right, false);
    } else if pointer.now.click && !pointer.before.click {
        pointer.interaction.clicked_id = Some(hit.overlay);
        update_focus(
            &mut app.hid_provider.keyboard_focus,
            hovered.config.keyboard_focus,
        );
        hovered.config.backend.on_pointer(app, &hit, true);
    } else if !pointer.now.click && pointer.before.click {
        // Only send release if we haven't already sent a long press click
        if !pointer.interaction.long_press_click_sent {
            if let Some(clicked_id) = pointer.interaction.clicked_id.take() {
                if let Some(clicked) = overlays.mut_by_id(clicked_id) {
                    clicked.config.backend.on_pointer(app, &hit, false);
                }
            } else {
                hovered.config.backend.on_pointer(app, &hit, false);
            }
        }
    }

    (Some((hit, raw_hit)), haptics.or(pending_haptics))
}

fn handle_no_hit<O>(
    pointer_idx: usize,
    hovered_id: Option<OverlayID>,
    overlays: &mut OverlayWindowManager<O>,
    app: &mut AppState,
) {
    if let Some(hovered_id) = hovered_id
        && let Some(hovered) = overlays.mut_by_id(hovered_id)
    {
        log::debug!("{} on_left (no hit)", hovered.config.name);
        hovered.config.backend.on_left(app, pointer_idx);
        hovered.hover_pointers[pointer_idx] = false;
        if !hovered.hover_pointers.iter().any(|x| *x) {
            overlays.edit_overlay(hovered_id, false, app);
        }
    }

    let pointer = &mut app.input_state.pointers[pointer_idx];
    pointer.interaction.should_block_input = false;
    pointer.interaction.should_block_poses = false;

    // in case click released while not aiming at anything
    // send release event to overlay that was originally clicked
    if !pointer.now.click
        && pointer.before.click
        && !pointer.interaction.long_press_click_sent
        && let Some(clicked_id) = pointer.interaction.clicked_id.take()
    {
        let hit = PointerHit {
            pointer: pointer.idx,
            overlay: clicked_id,
            mode: pointer.interaction.mode,
            ..Default::default()
        };
        if let Some(clicked) = overlays.mut_by_id(clicked_id) {
            clicked.config.backend.on_pointer(app, &hit, false);
        }
    }
}

fn handle_scroll<O>(hit: &PointerHit, hovered: &mut OverlayWindowData<O>, app: &mut AppState) {
    let pointer = &mut app.input_state.pointers[hit.pointer];
    if pointer.now.scroll_x.abs() <= 0.1 && pointer.now.scroll_y.abs() <= 0.1 {
        return;
    }

    let config = &app.session.config;

    let scroll_x = pointer.now.scroll_x
        * config.scroll_speed
        * if config.invert_scroll_direction_x {
            -1.0
        } else {
            1.0
        };
    let scroll_y = pointer.now.scroll_y
        * config.scroll_speed
        * if config.invert_scroll_direction_x {
            -1.0
        } else {
            1.0
        };

    if app.input_state.pointers[1 - hit.pointer]
        .interaction
        .grabbed
        .is_some_and(|x| x.grabbed_id == hit.overlay)
    {
        let can_curve = hovered
            .frame_meta()
            .is_some_and(|e| e.extent[0] >= e.extent[1]);

        // re-borrow
        let hovered_state = hovered.config.active_state.as_mut().unwrap();
        if can_curve {
            let cur = hovered_state.curvature.unwrap_or(0.0);
            let new = scroll_y.mul_add(-0.01, cur).min(0.5);
            if new <= f32::EPSILON {
                hovered_state.curvature = None;
            } else {
                hovered_state.curvature = Some(new);
            }
        } else {
            hovered_state.curvature = None;
        }
    } else {
        hovered.config.backend.on_scroll(
            app,
            hit,
            WheelDelta {
                x: scroll_x,
                y: scroll_y,
            },
        );
    }
}

fn get_nearest_hit<O>(
    pointer_idx: usize,
    overlays: &mut OverlayWindowManager<O>,
    app: &mut AppState,
) -> (Option<(PointerHit, RayHit)>, Option<Haptics>)
where
    O: Default,
{
    let pointer = &mut app.input_state.pointers[pointer_idx];
    let ray_origin = pointer.pose;
    let mode = pointer.interaction.mode;
    let edit_mode = overlays.get_edit_mode();

    let mut hits: SmallVec<[RayHit; 8]> = smallvec!();

    for (id, overlay) in overlays.iter() {
        let Some(overlay_state) = overlay.config.active_state.as_ref() else {
            continue;
        };
        if !overlay_state.interactable && !edit_mode {
            continue;
        }

        if let Some(hit) = ray_test(
            &ray_origin,
            id,
            &overlay_state.transform,
            overlay_state.curvature.as_ref(),
        ) && hit.dist.is_finite()
        {
            hits.push(hit);
        }
    }

    hits.sort_by(|a, b| a.dist.total_cmp(&b.dist));

    for hit in hits {
        let overlay = overlays.mut_by_id(hit.overlay).unwrap(); // safe because we just got the id from the overlay

        let Some(uv) = overlay
            .config
            .backend
            .as_mut()
            .get_interaction_transform()
            .map(|a| a.transform_point2(hit.local_pos))
        else {
            continue;
        };

        if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
            continue;
        }

        let pointer_hit = PointerHit {
            pointer: pointer_idx,
            overlay: hit.overlay,
            mode,
            primary: false,
            uv,
            dist: hit.dist,
        };

        let result = overlay.config.backend.on_hover(app, &pointer_hit);
        if result.consume || overlay.config.editing {
            return (Some((pointer_hit, hit)), result.haptics);
        }
    }

    (None, None)
}

fn start_grab(
    idx: usize,
    id: OverlayID,
    name: Arc<str>,
    editing: bool,
    state: &mut OverlayWindowState,
    app: &mut AppState,
    edit_mode: bool,
) {
    let pointer = &mut app.input_state.pointers[idx];

    // Grab anchor if:
    // - grabbed overlay is Anchored
    // - not in editmode
    // - grabbing with one hand. (grabbing with the 2nd hand will grab the individual overlay instead)
    let grab_anchor =
        !edit_mode && !app.anchor_grabbed && matches!(state.positioning, Positioning::Anchored);

    let relative_grab_transform = if grab_anchor {
        app.anchor
    } else {
        state.transform
    };

    let offset = pointer.pose.inverse() * relative_grab_transform;

    app.anchor_grabbed = grab_anchor;

    pointer.interaction.grabbed = Some(GrabData {
        offset,
        grabbed_id: id,
        grab_anchor,
    });

    // Show anchor
    app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
        OverlaySelector::Name(ANCHOR_NAME.clone()),
        Box::new(|app, o| {
            o.activate(app);
        }),
    )));

    if let Some(hand) = pointer.hand()
        && !app.session.config.hide_grab_help
        && !pointer.handsfree
    {
        let pos = state.positioning;
        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
            OverlaySelector::Name(GRAB_HELP_NAME.clone()),
            Box::new(move |app, o| {
                let _ = o
                    .backend
                    .notify(app, OverlayEventData::OverlayGrabbed { name, pos, editing })
                    .inspect_err(|e| log::warn!("Error during Notify OverlayGrabbed: {e:?}"));

                o.default_state.positioning = Positioning::FollowHand {
                    hand,
                    lerp: 0.1,
                    align_to_hmd: true,
                };
                o.activate(app);
            }),
        )));
    }
}

fn handle_scale(transform: &mut Affine3A, scroll_y: f32) {
    let cur_scale = transform.x_axis.length();
    if cur_scale < 0.1 && scroll_y > 0.0 {
        return;
    }
    if cur_scale > 20. && scroll_y < 0.0 {
        return;
    }

    transform.matrix3 = transform
        .matrix3
        .mul_scalar(0.025f32.mul_add(-scroll_y, 1.0));
}

#[allow(clippy::too_many_lines)]
fn handle_grabbed<O>(idx: usize, overlay: &mut OverlayWindowData<O>, app: &mut AppState)
where
    O: Default,
{
    let pointer = &mut app.input_state.pointers[idx];
    let Some(grab_data) = pointer.interaction.grabbed.as_mut() else {
        log::error!("Grabbed overlay does not exist");
        return;
    };
    let grab_anchor = grab_data.grab_anchor;

    if pointer.now.grab {
        let Some(overlay_state) = overlay.config.active_state.as_mut() else {
            // overlay got toggled off while being grabbed. those dastardly users!
            // just wait for them to release the grab
            return;
        };

        if grab_anchor {
            if pointer.now.click {
                pointer.interaction.mode = PointerMode::Special;
                let grab_dist = grab_data.offset.translation.length().clamp(0.5, 5.0) * 0.2 + 0.4;
                handle_scale(&mut app.anchor, pointer.now.scroll_y * grab_dist);
                handle_scale(&mut grab_data.offset, pointer.now.scroll_y * grab_dist);
            } else if app.session.config.allow_sliding && pointer.now.scroll_y.is_finite() {
                // single grab push/pull
                let grab_dist = grab_data.offset.translation.length().clamp(0.5, 5.0);
                grab_data.offset.translation.z -= pointer.now.scroll_y * 0.02 * grab_dist;
                grab_data.offset.translation.z = grab_data.offset.translation.z.min(-0.05);
            }
            if pointer.now.click_modifier_right {
                app.anchor = pointer.pose * grab_data.offset;
            } else {
                app.anchor.translation =
                    pointer.pose.transform_point3a(grab_data.offset.translation);
                realign(&mut app.anchor, &app.input_state.hmd);
            }
        } else {
            // single grab resize
            if pointer.now.click {
                pointer.interaction.mode = PointerMode::Special;
                let grab_dist = grab_data.offset.translation.length().clamp(0.5, 5.0) * 0.2 + 0.4;
                handle_scale(
                    &mut overlay_state.transform,
                    pointer.now.scroll_y * grab_dist,
                );
                handle_scale(&mut grab_data.offset, pointer.now.scroll_y * grab_dist);
            } else if app.session.config.allow_sliding && pointer.now.scroll_y.is_finite() {
                // single grab push/pull
                let grab_dist = grab_data.offset.translation.length().clamp(0.5, 5.0);
                grab_data.offset.translation.z -= pointer.now.scroll_y * 0.02 * grab_dist;
                grab_data.offset.translation.z = grab_data.offset.translation.z.min(-0.05);
            }
            if pointer.now.click_modifier_right {
                overlay_state.transform = pointer.pose * grab_data.offset;
            } else {
                overlay_state.transform.translation =
                    pointer.pose.transform_point3a(grab_data.offset.translation);
                realign(&mut overlay_state.transform, &app.input_state.hmd);
            }
            overlay.config.pause_movement = true;
            overlay.config.dirty = true;
        }
    } else {
        // not now.grab
        pointer.interaction.grabbed = None;
        if grab_anchor {
            app.anchor_grabbed = false;
        } else {
            // single grab released
            if &*overlay.config.name == WATCH_NAME {
                // watch special: when dropped, follow the hand that wasn't grabbing
                if let Some(overlay_state) = overlay.config.active_state.as_mut() {
                    overlay_state.positioning = match overlay_state.positioning {
                        Positioning::FollowHand {
                            hand,
                            lerp,
                            align_to_hmd,
                        } => match pointer.hand() {
                            Some(LeftRight::Left) => Positioning::FollowHand {
                                hand: LeftRight::Right,
                                lerp,
                                align_to_hmd,
                            },
                            Some(LeftRight::Right) => Positioning::FollowHand {
                                hand: LeftRight::Left,
                                lerp,
                                align_to_hmd,
                            },
                            _ => Positioning::FollowHand {
                                hand,
                                lerp,
                                align_to_hmd,
                            },
                        },
                        x => x,
                    };
                }
            } else if overlay.config.global
                && let Some(active_state) = overlay.config.active_state.as_ref()
            {
                let cur_scale = overlay.config.default_state.transform.x_axis.length();
                let tgt_scale = active_state.transform.x_axis.length();
                let mat = &mut overlay.config.default_state.transform.matrix3;
                *mat = mat.mul_scalar(tgt_scale / cur_scale);
            }
            overlay.config.pause_movement = false;
            if let Some(overlay_state) = overlay.config.active_state.as_mut() {
                window::save_transform(overlay_state, app);
            }
        }

        // Hide anchor
        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
            OverlaySelector::Name(ANCHOR_NAME.clone()),
            Box::new(|_app, o| {
                o.deactivate();
            }),
        )));
        app.tasks.enqueue(TaskType::Overlay(OverlayTask::Modify(
            OverlaySelector::Name(GRAB_HELP_NAME.clone()),
            Box::new(|_app, o| {
                o.deactivate();
            }),
        )));
        log::debug!("Hand {}: dropped {}", idx, overlay.config.name);
    }
}

fn ray_test(
    ray_origin: &Affine3A,
    overlay: OverlayID,
    overlay_pose: &Affine3A,
    curvature: Option<&f32>,
) -> Option<RayHit> {
    let (dist, local_pos) = curvature.map_or_else(
        || {
            Some(raycast_plane(
                ray_origin,
                Vec3A::NEG_Z,
                overlay_pose,
                Vec3A::NEG_Z,
            ))
        },
        |curvature| raycast_cylinder(ray_origin, Vec3A::NEG_Z, overlay_pose, *curvature),
    )?;

    if dist < 0.0 {
        // hit is behind us
        return None;
    }

    Some(RayHit {
        overlay,
        global_pos: ray_origin.transform_point3a(Vec3A::NEG_Z * dist),
        local_pos,
        dist,
    })
}

fn raycast_plane(
    source: &Affine3A,
    source_fwd: Vec3A,
    plane: &Affine3A,
    plane_norm: Vec3A,
) -> (f32, Vec2) {
    let plane_normal = plane.transform_vector3a(plane_norm);
    let ray_dir = source.transform_vector3a(source_fwd);

    let d = plane.translation.dot(-plane_normal);
    let mut dist = -(d + source.translation.dot(plane_normal)) / ray_dir.dot(plane_normal);

    let hit_local = plane
        .inverse()
        .transform_point3a(source.translation + ray_dir * dist)
        .xy();

    // hitting the backside of the plane, make the hit invalid
    if ray_dir.dot(plane_normal) < 0.0 && dist.is_sign_positive() {
        dist = -dist;
    }

    (dist, hit_local)
}

fn raycast_cylinder(
    source: &Affine3A,
    source_fwd: Vec3A,
    plane: &Affine3A,
    curvature: f32,
) -> Option<(f32, Vec2)> {
    // this is solved locally; (0,0) is the center of the cylinder, and the cylinder is aligned with the Y axis
    let size = plane.x_axis.length();
    let to_local = Affine3A {
        matrix3: plane.matrix3.mul_scalar(1.0 / size),
        translation: plane.translation,
    }
    .inverse();

    let radius = size / (2.0 * PI * curvature);

    let ray_dir = to_local.transform_vector3a(source.transform_vector3a(source_fwd));
    let ray_origin = to_local.transform_point3a(source.translation) + Vec3A::NEG_Z * radius;

    let v_dir = ray_dir.xz();
    let v_pos = ray_origin.xz();

    let l_dir = v_dir.dot(v_dir);
    let l_pos = v_dir.dot(v_pos);
    let c = radius.mul_add(-radius, v_pos.dot(v_pos));

    let d = l_pos.mul_add(l_pos, -(l_dir * c));
    if d < f32::EPSILON {
        return None;
    }

    let sqrt_d = d.sqrt();

    let t1 = (-l_pos - sqrt_d) / l_dir;
    let t2 = (-l_pos + sqrt_d) / l_dir;

    let mut t = t1.max(t2);

    if t < f32::EPSILON {
        return None;
    }

    let mut hit_local = ray_origin + ray_dir * t;
    if hit_local.z > 0.0 {
        // hitting the opposite half of the cylinder
        return None;
    }

    let normal = Vec3A::new(hit_local.x, 0.0, hit_local.z).normalize();
    // If hitting from the outside, flip t
    if ray_dir.dot(normal) < 0.0 && t.is_sign_positive() {
        t = -t;
    }

    let max_angle = 2.0 * (size / (2.0 * radius));
    let x_angle = (hit_local.x / radius).asin();

    hit_local.x = x_angle / max_angle;
    hit_local.y /= size;

    Some((t, hit_local.xy()))
}
