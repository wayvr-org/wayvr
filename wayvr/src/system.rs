use crate::{backend::task::SystemTask, state::AppState};

/// Volume change per HMD button click
#[cfg(feature = "pipewire")]
const VOLUME_STEP: f32 = 0.05;

pub struct SystemTaskManager;

impl SystemTaskManager {
    pub fn handle_task(app: &mut AppState, task: SystemTask) {
        match task {
            SystemTask::ChangeVolume(increase) => {
                #[cfg(feature = "pipewire")]
                if let Some(audio) = &app.audio_service {
                    let delta = if increase { VOLUME_STEP } else { -VOLUME_STEP };
                    audio.task(wlx_common::audio_devices::AudioTask::AddDefaultSinkVolume(
                        delta,
                    ));
                    return;
                }

                let _ = (app, increase);
                log::warn!("Cannot change volume: audio service unavailable");
            }
        }
    }
}

#[cfg(feature = "pipewire")]
pub fn show_volume_osd(app: &mut AppState) {
    use wlx_common::overlays::ToastTopic;

    use crate::overlays::osd::Osd;
    use crate::subsystem::audio::AudioEvent;

    let Some(audio) = &app.audio_service else {
        return;
    };

    let Some(AudioEvent::DefaultSinkVolume {
        description,
        volume,
        mute,
    }) = audio.poll_events().last()
    else {
        return;
    };

    Osd {
        topic: if mute {
            ToastTopic::Mute
        } else {
            ToastTopic::VolumeChange
        },
        title: description,
        value: format!("{:.0}%", volume * 100.0),
        level: Some(volume),
    }
    .show(app);
}
