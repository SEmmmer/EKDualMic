use anyhow::{Context, Result};
use common_types::{AudioFrame, CHANNELS, OutputBackend, OutputConfig, SAMPLE_RATE_HZ};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub trait OutputSink: Send {
    fn write_frame(&mut self, frame: &AudioFrame) -> Result<()>;

    fn finalize(&mut self) -> Result<()> {
        Ok(())
    }
}

pub fn build_output_sink(config: &OutputConfig) -> Result<Box<dyn OutputSink>> {
    match config.backend {
        OutputBackend::Null => Ok(Box::new(NullOutputSink)),
        OutputBackend::WavDump => Ok(Box::new(WavWriterSink::create(&config.wav_path)?)),
        OutputBackend::VirtualStub => {
            Ok(Box::new(VirtualMicStub::new(config.target_device.clone())))
        }
    }
}

pub struct NullOutputSink;

impl OutputSink for NullOutputSink {
    fn write_frame(&mut self, _frame: &AudioFrame) -> Result<()> {
        Ok(())
    }
}

pub struct VirtualMicStub {
    #[allow(dead_code)]
    device_name: String,
}

impl VirtualMicStub {
    pub fn new(device_name: String) -> Self {
        Self { device_name }
    }
}

impl OutputSink for VirtualMicStub {
    fn write_frame(&mut self, _frame: &AudioFrame) -> Result<()> {
        Ok(())
    }
}

pub struct WavWriterSink {
    writer: Option<WavWriter<BufWriter<File>>>,
}

impl WavWriterSink {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }

        let spec = WavSpec {
            channels: CHANNELS as u16,
            sample_rate: SAMPLE_RATE_HZ,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let writer = WavWriter::create(path, spec)
            .with_context(|| format!("failed to create WAV writer at {}", path.display()))?;

        Ok(Self {
            writer: Some(writer),
        })
    }
}

impl OutputSink for WavWriterSink {
    fn write_frame(&mut self, frame: &AudioFrame) -> Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            for sample in &frame.samples {
                writer
                    .write_sample(*sample)
                    .context("failed to write WAV sample")?;
            }
        }

        Ok(())
    }

    fn finalize(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.take() {
            writer.finalize().context("failed to finalize WAV writer")?;
        }

        Ok(())
    }
}

pub fn default_debug_wav_path(base_dir: &Path, stem: &str) -> PathBuf {
    base_dir.join(format!("{stem}.wav"))
}
