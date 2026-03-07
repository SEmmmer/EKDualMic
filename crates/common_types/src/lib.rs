use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const CHANNELS: usize = 1;
pub const FRAME_MS: usize = 10;
pub const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE_HZ as usize * FRAME_MS) / 1_000;

pub type Sample = f32;

pub fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros() as u64)
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub sequence: u64,
    pub capture_timestamp_us: u64,
    pub sample_rate: u32,
    pub samples: Vec<Sample>,
}

impl AudioFrame {
    pub fn new(
        sequence: u64,
        capture_timestamp_us: u64,
        sample_rate: u32,
        samples: Vec<Sample>,
    ) -> Self {
        Self {
            sequence,
            capture_timestamp_us,
            sample_rate,
            samples: normalize_samples(samples),
        }
    }

    pub fn zero(sequence: u64) -> Self {
        Self::new(
            sequence,
            now_micros(),
            SAMPLE_RATE_HZ,
            vec![0.0; SAMPLES_PER_FRAME],
        )
    }

    pub fn with_sequence(&self, sequence: u64) -> Self {
        let mut cloned = self.clone();
        cloned.sequence = sequence;
        cloned
    }

    pub fn with_timestamp(&self, capture_timestamp_us: u64) -> Self {
        let mut cloned = self.clone();
        cloned.capture_timestamp_us = capture_timestamp_us;
        cloned
    }

    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }

        let sum = self
            .samples
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>();

        (sum / self.samples.len() as f64).sqrt() as f32
    }

    pub fn peak(&self) -> f32 {
        self.samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max)
    }

    pub fn correlation(&self, other: &Self) -> f32 {
        if self.samples.is_empty() || other.samples.is_empty() {
            return 0.0;
        }

        let (dot, lhs, rhs) = self.samples.iter().zip(other.samples.iter()).fold(
            (0.0_f64, 0.0_f64, 0.0_f64),
            |(dot, lhs, rhs), (left, right)| {
                (
                    dot + (*left as f64 * *right as f64),
                    lhs + (*left as f64 * *left as f64),
                    rhs + (*right as f64 * *right as f64),
                )
            },
        );

        if lhs <= f64::EPSILON || rhs <= f64::EPSILON {
            return 0.0;
        }

        (dot / (lhs.sqrt() * rhs.sqrt())).clamp(-1.0, 1.0) as f32
    }
}

fn normalize_samples(mut samples: Vec<Sample>) -> Vec<Sample> {
    samples.truncate(SAMPLES_PER_FRAME);

    if samples.len() < SAMPLES_PER_FRAME {
        samples.resize(SAMPLES_PER_FRAME, 0.0);
    }

    samples
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VadDecision {
    pub score: f32,
    pub is_speech: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SyncReport {
    pub coarse_delay_ms: f32,
    pub drift_ppm: f32,
    pub coherence: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CancelReport {
    pub filter_frozen: bool,
    pub estimated_crosstalk_rms: f32,
    pub output_rms: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TransportStats {
    pub sent_packets: u64,
    pub received_packets: u64,
    pub concealed_packets: u64,
    pub dropped_packets: u64,
}

impl TransportStats {
    pub fn loss_rate(&self) -> f32 {
        let delivered = self.received_packets + self.concealed_packets;
        if delivered == 0 {
            0.0
        } else {
            self.concealed_packets as f32 / delivered as f32
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeSnapshot {
    pub node_name: String,
    pub sequence: u64,
    pub coarse_delay_ms: f32,
    pub drift_ppm: f32,
    pub coherence: f32,
    pub local_vad: VadDecision,
    pub peer_vad: VadDecision,
    pub update_frozen: bool,
    pub transport_loss_rate: f32,
    pub sent_packets: u64,
    pub received_packets: u64,
    pub concealed_packets: u64,
    pub input_rms: f32,
    pub output_rms: f32,
    pub estimated_crosstalk_rms: f32,
    pub clip_events: u64,
    pub processing_time_us: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TransportBackend {
    #[default]
    Udp,
    Mock,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    #[default]
    Wasapi,
    Mock,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OutputBackend {
    #[default]
    VirtualStub,
    WavDump,
    Null,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct NodeConfig {
    pub node: NodeSection,
    pub audio: AudioConfig,
    pub output: OutputConfig,
    pub sync: SyncConfig,
    pub cancel: CancelConfig,
    pub vad: VadConfig,
    pub residual: ResidualConfig,
    pub debug: DebugConfig,
    pub gui: GuiConfig,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            node: NodeSection::default(),
            audio: AudioConfig::default(),
            output: OutputConfig::default(),
            sync: SyncConfig::default(),
            cancel: CancelConfig::default(),
            vad: VadConfig::default(),
            residual: ResidualConfig::default(),
            debug: DebugConfig::default(),
            gui: GuiConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct NodeSection {
    pub name: String,
    pub listen_addr: String,
    pub peer_addr: String,
    pub transport_backend: TransportBackend,
}

impl Default for NodeSection {
    fn default() -> Self {
        Self {
            name: "node-a".to_owned(),
            listen_addr: "0.0.0.0:38001".to_owned(),
            peer_addr: "127.0.0.1:38002".to_owned(),
            transport_backend: TransportBackend::Udp,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AudioConfig {
    pub backend: AudioBackend,
    pub input_device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_ms: u16,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            backend: AudioBackend::Wasapi,
            input_device: "Microphone".to_owned(),
            sample_rate: SAMPLE_RATE_HZ,
            channels: CHANNELS as u16,
            frame_ms: FRAME_MS as u16,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct OutputConfig {
    pub backend: OutputBackend,
    pub target_device: String,
    pub wav_path: PathBuf,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            backend: OutputBackend::VirtualStub,
            target_device: "Processed Mic".to_owned(),
            wav_path: PathBuf::from("artifacts/output.wav"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct SyncConfig {
    pub jitter_buffer_frames: u16,
    pub coarse_search_ms: u16,
    pub drift_compensation: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            jitter_buffer_frames: 3,
            coarse_search_ms: 30,
            drift_compensation: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct CancelConfig {
    pub filter_length: usize,
    pub step_size: f32,
    pub leakage: f32,
    pub update_threshold: f32,
}

impl Default for CancelConfig {
    fn default() -> Self {
        Self {
            filter_length: 1_536,
            step_size: 0.04,
            leakage: 0.0001,
            update_threshold: 0.65,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct VadConfig {
    pub enabled: bool,
    pub local_threshold: f32,
    pub peer_threshold: f32,
    pub smoothing: f32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            local_threshold: 0.6,
            peer_threshold: 0.6,
            smoothing: 0.85,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ResidualConfig {
    pub enabled: bool,
    pub strength: f32,
}

impl Default for ResidualConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strength: 0.2,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct DebugConfig {
    pub dump_wav: bool,
    pub dump_metrics: bool,
    pub dump_dir: PathBuf,
    pub log_level: String,
    pub mock_peer_delay_ms: u16,
    pub mock_peer_gain: f32,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            dump_wav: true,
            dump_metrics: true,
            dump_dir: PathBuf::from("artifacts"),
            log_level: "info".to_owned(),
            mock_peer_delay_ms: 20,
            mock_peer_gain: 0.35,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct GuiConfig {
    pub auto_start: bool,
    pub refresh_hz: u16,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            refresh_hz: 30,
        }
    }
}
