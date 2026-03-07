use anyhow::{Context, Result, bail};
use common_types::{
    AudioBackend, AudioConfig, AudioDeviceInfo, AudioFrame, CHANNELS, SAMPLE_RATE_HZ,
    SAMPLES_PER_FRAME, now_micros,
};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
#[cfg(windows)]
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, IAudioCaptureClient,
    IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eCapture,
    eConsole,
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

pub trait CaptureSource {
    fn read_frame(&mut self) -> Result<AudioFrame>;
    fn device_name(&self) -> &str;
}

#[cfg(windows)]
pub fn list_capture_devices() -> Result<Vec<AudioDeviceInfo>> {
    let com_initialized = init_com_for_wasapi("failed to initialize COM for capture device probe")?;
    let result = (|| -> Result<Vec<AudioDeviceInfo>> {
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .context("failed to create MMDeviceEnumerator for capture device probe")?
        };

        let default_id = unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) }
            .ok()
            .and_then(|device| capture_device_id(&device).ok());
        let devices = unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }
            .context("failed to enumerate active capture endpoints")?;
        let count = unsafe { devices.GetCount() }.context("failed to count capture endpoints")?;

        let mut infos = Vec::with_capacity(count as usize);
        for index in 0..count {
            let device = unsafe { devices.Item(index) }
                .with_context(|| format!("failed to open capture endpoint #{index}"))?;
            let id = capture_device_id(&device).unwrap_or_default();
            let name = capture_device_name(&device).unwrap_or_else(|_| format!("endpoint-{index}"));
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
pub fn list_capture_devices() -> Result<Vec<AudioDeviceInfo>> {
    bail!("capture device enumeration is only available on Windows")
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
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    pending_samples: VecDeque<f32>,
    sequence: u64,
    com_initialized: bool,
}

#[cfg(windows)]
impl WindowsCaptureSource {
    pub fn try_default(config: &AudioConfig) -> Result<Self> {
        if config.sample_rate != SAMPLE_RATE_HZ {
            bail!(
                "WASAPI capture currently requires sample_rate={} but got {}",
                SAMPLE_RATE_HZ,
                config.sample_rate
            );
        }

        if config.channels as usize != CHANNELS {
            bail!(
                "WASAPI capture currently requires channels={} but got {}",
                CHANNELS,
                config.channels
            );
        }

        let com_initialized = init_com_for_wasapi("failed to initialize COM for WASAPI capture")?;

        let result = (|| -> Result<Self> {
            let enumerator: IMMDeviceEnumerator = unsafe {
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .context("failed to create MMDeviceEnumerator")?
            };

            let device = select_capture_device(&enumerator, &config.input_device)?;
            let device_name = capture_device_name(&device)
                .or_else(|_| capture_device_id(&device))
                .unwrap_or_else(|_| "wasapi-capture".to_owned());

            let audio_client: IAudioClient = unsafe {
                device
                    .Activate(CLSCTX_ALL, None)
                    .with_context(|| format!("failed to activate capture device `{device_name}`"))?
            };

            let mut capture_format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
                nChannels: CHANNELS as u16,
                nSamplesPerSec: SAMPLE_RATE_HZ,
                nAvgBytesPerSec: SAMPLE_RATE_HZ * 4,
                nBlockAlign: 4,
                wBitsPerSample: 32,
                cbSize: 0,
            };

            let buffer_duration_hns = frame_count_to_hns(SAMPLES_PER_FRAME as u32 * 4);
            let stream_flags =
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

            unsafe {
                audio_client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    buffer_duration_hns,
                    0,
                    &mut capture_format,
                    None,
                )
            }
            .with_context(|| {
                format!(
                    "failed to initialize WASAPI capture for `{device_name}` with 48 kHz mono float32"
                )
            })?;

            let capture_client = unsafe {
                audio_client
                    .GetService::<IAudioCaptureClient>()
                    .context("failed to acquire IAudioCaptureClient service")?
            };

            unsafe { audio_client.Start() }.context("failed to start WASAPI capture stream")?;

            Ok(Self {
                device_name,
                audio_client,
                capture_client,
                pending_samples: VecDeque::with_capacity(SAMPLES_PER_FRAME * 2),
                sequence: 0,
                com_initialized,
            })
        })();

        if result.is_err() && com_initialized {
            unsafe { CoUninitialize() };
        }

        result
    }

    fn fill_pending_samples(&mut self, required_samples: usize) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(500);
        while self.pending_samples.len() < required_samples {
            self.read_available_packets()?;
            if self.pending_samples.len() >= required_samples {
                break;
            }

            if Instant::now() >= deadline {
                bail!(
                    "timed out waiting for captured audio from `{}`",
                    self.device_name
                );
            }

            thread::sleep(Duration::from_millis(1));
        }

        Ok(())
    }

    fn read_available_packets(&mut self) -> Result<()> {
        loop {
            let packet_frames = unsafe { self.capture_client.GetNextPacketSize() }
                .context("failed to query next WASAPI capture packet size")?;
            if packet_frames == 0 {
                break;
            }

            let mut data = std::ptr::null_mut();
            let mut frames_to_read = 0_u32;
            let mut flags = 0_u32;
            unsafe {
                self.capture_client.GetBuffer(
                    &mut data,
                    &mut frames_to_read,
                    &mut flags,
                    None,
                    None,
                )
            }
            .context("failed to read from WASAPI capture buffer")?;

            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                self.pending_samples
                    .extend(std::iter::repeat_n(0.0_f32, frames_to_read as usize));
            } else {
                if data.is_null() {
                    unsafe { self.capture_client.ReleaseBuffer(frames_to_read) }
                        .context("failed to release null WASAPI capture buffer")?;
                    bail!("WASAPI capture returned a null audio buffer");
                }

                let sample_count = frames_to_read as usize * CHANNELS;
                let samples =
                    unsafe { std::slice::from_raw_parts(data as *const f32, sample_count) };
                self.pending_samples.extend(samples.iter().copied());
            }

            unsafe { self.capture_client.ReleaseBuffer(frames_to_read) }
                .context("failed to release WASAPI capture buffer")?;
        }

        Ok(())
    }
}

#[cfg(windows)]
fn init_com_for_wasapi(context: &str) -> Result<bool> {
    let init_hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if init_hr.is_err() && init_hr != RPC_E_CHANGED_MODE {
        init_hr.ok().with_context(|| context.to_owned())?;
    }
    Ok(init_hr.is_ok())
}

#[cfg(windows)]
impl CaptureSource for WindowsCaptureSource {
    fn read_frame(&mut self) -> Result<AudioFrame> {
        self.fill_pending_samples(SAMPLES_PER_FRAME)?;
        let samples = self.pending_samples.drain(..SAMPLES_PER_FRAME).collect();
        self.sequence += 1;

        Ok(AudioFrame::new(
            self.sequence,
            now_micros(),
            SAMPLE_RATE_HZ,
            samples,
        ))
    }

    fn device_name(&self) -> &str {
        &self.device_name
    }
}

#[cfg(windows)]
impl Drop for WindowsCaptureSource {
    fn drop(&mut self) {
        let _ = unsafe { self.audio_client.Stop() };
        if self.com_initialized {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(windows)]
fn select_capture_device(
    enumerator: &IMMDeviceEnumerator,
    requested_name: &str,
) -> Result<IMMDevice> {
    let requested_name = requested_name.trim();
    if requested_name.is_empty() || requested_name.eq_ignore_ascii_case("default") {
        return unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) }
            .context("failed to get default capture endpoint");
    }

    let devices = unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }
        .context("failed to enumerate active capture endpoints")?;
    let count = unsafe { devices.GetCount() }.context("failed to count capture endpoints")?;

    let mut available_names = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = unsafe { devices.Item(index) }
            .with_context(|| format!("failed to open capture endpoint #{index}"))?;
        let name = capture_device_name(&device).unwrap_or_else(|_| format!("endpoint-{index}"));
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
    bail!("failed to locate capture device `{requested_name}`. Active capture devices: {available}")
}

#[cfg(windows)]
fn capture_device_name(device: &IMMDevice) -> Result<String> {
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }
        .context("failed to open capture device property store")?;
    let mut value: PROPVARIANT = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
        .with_context(
            || "failed to read PKEY_Device_FriendlyName from capture device property store",
        )?;

    let converted = unsafe { PropVariantToStringAlloc(&value) }
        .context("failed to convert capture device name property to string")?;
    let name = unsafe { converted.to_string() }
        .context("capture device name property is not valid UTF-16")?;

    unsafe {
        CoTaskMemFree(Some(converted.0.cast()));
        PropVariantClear(&mut value)?;
    }

    Ok(name)
}

#[cfg(windows)]
fn capture_device_id(device: &IMMDevice) -> Result<String> {
    let identifier = unsafe { device.GetId() }.context("failed to read capture device id")?;
    let result = unsafe { identifier.to_string() }.context("capture device id is not valid UTF-16");
    unsafe {
        CoTaskMemFree(Some(identifier.0.cast()));
    }
    result
}

#[cfg(windows)]
fn frame_count_to_hns(frame_count: u32) -> i64 {
    (10_000_000_i64 * frame_count as i64) / SAMPLE_RATE_HZ as i64
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
