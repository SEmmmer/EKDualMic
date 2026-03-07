use anyhow::{Context, Result};
use common_types::{
    AudioDeviceInfo, AudioFrame, CHANNELS, OutputBackend, OutputConfig, SAMPLE_RATE_HZ,
};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
#[cfg(windows)]
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, IAudioClient, IAudioRenderClient,
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eConsole, eRender,
};
#[cfg(windows)]
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
#[cfg(windows)]
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToStringAlloc,
};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};

pub trait OutputSink {
    fn write_frame(&mut self, frame: &AudioFrame) -> Result<()>;

    fn finalize(&mut self) -> Result<()> {
        Ok(())
    }
}

pub fn build_output_sink(config: &OutputConfig) -> Result<Box<dyn OutputSink>> {
    match config.backend {
        OutputBackend::Null => Ok(Box::new(NullOutputSink)),
        OutputBackend::WavDump => Ok(Box::new(WavWriterSink::create(&config.wav_path)?)),
        OutputBackend::VirtualStub => build_virtual_output_sink(config),
    }
}

#[cfg(windows)]
pub fn list_render_devices() -> Result<Vec<AudioDeviceInfo>> {
    let com_initialized = init_com_for_output("failed to initialize COM for render device probe")?;
    let result = (|| -> Result<Vec<AudioDeviceInfo>> {
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("failed to create MMDeviceEnumerator for render device probe")?
        };

        let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .ok()
            .and_then(|device| render_device_id(&device).ok());
        let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .context("failed to enumerate active render endpoints")?;
        let count = unsafe { devices.GetCount() }.context("failed to count render endpoints")?;

        let mut infos = Vec::with_capacity(count as usize);
        for index in 0..count {
            let device = unsafe { devices.Item(index) }
                .with_context(|| format!("failed to open render endpoint #{index}"))?;
            let id = render_device_id(&device).unwrap_or_default();
            let name = render_device_name(&device).unwrap_or_else(|_| format!("endpoint-{index}"));
            infos.push(AudioDeviceInfo {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            });
        }

        Ok(infos)
    })();

    if com_initialized {
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(not(windows))]
pub fn list_render_devices() -> Result<Vec<AudioDeviceInfo>> {
    anyhow::bail!("render device enumeration is only available on Windows")
}

#[cfg(windows)]
fn build_virtual_output_sink(config: &OutputConfig) -> Result<Box<dyn OutputSink>> {
    Ok(Box::new(WindowsRenderSink::try_default(config)?))
}

#[cfg(not(windows))]
fn build_virtual_output_sink(config: &OutputConfig) -> Result<Box<dyn OutputSink>> {
    Ok(Box::new(VirtualMicStub::new(config.target_device.clone())))
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

#[cfg(windows)]
pub struct WindowsRenderSink {
    device_name: String,
    audio_client: IAudioClient,
    render_client: IAudioRenderClient,
    buffer_frames: u32,
    com_initialized: bool,
}

#[cfg(windows)]
impl WindowsRenderSink {
    pub fn try_default(config: &OutputConfig) -> Result<Self> {
        let com_initialized = init_com_for_output("failed to initialize COM for WASAPI output")?;

        let result = (|| -> Result<Self> {
            let enumerator: IMMDeviceEnumerator = unsafe {
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .context("failed to create MMDeviceEnumerator for output")?
            };

            let device = select_render_device(&enumerator, &config.target_device)?;
            let device_name = render_device_name(&device)
                .or_else(|_| render_device_id(&device))
                .unwrap_or_else(|_| "wasapi-render".to_owned());

            let audio_client: IAudioClient = unsafe {
                device
                    .Activate(CLSCTX_ALL, None)
                    .with_context(|| format!("failed to activate render device `{device_name}`"))?
            };

            let mut render_format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
                nChannels: CHANNELS as u16,
                nSamplesPerSec: SAMPLE_RATE_HZ,
                nAvgBytesPerSec: SAMPLE_RATE_HZ * 4,
                nBlockAlign: 4,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            let buffer_duration_hns = frame_count_to_hns(480 * 4);
            let stream_flags =
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

            unsafe {
                audio_client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    buffer_duration_hns,
                    0,
                    &mut render_format,
                    None,
                )
            }
            .with_context(|| {
                format!(
                    "failed to initialize WASAPI render for `{device_name}` with 48 kHz mono float32"
                )
            })?;

            let render_client = unsafe {
                audio_client
                    .GetService::<IAudioRenderClient>()
                    .context("failed to acquire IAudioRenderClient service")?
            };
            let buffer_frames = unsafe { audio_client.GetBufferSize() }
                .context("failed to query WASAPI render buffer size")?;

            unsafe { audio_client.Start() }.context("failed to start WASAPI render stream")?;

            Ok(Self {
                device_name,
                audio_client,
                render_client,
                buffer_frames,
                com_initialized,
            })
        })();

        if result.is_err() && com_initialized {
            unsafe { CoUninitialize() };
        }

        result
    }

    fn wait_for_available_frames(&self, requested_frames: u32) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let padding = unsafe { self.audio_client.GetCurrentPadding() }
                .context("failed to query WASAPI render padding")?;
            let available_frames = self.buffer_frames.saturating_sub(padding);
            if available_frames >= requested_frames {
                return Ok(());
            }

            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for render buffer space on `{}`",
                    self.device_name
                );
            }

            thread::sleep(Duration::from_millis(1));
        }
    }
}

#[cfg(windows)]
fn init_com_for_output(context: &str) -> Result<bool> {
    let init_hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if init_hr.is_err() && init_hr != RPC_E_CHANGED_MODE {
        init_hr.ok().with_context(|| context.to_owned())?;
    }
    Ok(init_hr.is_ok())
}

#[cfg(windows)]
impl OutputSink for WindowsRenderSink {
    fn write_frame(&mut self, frame: &AudioFrame) -> Result<()> {
        if frame.sample_rate != SAMPLE_RATE_HZ {
            anyhow::bail!(
                "WASAPI output currently requires sample_rate={} but got {}",
                SAMPLE_RATE_HZ,
                frame.sample_rate
            );
        }

        let requested_frames = (frame.samples.len() / CHANNELS) as u32;
        self.wait_for_available_frames(requested_frames)?;

        let data = unsafe { self.render_client.GetBuffer(requested_frames) }
            .context("failed to acquire WASAPI render buffer")?;
        if data.is_null() {
            anyhow::bail!("WASAPI render returned a null buffer");
        }

        let sample_count = requested_frames as usize * CHANNELS;
        let destination = unsafe { std::slice::from_raw_parts_mut(data as *mut f32, sample_count) };
        destination.copy_from_slice(&frame.samples[..sample_count]);

        unsafe { self.render_client.ReleaseBuffer(requested_frames, 0) }
            .context("failed to release WASAPI render buffer")?;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for WindowsRenderSink {
    fn drop(&mut self) {
        let _ = unsafe { self.audio_client.Stop() };
        if self.com_initialized {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(windows)]
fn select_render_device(
    enumerator: &IMMDeviceEnumerator,
    requested_name: &str,
) -> Result<IMMDevice> {
    let requested_name = requested_name.trim();
    if requested_name.is_empty() || requested_name.eq_ignore_ascii_case("default") {
        return unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .context("failed to get default render endpoint");
    }

    let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
        .context("failed to enumerate active render endpoints")?;
    let count = unsafe { devices.GetCount() }.context("failed to count render endpoints")?;

    let mut available_names = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = unsafe { devices.Item(index) }
            .with_context(|| format!("failed to open render endpoint #{index}"))?;
        let name = render_device_name(&device).unwrap_or_else(|_| format!("endpoint-{index}"));
        if name.eq_ignore_ascii_case(requested_name) {
            return Ok(device);
        }
        available_names.push(name);
    }

    let available = if available_names.is_empty() {
        "<none>".to_owned()
    } else {
        available_names.join(", ")
    };
    anyhow::bail!(
        "failed to locate render device `{requested_name}`. Active render devices: {available}"
    )
}

#[cfg(windows)]
fn render_device_name(device: &IMMDevice) -> Result<String> {
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }
        .context("failed to open render device property store")?;
    let mut value: PROPVARIANT = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
        .context("failed to read PKEY_Device_FriendlyName from render device")?;

    let converted = unsafe { PropVariantToStringAlloc(&value) }
        .context("failed to convert render device name property to string")?;
    let name = unsafe { converted.to_string() }
        .context("render device name property is not valid UTF-16")?;

    unsafe {
        CoTaskMemFree(Some(converted.0.cast()));
        PropVariantClear(&mut value)?;
    }

    Ok(name)
}

#[cfg(windows)]
fn render_device_id(device: &IMMDevice) -> Result<String> {
    let identifier = unsafe { device.GetId() }.context("failed to read render device id")?;
    let result = unsafe { identifier.to_string() }.context("render device id is not valid UTF-16");
    unsafe {
        CoTaskMemFree(Some(identifier.0.cast()));
    }
    result
}

#[cfg(windows)]
fn frame_count_to_hns(frame_count: u32) -> i64 {
    (10_000_000_i64 * frame_count as i64) / SAMPLE_RATE_HZ as i64
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
