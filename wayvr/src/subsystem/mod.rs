#[cfg(feature = "pipewire")]
pub mod audio;
pub mod dbus;
pub mod hid;
pub mod input;
pub mod notifications;

#[cfg(feature = "whisper")]
pub mod clipboard;

#[cfg(feature = "whisper")]
pub mod whisper_stt;

#[cfg(feature = "osc")]
pub mod osc;

#[cfg(feature = "openxr")]
#[cfg(feature = "feat-monado-metrics")]
pub mod monado_metrics;
