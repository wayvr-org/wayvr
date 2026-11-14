use crate::backend::task::SystemTask;

/// Volume change per HMD button click
const VOLUME_STEP: f32 = 0.05;

pub struct SystemTaskManager;

impl SystemTaskManager {
    pub fn handle_task(task: SystemTask) -> anyhow::Result<()> {
        match task {
            SystemTask::ChangeVolume(increase) => {
                Self::update_volume(if increase { VOLUME_STEP } else { -VOLUME_STEP })
            }
        }
        Ok(())
    }

    fn update_volume(delta: f32) {
        // pactl is a subprocess; keep it off the frame loop.
        std::thread::spawn(move || {
            if let Err(e) = wlx_common::pactl_wrapper::add_default_sink_volume(delta) {
                log::warn!("Could not change volume: {e:?}");
            }
        });
    }
}
