use glam::{Affine3A, Quat, Vec3, Vec3A, vec3a};
use ovr_overlay::{
    chaperone_setup::ChaperoneSetupManager,
    compositor::CompositorManager,
    sys::{EChaperoneConfigFile, ETrackingUniverseOrigin, HmdMatrix34_t},
};

use crate::{
    backend::{input::InputState, task::PlayspaceTask},
    state::AppState,
    windowing::manager::OverlayWindowManager,
};

use super::{helpers::Affine3AConvert, overlay::OpenVrOverlayData};

struct MoverData<T> {
    pose: Affine3A,
    hand: usize,
    hand_pose: T,
}

pub(super) struct PlayspaceMover {
    universe: ETrackingUniverseOrigin,
    drag: Option<MoverData<Vec3A>>,
    rotate: Option<MoverData<Quat>>,
}

impl PlayspaceMover {
    pub const fn new() -> Self {
        Self {
            universe: ETrackingUniverseOrigin::TrackingUniverseRawAndUncalibrated,
            drag: None,
            rotate: None,
        }
    }

    pub fn handle_task(
        &mut self,
        app: &AppState,
        chaperone_mgr: &mut ChaperoneSetupManager,
        task: PlayspaceTask,
    ) {
        match task {
            PlayspaceTask::FixFloor => {
                self.fix_floor(chaperone_mgr, &app.input_state);
            }
            PlayspaceTask::Reset => {
                self.reset_offset(chaperone_mgr, &app.input_state);
            }
            PlayspaceTask::Recenter => {
                self.recenter(chaperone_mgr, &app.input_state);
            }
        }
    }

    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
    pub fn update(
        &mut self,
        chaperone_mgr: &mut ChaperoneSetupManager,
        overlays: &mut OverlayWindowManager<OpenVrOverlayData>,
        app: &AppState,
    ) {
        let universe = self.universe.clone();

        if let Some(data) = self.rotate.as_mut() {
            let pointer = &app.input_state.pointers[data.hand];
            if !pointer.now.space_rotate {
                self.rotate = None;
                log::info!("End space rotate");
                return;
            }

            let new_hand =
                Quat::from_affine3(&(data.pose * app.input_state.pointers[data.hand].raw_pose));

            let dq = new_hand * data.hand_pose.conjugate();
            let rel_y = f32::atan2(
                2.0 * dq.y.mul_add(dq.w, dq.x * dq.z),
                2.0f32.mul_add(dq.w.mul_add(dq.w, dq.x * dq.x), -1.0),
            );

            let mut space_transform = Affine3A::from_rotation_y(rel_y);
            let offset = (space_transform.transform_vector3a(app.input_state.hmd.translation)
                - app.input_state.hmd.translation)
                * -1.0;

            space_transform.translation = offset;

            data.pose *= space_transform;
            data.hand_pose = new_hand;

            if self.universe == ETrackingUniverseOrigin::TrackingUniverseStanding {
                apply_chaperone_transform(space_transform.inverse(), chaperone_mgr);
            }
            set_working_copy(&universe, chaperone_mgr, &data.pose);
            chaperone_mgr.commit_working_copy(EChaperoneConfigFile::EChaperoneConfigFile_Live);
        } else {
            for (i, pointer) in app.input_state.pointers.iter().enumerate() {
                if pointer.now.space_rotate {
                    let Some(mat) = get_working_copy(&universe, chaperone_mgr) else {
                        log::warn!("Can't space rotate - failed to get zero pose");
                        return;
                    };
                    let hand_pose = Quat::from_affine3(&(mat * pointer.raw_pose));
                    self.rotate = Some(MoverData {
                        pose: mat,
                        hand: i,
                        hand_pose,
                    });
                    self.drag = None;
                    log::info!("Start space rotate");
                    return;
                }
            }
        }

        if let Some(data) = self.drag.as_mut() {
            let pointer = &app.input_state.pointers[data.hand];
            if !pointer.now.space_drag {
                self.drag = None;
                log::info!("End space drag");
                return;
            }

            let new_hand = data
                .pose
                .transform_point3a(app.input_state.pointers[data.hand].raw_pose.translation);

            let relative_pos = if app.session.config.space_drag_unlocked {
                new_hand - data.hand_pose
            } else {
                vec3a(0., new_hand.y - data.hand_pose.y, 0.)
            } * app.session.config.space_drag_multiplier;

            if relative_pos.length_squared() > 1000.0 {
                log::warn!("Space drag too fast, ignoring");
                return;
            }

            let overlay_offset = data.pose.inverse().transform_vector3a(relative_pos) * -1.0;

            overlays.values_mut().for_each(|overlay| {
                let Some(state) = overlay.config.active_state.as_mut() else {
                    return;
                };
                if state.positioning.moves_with_space() {
                    state.transform.translation += overlay_offset;
                    overlay.config.dirty = true;
                }
            });

            data.pose.translation += relative_pos;
            data.hand_pose = new_hand;

            if self.universe == ETrackingUniverseOrigin::TrackingUniverseStanding {
                apply_chaperone_offset(overlay_offset, chaperone_mgr);
            }
            set_working_copy(&universe, chaperone_mgr, &data.pose);
            chaperone_mgr.commit_working_copy(EChaperoneConfigFile::EChaperoneConfigFile_Live);
        } else {
            for (i, pointer) in app.input_state.pointers.iter().enumerate() {
                if pointer.now.space_drag {
                    let Some(mat) = get_working_copy(&universe, chaperone_mgr) else {
                        log::warn!("Can't space drag - failed to get zero pose");
                        return;
                    };
                    let hand_pos = mat.transform_point3a(pointer.raw_pose.translation);
                    self.drag = Some(MoverData {
                        pose: mat,
                        hand: i,
                        hand_pose: hand_pos,
                    });
                    self.rotate = None;
                    log::info!("Start space drag");
                    return;
                }
            }
        }

        for (_, pointer) in app.input_state.pointers.iter().enumerate() {
            if pointer.now.space_reset && !pointer.before.space_reset {
                self.reset_offset(chaperone_mgr, &app.input_state);
                log::info!("Space reset");
                return;
            }
        }
    }

    pub fn reset_offset(&mut self, chaperone_mgr: &mut ChaperoneSetupManager, input: &InputState) {
        let mut height = 1.6;
        if let Some(mat) = get_working_copy(&self.universe, chaperone_mgr) {
            height = input.hmd.translation.y - mat.translation.y;
            if self.universe == ETrackingUniverseOrigin::TrackingUniverseStanding {
                apply_chaperone_transform(mat, chaperone_mgr);
            }
        }

        let xform = if self.universe == ETrackingUniverseOrigin::TrackingUniverseSeated {
            Affine3A::from_translation(Vec3::NEG_Y * height)
        } else {
            Affine3A::IDENTITY
        };

        set_working_copy(&self.universe, chaperone_mgr, &xform);
        chaperone_mgr.commit_working_copy(EChaperoneConfigFile::EChaperoneConfigFile_Live);

        if self.drag.is_some() {
            log::info!("Space drag interrupted by manual reset");
            self.drag = None;
        }
        if self.rotate.is_some() {
            log::info!("Space rotate interrupted by manual reset");
            self.rotate = None;
        }
    }

    pub fn fix_floor(&mut self, chaperone_mgr: &mut ChaperoneSetupManager, input: &InputState) {
        let y1 = input.pointers[0].pose.translation.y;
        let y2 = input.pointers[1].pose.translation.y;
        let Some(mut mat) = get_working_copy(&self.universe, chaperone_mgr) else {
            log::warn!("Can't fix floor - failed to get zero pose");
            return;
        };
        let offset = y1.min(y2) - 0.03;
        mat.translation.y += offset;

        set_working_copy(&self.universe, chaperone_mgr, &mat);
        chaperone_mgr.commit_working_copy(EChaperoneConfigFile::EChaperoneConfigFile_Live);

        if self.drag.is_some() {
            log::info!("Space drag interrupted by fix floor");
            self.drag = None;
        }
        if self.rotate.is_some() {
            log::info!("Space rotate interrupted by fix floor");
            self.rotate = None;
        }
    }

    pub fn recenter(&mut self, chaperone_mgr: &mut ChaperoneSetupManager, input: &InputState) {
        let Some(mut mat) = get_working_copy(&self.universe, chaperone_mgr) else {
            log::warn!("Can't recenter - failed to get zero pose");
            return;
        };

        mat.translation.x += input.hmd.translation.x;
        mat.translation.z += input.hmd.translation.z;

        set_working_copy(&self.universe, chaperone_mgr, &mat);
        chaperone_mgr.commit_working_copy(EChaperoneConfigFile::EChaperoneConfigFile_Live);

        if self.drag.is_some() {
            log::info!("Space drag interrupted by recenter");
            self.drag = None;
        }
        if self.rotate.is_some() {
            log::info!("Space rotate interrupted by recenter");
            self.rotate = None;
        }
    }

    pub fn playspace_changed(
        &mut self,
        compositor_mgr: &mut CompositorManager,
        _chaperone_mgr: &mut ChaperoneSetupManager,
    ) {
        let new_universe = compositor_mgr.get_tracking_space();
        if new_universe != self.universe {
            log::info!(
                "Playspace changed: {} -> {}",
                universe_str(&self.universe),
                universe_str(&new_universe)
            );
            self.universe = new_universe;
        }

        if self.drag.is_some() {
            log::info!("Space drag interrupted by external change");
            self.drag = None;
        }
        if self.rotate.is_some() {
            log::info!("Space rotate interrupted by external change");
            self.rotate = None;
        }
    }

    pub fn get_universe(&self) -> ETrackingUniverseOrigin {
        self.universe.clone()
    }
}

const fn universe_str(universe: &ETrackingUniverseOrigin) -> &'static str {
    match universe {
        ETrackingUniverseOrigin::TrackingUniverseSeated => "Seated",
        ETrackingUniverseOrigin::TrackingUniverseStanding => "Standing",
        ETrackingUniverseOrigin::TrackingUniverseRawAndUncalibrated => "Raw",
    }
}

fn get_working_copy(
    universe: &ETrackingUniverseOrigin,
    chaperone_mgr: &mut ChaperoneSetupManager,
) -> Option<Affine3A> {
    chaperone_mgr.revert_working_copy();
    let mat = match universe {
        ETrackingUniverseOrigin::TrackingUniverseStanding => {
            chaperone_mgr.get_working_standing_zero_pose_to_raw_tracking_pose()
        }
        _ => chaperone_mgr.get_working_seated_zero_pose_to_raw_tracking_pose(),
    };
    mat.map(|m| m.to_affine())
}

fn set_working_copy(
    universe: &ETrackingUniverseOrigin,
    chaperone_mgr: &mut ChaperoneSetupManager,
    mat: &Affine3A,
) {
    let mat = HmdMatrix34_t::from_affine(mat);
    match universe {
        ETrackingUniverseOrigin::TrackingUniverseStanding => {
            chaperone_mgr.set_working_standing_zero_pose_to_raw_tracking_pose(&mat);
        }
        _ => chaperone_mgr.set_working_seated_zero_pose_to_raw_tracking_pose(&mat),
    }
}

fn apply_chaperone_offset(offset: Vec3A, chaperone_mgr: &mut ChaperoneSetupManager) {
    let mut quads = chaperone_mgr.get_live_collision_bounds_info();
    for quad in &mut quads {
        quad.vCorners.iter_mut().for_each(|corner| {
            corner.v[0] += offset.x;
            corner.v[2] += offset.z;
        });
    }
    chaperone_mgr.set_working_collision_bounds_info(quads.as_mut_slice());
}

fn apply_chaperone_transform(transform: Affine3A, chaperone_mgr: &mut ChaperoneSetupManager) {
    let mut quads = chaperone_mgr.get_live_collision_bounds_info();
    for quad in &mut quads {
        quad.vCorners.iter_mut().for_each(|corner| {
            let coord = transform.transform_point3a(Vec3A::from_slice(&corner.v));
            corner.v[0] = coord.x;
            corner.v[2] = coord.z;
        });
    }
    chaperone_mgr.set_working_collision_bounds_info(quads.as_mut_slice());
}
