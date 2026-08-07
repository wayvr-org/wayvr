use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use wlx_common::audio::rodio::{
    Source,
    microphone::{MicrophoneBuilder, available_inputs},
};

const WHISPER_SAMPLE_RATE: usize = 16_000;
const MAX_DURATION: Duration = Duration::from_secs(30);
const UNLOAD_AFTER: Duration = Duration::from_mins(5);

#[derive(Clone, Debug)]
pub struct WhisperSttConfig {
    pub model_path: PathBuf,
    pub language: Option<String>,

    pub initial_prompt: Option<String>,
    pub n_threads: i32,

    /// lower values reduce release-time lag but cost more CPU/GPU
    pub partial_decode_interval_ms: u64,

    /// ignore extremely short accidental taps
    pub min_audio_ms: u64,

    /// force a specific recording device; see `rodio::microphone::available_inputs()`
    pub rodio_input_device_name: Option<String>,

    pub use_gpu: bool,
    pub gpu_device: i32,
    pub flash_attn: bool,
}

impl WhisperSttConfig {
    pub fn new(model_path: impl AsRef<Path>) -> Self {
        let n_threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(4) as i32);

        Self {
            model_path: model_path.as_ref().to_path_buf(),
            language: None,
            initial_prompt: None,
            n_threads,
            partial_decode_interval_ms: 700,
            min_audio_ms: 250,
            rodio_input_device_name: None,
            use_gpu: true,
            gpu_device: 0,
            flash_attn: false,
        }
    }
}

#[derive(Debug)]
pub enum WhisperSttError {
    ModelLoad(String),
    Whisper(String),
    Rodio(String),
    CaptureInit(String),
    ThreadSpawn(String),
    CaptureThreadPanicked,
    AlreadyRecording,
    NotRecording,
}

impl fmt::Display for WhisperSttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelLoad(e) => write!(f, "failed to load whisper model: {e}"),
            Self::Whisper(e) => write!(f, "whisper error: {e}"),
            Self::Rodio(e) => write!(f, "rodio error: {e}"),
            Self::CaptureInit(e) => write!(f, "failed to initialize capture: {e}"),
            Self::ThreadSpawn(e) => write!(f, "failed to spawn thread: {e}"),
            Self::CaptureThreadPanicked => write!(f, "capture thread panicked"),
            Self::AlreadyRecording => write!(f, "PTT is already active"),
            Self::NotRecording => write!(f, "PTT is not active"),
        }
    }
}

impl std::error::Error for WhisperSttError {}

struct StopCapture;

struct CaptureSession {
    stop_tx: mpsc::Sender<StopCapture>,
    capture_thread: Option<JoinHandle<()>>,
    recognizer_thread: Option<JoinHandle<()>>,
    deadline: Instant,
}

pub struct WhisperStt {
    config: WhisperSttConfig,
    ctx: Arc<WhisperContext>,

    active: Option<CaptureSession>,
    finished_recognizers: Vec<JoinHandle<()>>,

    completed_rx: mpsc::Receiver<Result<String, String>>,
    completed_tx: mpsc::Sender<Result<String, String>>,

    last_error: Option<String>,
    unload_at: Instant,
}

impl WhisperStt {
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self, WhisperSttError> {
        Self::init(WhisperSttConfig::new(model_path))
    }

    pub fn init(config: WhisperSttConfig) -> Result<Self, WhisperSttError> {
        let ctx_params = WhisperContextParameters {
            use_gpu: config.use_gpu,
            gpu_device: config.gpu_device,
            flash_attn: config.flash_attn,
            ..Default::default()
        };

        let ctx = WhisperContext::new_with_params(&config.model_path, ctx_params)
            .map_err(|e| WhisperSttError::ModelLoad(e.to_string()))?;

        let (completed_tx, completed_rx) = mpsc::channel();

        Ok(Self {
            config,
            ctx: Arc::new(ctx),
            active: None,
            finished_recognizers: Vec::new(),
            completed_rx,
            completed_tx,
            last_error: None,
            unload_at: Instant::now() + UNLOAD_AFTER,
        })
    }

    /// starts a fresh capture stream and a transcription worker
    pub fn ptt_start(&mut self) -> Result<(), WhisperSttError> {
        self.unload_at = Instant::now() + UNLOAD_AFTER;
        self.reap_finished_recognizers();

        if self.active.is_some() {
            return Err(WhisperSttError::AlreadyRecording);
        }

        let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<StopCapture>();

        let recognizer_thread = spawn_recognizer_thread(
            Arc::clone(&self.ctx),
            self.config.clone(),
            audio_rx,
            self.completed_tx.clone(),
        )?;

        let input_device_name = self.config.rodio_input_device_name.clone();

        let capture_thread = thread::Builder::new()
            .name("whisper-stt-rodio-capture".to_string())
            .spawn(move || {
                rodio_capture_thread(audio_tx, stop_rx, input_device_name, ready_tx);
            })
            .map_err(|e| WhisperSttError::ThreadSpawn(e.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.active = Some(CaptureSession {
                    stop_tx,
                    capture_thread: Some(capture_thread),
                    recognizer_thread: Some(recognizer_thread),
                    deadline: Instant::now() + MAX_DURATION,
                });

                Ok(())
            }
            Ok(Err(e)) => {
                let _ = stop_tx.send(StopCapture);
                let _ = capture_thread.join();
                let _ = recognizer_thread.join();

                Err(WhisperSttError::CaptureInit(e))
            }
            Err(e) => {
                let _ = stop_tx.send(StopCapture);
                let _ = capture_thread.join();
                let _ = recognizer_thread.join();

                Err(WhisperSttError::CaptureInit(e.to_string()))
            }
        }
    }

    fn stop_active_capture(&mut self) -> Result<(), WhisperSttError> {
        let Some(mut session) = self.active.take() else {
            return Err(WhisperSttError::NotRecording);
        };

        let _ = session.stop_tx.send(StopCapture);

        let capture_result = if let Some(capture_thread) = session.capture_thread.take() {
            capture_thread
                .join()
                .map_err(|_| WhisperSttError::CaptureThreadPanicked)
        } else {
            Ok(())
        };

        if let Some(recognizer_thread) = session.recognizer_thread.take() {
            self.finished_recognizers.push(recognizer_thread);
        }

        capture_result
    }

    fn drain_completed_transcriptions(&mut self) -> Option<String> {
        let mut latest = None;

        while let Ok(result) = self.completed_rx.try_recv() {
            match result {
                Ok(text) => {
                    let text = normalize_transcript(text);
                    if !text.is_empty() {
                        latest = Some(text);
                    }
                }
                Err(e) => {
                    self.last_error = Some(e);
                }
            }
        }

        latest
    }

    /// stops the pw stream & finalizes recognition asynchronously
    /// poll `take_transcription()` from your main loop to receive transcription
    pub fn ptt_end(&mut self) -> Result<(), WhisperSttError> {
        self.unload_at = Instant::now() + UNLOAD_AFTER;
        self.stop_active_capture()
    }

    pub fn take_transcription(&mut self) -> Option<String> {
        self.reap_finished_recognizers();

        let latest = self.drain_completed_transcriptions();

        if latest.is_some() {
            self.unload_at = Instant::now() + UNLOAD_AFTER;
            return latest;
        }

        // been recording for too long, force send a stop signal
        if self
            .active
            .as_ref()
            .is_some_and(|session| Instant::now() >= session.deadline)
            && let Err(e) = self.stop_active_capture()
        {
            self.last_error = Some(e.to_string());
        }

        None
    }

    pub fn should_unload(&self) -> bool {
        self.unload_at < Instant::now()
    }

    fn reap_finished_recognizers(&mut self) {
        let mut i = 0;

        while i < self.finished_recognizers.len() {
            if self.finished_recognizers[i].is_finished() {
                let handle = self.finished_recognizers.swap_remove(i);
                let _ = handle.join();
            } else {
                i += 1;
            }
        }
    }
}

impl Drop for WhisperStt {
    fn drop(&mut self) {
        if self.active.is_some() {
            let _ = self.ptt_end();
        }

        for handle in self.finished_recognizers.drain(..) {
            let _ = handle.join();
        }
    }
}

fn spawn_recognizer_thread(
    ctx: Arc<WhisperContext>,
    config: WhisperSttConfig,
    audio_rx: mpsc::Receiver<Vec<f32>>,
    completed_tx: mpsc::Sender<Result<String, String>>,
) -> Result<JoinHandle<()>, WhisperSttError> {
    thread::Builder::new()
        .name("whisper-stt-recognizer".to_string())
        .spawn(move || {
            recognizer_thread(ctx, config, audio_rx, completed_tx);
        })
        .map_err(|e| WhisperSttError::ThreadSpawn(e.to_string()))
}

fn recognizer_thread(
    ctx: Arc<WhisperContext>,
    config: WhisperSttConfig,
    audio_rx: mpsc::Receiver<Vec<f32>>,
    completed_tx: mpsc::Sender<Result<String, String>>,
) {
    let partial_stride_samples =
        ms_to_samples(config.partial_decode_interval_ms).max(WHISPER_SAMPLE_RATE / 4);
    let min_samples = ms_to_samples(config.min_audio_ms);

    let mut audio = Vec::<f32>::new();
    let mut last_decoded_len = 0usize;
    let mut latest_partial = String::new();

    while let Ok(chunk) = audio_rx.recv() {
        if chunk.is_empty() {
            continue;
        }

        audio.extend_from_slice(&chunk);

        let enough_new_audio =
            audio.len().saturating_sub(last_decoded_len) >= partial_stride_samples;

        if audio.len() >= min_samples && enough_new_audio {
            if let Ok(text) = transcribe_audio(&ctx, &config, &audio) {
                latest_partial = text;
                last_decoded_len = audio.len();
            } else {
                // do not fail the session on a speculative decode
                // the final decode after PTT end gets reported
            }
        }
    }

    if audio.len() < min_samples {
        let _ = completed_tx.send(Ok(String::new()));
        return;
    }

    match transcribe_audio(&ctx, &config, &audio) {
        Ok(text) => {
            let _ = completed_tx.send(Ok(text));
        }
        Err(e) if !latest_partial.trim().is_empty() => {
            // Prefer a recent partial over losing the utterance completely.
            let _ = completed_tx.send(Ok(latest_partial));
            let _ = completed_tx.send(Err(e.to_string()));
        }
        Err(e) => {
            let _ = completed_tx.send(Err(e.to_string()));
        }
    }
}

fn transcribe_audio(
    ctx: &WhisperContext,
    config: &WhisperSttConfig,
    audio: &[f32],
) -> Result<String, WhisperSttError> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    params.set_n_threads(config.n_threads);
    params.set_language(config.language.as_deref());
    params.set_no_timestamps(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    if let Some(prompt) = config.initial_prompt.as_deref() {
        params.set_initial_prompt(prompt);
    }

    let mut state = ctx
        .create_state()
        .map_err(|e| WhisperSttError::Whisper(e.to_string()))?;

    state
        .full(params, audio)
        .map_err(|e| WhisperSttError::Whisper(e.to_string()))?;

    let text = state
        .as_iter()
        .map(|segment| segment.to_string())
        .collect::<String>();

    Ok(normalize_transcript(text))
}

fn rodio_capture_thread(
    audio_tx: mpsc::Sender<Vec<f32>>,
    stop_rx: mpsc::Receiver<StopCapture>,
    input_device_name: Option<String>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) {
    let mut ready_tx = Some(ready_tx);

    let result = run_rodio_capture(audio_tx, stop_rx, input_device_name, &mut ready_tx);

    if let Err(e) = result
        && let Some(ready_tx) = ready_tx.take()
    {
        let _ = ready_tx.send(Err(e.to_string()));
    }
}

fn run_rodio_capture(
    audio_tx: mpsc::Sender<Vec<f32>>,
    stop_rx: mpsc::Receiver<StopCapture>,
    input_device_name: Option<String>,
    ready_tx: &mut Option<mpsc::Sender<Result<(), String>>>,
) -> Result<(), WhisperSttError> {
    let builder = MicrophoneBuilder::new();

    let builder = if let Some(input_device_name) = input_device_name {
        let inputs = available_inputs().map_err(|e| WhisperSttError::Rodio(e.to_string()))?;
        let input_device_name_lower = input_device_name.to_lowercase();

        let input = inputs
            .into_iter()
            .find(|input| {
                input
                    .to_string()
                    .to_lowercase()
                    .contains(&input_device_name_lower)
            })
            .ok_or_else(|| {
                WhisperSttError::Rodio(format!(
                    "no rodio input device matched {input_device_name:?}"
                ))
            })?;

        builder
            .device(input)
            .map_err(|e| WhisperSttError::Rodio(e.to_string()))?
    } else {
        builder
            .default_device()
            .map_err(|e| WhisperSttError::Rodio(e.to_string()))?
    };

    let builder = builder
        .default_config()
        .map_err(|e| WhisperSttError::Rodio(e.to_string()))?
        .prefer_channel_counts([
            1.try_into().expect("not zero"),
            2.try_into().expect("not zero"),
        ])
        .prefer_sample_rates([
            16_000.try_into().expect("not zero"),
            32_000.try_into().expect("not zero"),
            48_000.try_into().expect("not zero"),
        ])
        .prefer_buffer_sizes(512..);

    let mut mic = builder
        .open_stream()
        .map_err(|e| WhisperSttError::Rodio(e.to_string()))?;

    let channels = mic.channels().get() as usize;
    let input_rate = mic.sample_rate().get() as usize;

    if let Some(ready_tx) = ready_tx.take() {
        let _ = ready_tx.send(Ok(()));
    }

    let mut resampler = StreamingResampler::default();
    let mut interleaved = Vec::new();

    // ~20 ms of input frames; whisper still receives 16 kHz mono chunks
    let chunk_input_samples = ((input_rate / 50).max(1)) * channels.max(1);

    'capture: loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        interleaved.clear();

        while interleaved.len() < chunk_input_samples {
            if stop_rx.try_recv().is_ok() {
                break 'capture;
            }

            let Some(sample) = mic.next() else {
                return Err(WhisperSttError::Rodio(
                    "microphone stream ended unexpectedly".to_string(),
                ));
            };

            // Rodio's default sample type is f32. This cast also keeps the code
            // compiling if the crate is built with rodio's `64bit` feature.
            interleaved.push(sample);
        }

        let resampled_vec = resampler.push_interleaved_mono_16k(&interleaved, channels, input_rate);

        if !resampled_vec.is_empty() && audio_tx.send(resampled_vec).is_err() {
            break;
        }
    }

    Ok(())
}

#[derive(Default)]
struct StreamingResampler {
    pending: Vec<f32>,
    position: f64,
    input_rate: usize,
}

impl StreamingResampler {
    fn push_interleaved_mono_16k(
        &mut self,
        samples: &[f32],
        channels: usize,
        input_rate: usize,
    ) -> Vec<f32> {
        if channels == 0 || input_rate == 0 {
            return Vec::new();
        }

        if self.input_rate != input_rate {
            self.pending.clear();
            self.position = 0.0;
            self.input_rate = input_rate;
        }

        let frames = samples.len() / channels;
        if frames == 0 {
            return Vec::new();
        }

        let mut mono = Vec::with_capacity(frames);

        for frame in 0..frames {
            let frame_start = frame * channels;
            let mut sum = 0.0f32;

            for ch in 0..channels {
                sum += samples[frame_start + ch];
            }

            mono.push(sum / channels as f32);
        }

        self.pending.extend_from_slice(&mono);

        let step = input_rate as f64 / WHISPER_SAMPLE_RATE as f64;
        let mut out = Vec::with_capacity(
            ((self.pending.len() as f64 - self.position) / step).max(0.0) as usize,
        );

        #[allow(clippy::while_float)]
        while self.position + 1.0 < self.pending.len() as f64 {
            let i = self.position.floor() as usize;
            let frac = (self.position - i as f64) as f32;

            let a = self.pending[i];
            let b = self.pending[i + 1];

            out.push(a + (b - a) * frac);

            self.position += step;
        }

        let drop_count = self.position.floor() as usize;
        if drop_count > 0 {
            self.pending.drain(..drop_count);
            self.position -= drop_count as f64;
        }

        out
    }
}

const fn ms_to_samples(ms: u64) -> usize {
    ((ms as usize) * WHISPER_SAMPLE_RATE) / 1000
}

fn normalize_transcript(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
