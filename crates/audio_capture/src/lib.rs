use anyhow::{Context, Result, bail};
use common_types::{
    AudioBackend, AudioConfig, AudioDeviceInfo, AudioFrame, CHANNELS, SAMPLE_RATE_HZ,
    SAMPLES_PER_FRAME, now_micros,
};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
#[cfg(windows)]
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
#[cfg(windows)]
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    DEVICE_STATE_ACTIVE, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eCapture, eConsole,
};
#[cfg(windows)]
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
#[cfg(windows)]
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToStringAlloc,
};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

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
    resample_buffer: VecDeque<f32>,
    resample_position: f64,
    capture_spec: CaptureFormatSpec,
    receive_signal: windows::Win32::Foundation::HANDLE,
    sequence: u64,
    com_initialized: bool,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
struct CaptureFormatSpec {
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    sample_format: CaptureSampleFormat,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
enum CaptureSampleFormat {
    Float32,
    Pcm16,
    Pcm24,
    Pcm32,
}

#[cfg(windows)]
const KSDATAFORMAT_SUBTYPE_PCM_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);

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

            let capture_format_ptr = unsafe { audio_client.GetMixFormat() }
                .context("failed to query WASAPI capture mix format")?;
            let capture_spec = describe_capture_format(unsafe { &*capture_format_ptr })?;

            let buffer_duration_hns = 10_000_000_i64;
            let receive_signal = unsafe { CreateEventW(None, false, false, None) }
                .context("failed to create WASAPI capture receive event")?;
            let stream_flags = if capture_spec.sample_rate == SAMPLE_RATE_HZ
                && capture_spec.channels as usize == CHANNELS
                && matches!(capture_spec.sample_format, CaptureSampleFormat::Float32)
            {
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            } else {
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            };

            unsafe {
                audio_client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    buffer_duration_hns,
                    0,
                    capture_format_ptr,
                    None,
                )
            }
            .with_context(|| {
                format!(
                    "failed to initialize WASAPI capture for `{device_name}` with device mix format {} Hz, {} channels, {:?}",
                    capture_spec.sample_rate,
                    capture_spec.channels,
                    capture_spec.sample_format
                )
            })?;
            unsafe { CoTaskMemFree(Some(capture_format_ptr.cast())) };

            let capture_client = unsafe {
                audio_client
                    .GetService::<IAudioCaptureClient>()
                    .context("failed to acquire IAudioCaptureClient service")?
            };
            unsafe { audio_client.SetEventHandle(receive_signal) }
                .context("failed to attach WASAPI capture event handle")?;

            unsafe { audio_client.Start() }.context("failed to start WASAPI capture stream")?;

            Ok(Self {
                device_name,
                audio_client,
                capture_client,
                pending_samples: VecDeque::with_capacity(SAMPLES_PER_FRAME * 2),
                resample_buffer: VecDeque::with_capacity(SAMPLES_PER_FRAME * 2),
                resample_position: 0.0,
                capture_spec,
                receive_signal,
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

            unsafe { WaitForSingleObject(self.receive_signal, 5) };
        }

        Ok(())
    }

    fn push_capture_samples(&mut self, samples: &[f32]) {
        if self.capture_spec.sample_rate == SAMPLE_RATE_HZ {
            self.pending_samples.extend(samples.iter().copied());
            return;
        }

        self.resample_buffer.extend(samples.iter().copied());
        let step = self.capture_spec.sample_rate as f64 / SAMPLE_RATE_HZ as f64;

        while self.resample_buffer.len() >= 2 {
            let left_index = self.resample_position.floor() as usize;
            if left_index + 1 >= self.resample_buffer.len() {
                break;
            }

            let fraction = self.resample_position - left_index as f64;
            let left = *self
                .resample_buffer
                .get(left_index)
                .expect("left index should exist");
            let right = *self
                .resample_buffer
                .get(left_index + 1)
                .expect("right index should exist");
            let sample = left + (right - left) * fraction as f32;
            self.pending_samples.push_back(sample);

            self.resample_position += step;
            let consumed = self.resample_position.floor() as usize;
            if consumed > 0 {
                for _ in 0..consumed.min(self.resample_buffer.len().saturating_sub(1)) {
                    self.resample_buffer.pop_front();
                }
                self.resample_position -= consumed as f64;
            }
        }
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
                let silence = vec![0.0_f32; frames_to_read as usize];
                self.push_capture_samples(&silence);
            } else {
                if data.is_null() {
                    unsafe { self.capture_client.ReleaseBuffer(frames_to_read) }
                        .context("failed to release null WASAPI capture buffer")?;
                    bail!("WASAPI capture returned a null audio buffer");
                }

                let byte_count = frames_to_read as usize * self.capture_spec.block_align as usize;
                let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, byte_count) };
                let mono_samples = decode_capture_packet_to_mono(
                    bytes,
                    frames_to_read as usize,
                    self.capture_spec,
                )?;
                self.push_capture_samples(&mono_samples);
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
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.receive_signal) };
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
fn describe_capture_format(format: &WAVEFORMATEX) -> Result<CaptureFormatSpec> {
    const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;

    let channels = unsafe { std::ptr::addr_of!(format.nChannels).read_unaligned() }.max(1);
    let sample_rate = unsafe { std::ptr::addr_of!(format.nSamplesPerSec).read_unaligned() }.max(1);
    let block_align = unsafe { std::ptr::addr_of!(format.nBlockAlign).read_unaligned() }.max(1);
    let format_tag = unsafe { std::ptr::addr_of!(format.wFormatTag).read_unaligned() };
    let bits_per_sample = unsafe { std::ptr::addr_of!(format.wBitsPerSample).read_unaligned() };

    let sample_format = match format_tag {
        value if value == WAVE_FORMAT_IEEE_FLOAT as u16 => CaptureSampleFormat::Float32,
        value if value == WAVE_FORMAT_PCM as u16 => pcm_capture_format_from_bits(bits_per_sample)?,
        WAVE_FORMAT_EXTENSIBLE_TAG => {
            let extensible =
                unsafe { &*(format as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE) };
            let sub_format = unsafe { std::ptr::addr_of!(extensible.SubFormat).read_unaligned() };
            if sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                CaptureSampleFormat::Float32
            } else if sub_format == KSDATAFORMAT_SUBTYPE_PCM_GUID {
                pcm_capture_format_from_bits(bits_per_sample)?
            } else {
                bail!("unsupported WASAPI capture sub-format {:?}", sub_format);
            }
        }
        other => bail!("unsupported WASAPI capture format tag {other}"),
    };

    Ok(CaptureFormatSpec {
        channels,
        sample_rate,
        block_align,
        sample_format,
    })
}

#[cfg(windows)]
fn pcm_capture_format_from_bits(bits_per_sample: u16) -> Result<CaptureSampleFormat> {
    match bits_per_sample {
        16 => Ok(CaptureSampleFormat::Pcm16),
        24 => Ok(CaptureSampleFormat::Pcm24),
        32 => Ok(CaptureSampleFormat::Pcm32),
        other => bail!("unsupported PCM capture bit depth {other}"),
    }
}

#[cfg(windows)]
fn decode_capture_packet_to_mono(
    bytes: &[u8],
    frame_count: usize,
    format: CaptureFormatSpec,
) -> Result<Vec<f32>> {
    let channel_count = format.channels as usize;
    let bytes_per_sample = (format.block_align as usize / channel_count.max(1)).max(1);
    let mut mono = Vec::with_capacity(frame_count);

    for frame_index in 0..frame_count {
        let frame_offset = frame_index * format.block_align as usize;
        let mut sum = 0.0_f32;
        for channel_index in 0..channel_count {
            let offset = frame_offset + channel_index * bytes_per_sample;
            let sample = match format.sample_format {
                CaptureSampleFormat::Float32 => {
                    let chunk: [u8; 4] = bytes[offset..offset + 4]
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("invalid float32 capture packet size"))?;
                    f32::from_le_bytes(chunk)
                }
                CaptureSampleFormat::Pcm16 => {
                    let chunk: [u8; 2] = bytes[offset..offset + 2]
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("invalid pcm16 capture packet size"))?;
                    i16::from_le_bytes(chunk) as f32 / i16::MAX as f32
                }
                CaptureSampleFormat::Pcm24 => {
                    let chunk = &bytes[offset..offset + 3];
                    let sign = if chunk[2] & 0x80 != 0 { 0xFF } else { 0x00 };
                    let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], sign]);
                    value as f32 / 8_388_607.0
                }
                CaptureSampleFormat::Pcm32 => {
                    let chunk: [u8; 4] = bytes[offset..offset + 4]
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("invalid pcm32 capture packet size"))?;
                    i32::from_le_bytes(chunk) as f32 / i32::MAX as f32
                }
            };
            sum += sample;
        }

        mono.push(sum / channel_count as f32);
    }

    Ok(mono)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_pcm16_stereo_capture_packet_to_mono() {
        let bytes = [
            0x00_u8, 0x40, 0x00, 0x40, // 0.5, 0.5
            0x00, 0x20, 0x00, 0xE0, // 0.25, -0.25
        ];
        let mono = decode_capture_packet_to_mono(
            &bytes,
            2,
            CaptureFormatSpec {
                channels: 2,
                sample_rate: SAMPLE_RATE_HZ,
                block_align: 4,
                sample_format: CaptureSampleFormat::Pcm16,
            },
        )
        .expect("pcm16 stereo packet should decode");

        assert!((mono[0] - 0.5).abs() < 0.01);
        assert!(mono[1].abs() < 0.01);
    }

    #[test]
    fn decode_float32_mono_capture_packet_preserves_samples() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0.25_f32.to_le_bytes());
        bytes.extend_from_slice(&(-0.75_f32).to_le_bytes());
        let mono = decode_capture_packet_to_mono(
            &bytes,
            2,
            CaptureFormatSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE_HZ,
                block_align: 4,
                sample_format: CaptureSampleFormat::Float32,
            },
        )
        .expect("float32 packet should decode");

        assert_eq!(mono, vec![0.25, -0.75]);
    }

    #[test]
    fn decode_float32_mono_capture_packet_does_not_hard_clip_over_range_samples() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1.25_f32.to_le_bytes());
        bytes.extend_from_slice(&(-1.10_f32).to_le_bytes());
        let mono = decode_capture_packet_to_mono(
            &bytes,
            2,
            CaptureFormatSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE_HZ,
                block_align: 4,
                sample_format: CaptureSampleFormat::Float32,
            },
        )
        .expect("float32 packet should decode");

        assert_eq!(mono, vec![1.25, -1.10]);
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
