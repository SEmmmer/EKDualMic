use anyhow::{Result, bail};
use common_types::{AudioBackend, AudioConfig, AudioFrame, SAMPLES_PER_FRAME, now_micros};
use std::f32::consts::TAU;

pub trait CaptureSource: Send {
    fn read_frame(&mut self) -> Result<AudioFrame>;
    fn device_name(&self) -> &str;
}

pub fn build_capture_source(config: &AudioConfig) -> Result<Box<dyn CaptureSource>> {
    match config.backend {
        AudioBackend::Mock => Ok(Box::new(SyntheticCaptureSource::new(
            config.input_device.clone(),
            config.sample_rate as f32,
        ))),
        AudioBackend::Wasapi => {
            let capture = WindowsCaptureSource::try_default(config)?;
            Ok(Box::new(capture))
        }
    }
}

pub struct SyntheticCaptureSource {
    device_name: String,
    sample_rate: f32,
    sequence: u64,
    phase: f32,
}

impl SyntheticCaptureSource {
    pub fn new(device_name: String, sample_rate: f32) -> Self {
        Self {
            device_name,
            sample_rate,
            sequence: 0,
            phase: 0.0,
        }
    }
}

impl CaptureSource for SyntheticCaptureSource {
    fn read_frame(&mut self) -> Result<AudioFrame> {
        let base_frequency = 220.0_f32;
        let phase_step = TAU * base_frequency / self.sample_rate.max(1.0);

        let mut samples = Vec::with_capacity(SAMPLES_PER_FRAME);
        for index in 0..SAMPLES_PER_FRAME {
            let t = self.phase + phase_step * index as f32;
            let harmonic = (t * 2.0).sin() * 0.04;
            let carrier = t.sin() * 0.12;
            samples.push(carrier + harmonic);
        }

        self.phase += phase_step * SAMPLES_PER_FRAME as f32;
        self.sequence += 1;

        Ok(AudioFrame::new(
            self.sequence,
            now_micros(),
            self.sample_rate as u32,
            samples,
        ))
    }

    fn device_name(&self) -> &str {
        &self.device_name
    }
}

#[cfg(windows)]
pub struct WindowsCaptureSource {
    device_name: String,
}

#[cfg(windows)]
impl WindowsCaptureSource {
    pub fn try_default(config: &AudioConfig) -> Result<Self> {
        bail!(
            "WASAPI capture backend is reserved for the next implementation phase: {}",
            config.input_device
        )
    }
}

#[cfg(windows)]
impl CaptureSource for WindowsCaptureSource {
    fn read_frame(&mut self) -> Result<AudioFrame> {
        bail!("WASAPI capture backend is not implemented yet")
    }

    fn device_name(&self) -> &str {
        &self.device_name
    }
}

#[cfg(not(windows))]
pub struct WindowsCaptureSource;

#[cfg(not(windows))]
impl WindowsCaptureSource {
    pub fn try_default(_config: &AudioConfig) -> Result<Self> {
        bail!("WASAPI capture backend is only available on Windows")
    }
}

#[cfg(not(windows))]
impl CaptureSource for WindowsCaptureSource {
    fn read_frame(&mut self) -> Result<AudioFrame> {
        bail!("WASAPI capture backend is only available on Windows")
    }

    fn device_name(&self) -> &str {
        "wasapi-unavailable"
    }
}
