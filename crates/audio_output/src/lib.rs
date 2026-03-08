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
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, eConsole, eRender,
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
    Ok(Box::new(VirtualMicStub::new(
        config.primary_target_device.clone(),
    )))
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
    render_spec: RenderFormatSpec,
    com_initialized: bool,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
struct RenderFormatSpec {
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    sample_format: RenderSampleFormat,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
enum RenderSampleFormat {
    Float32,
    Pcm16,
    Pcm24,
    Pcm32,
}

#[cfg(windows)]
const KSDATAFORMAT_SUBTYPE_PCM_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);

#[cfg(windows)]
impl WindowsRenderSink {
    pub fn try_default(config: &OutputConfig) -> Result<Self> {
        let com_initialized = init_com_for_output("failed to initialize COM for WASAPI output")?;

        let result = (|| -> Result<Self> {
            let enumerator: IMMDeviceEnumerator = unsafe {
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .context("failed to create MMDeviceEnumerator for output")?
            };

            let device = select_render_device(&enumerator, &config.primary_target_device)?;
            let device_name = render_device_name(&device)
                .or_else(|_| render_device_id(&device))
                .unwrap_or_else(|_| "wasapi-render".to_owned());

            let audio_client: IAudioClient = unsafe {
                device
                    .Activate(CLSCTX_ALL, None)
                    .with_context(|| format!("failed to activate render device `{device_name}`"))?
            };

            let mix_format_ptr = unsafe { audio_client.GetMixFormat() }
                .context("failed to query WASAPI render mix format")?;
            let render_spec = describe_render_format(unsafe { &*mix_format_ptr })?;
            let buffer_duration_hns = 10_000_000_i64;
            let stream_flags = if render_spec.sample_rate == SAMPLE_RATE_HZ {
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
            } else {
                Default::default()
            };

            unsafe {
                audio_client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    stream_flags,
                    buffer_duration_hns,
                    0,
                    mix_format_ptr,
                    None,
                )
            }
            .with_context(|| {
                format!(
                    "failed to initialize WASAPI render for `{device_name}` with device mix format {} Hz, {} channels, {:?}",
                    render_spec.sample_rate,
                    render_spec.channels,
                    render_spec.sample_format
                )
            })?;
            unsafe { CoTaskMemFree(Some(mix_format_ptr.cast())) };

            let render_client = unsafe {
                audio_client
                    .GetService::<IAudioRenderClient>()
                    .context("failed to acquire IAudioRenderClient service")?
            };
            let buffer_frames = unsafe { audio_client.GetBufferSize() }
                .context("failed to query WASAPI render buffer size")?;

            prime_render_buffer_with_silence(
                &render_client,
                buffer_frames,
                render_spec.block_align,
            )?;
            unsafe { audio_client.Start() }.context("failed to start WASAPI render stream")?;

            Ok(Self {
                device_name,
                audio_client,
                render_client,
                buffer_frames,
                render_spec,
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
        const LIVE_OUTPUT_GAIN: f32 = 1.0;

        if frame.sample_rate != SAMPLE_RATE_HZ {
            anyhow::bail!(
                "WASAPI output currently requires sample_rate={} but got {}",
                SAMPLE_RATE_HZ,
                frame.sample_rate
            );
        }

        let source_frames = frame.samples.len() / CHANNELS;
        let mono_samples = if self.render_spec.sample_rate == SAMPLE_RATE_HZ {
            frame.samples.clone()
        } else {
            resample_mono_frame(
                &frame.samples,
                source_frames,
                self.render_spec.sample_rate,
                SAMPLE_RATE_HZ,
            )
        };
        let mono_samples = mono_samples
            .into_iter()
            .map(|sample| soft_limit_monitor(sample * LIVE_OUTPUT_GAIN))
            .collect::<Vec<_>>();
        let requested_frames = mono_samples.len() as u32;
        self.wait_for_available_frames(requested_frames)?;

        let data = unsafe { self.render_client.GetBuffer(requested_frames) }
            .context("failed to acquire WASAPI render buffer")?;
        if data.is_null() {
            anyhow::bail!("WASAPI render returned a null buffer");
        }

        let byte_count = requested_frames as usize * self.render_spec.block_align as usize;
        let destination = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, byte_count) };
        write_mono_frame_to_output_buffer(destination, &mono_samples, self.render_spec);

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
fn prime_render_buffer_with_silence(
    render_client: &IAudioRenderClient,
    buffer_frames: u32,
    block_align: u16,
) -> Result<()> {
    if buffer_frames == 0 {
        return Ok(());
    }

    let data = unsafe { render_client.GetBuffer(buffer_frames) }
        .context("failed to acquire initial WASAPI render buffer")?;
    if data.is_null() {
        anyhow::bail!("WASAPI render returned a null buffer during initial priming");
    }

    let byte_count = buffer_frames as usize * block_align as usize;
    let destination = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, byte_count) };
    destination.fill(0);

    unsafe { render_client.ReleaseBuffer(buffer_frames, 0) }
        .context("failed to release initial WASAPI render buffer")?;
    Ok(())
}

#[cfg(windows)]
fn describe_render_format(format: &WAVEFORMATEX) -> Result<RenderFormatSpec> {
    const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;

    let channels = unsafe { std::ptr::addr_of!(format.nChannels).read_unaligned() }.max(1);
    let sample_rate = unsafe { std::ptr::addr_of!(format.nSamplesPerSec).read_unaligned() }.max(1);
    let block_align = unsafe { std::ptr::addr_of!(format.nBlockAlign).read_unaligned() }.max(1);
    let format_tag = unsafe { std::ptr::addr_of!(format.wFormatTag).read_unaligned() };
    let bits_per_sample = unsafe { std::ptr::addr_of!(format.wBitsPerSample).read_unaligned() };

    let sample_format = match format_tag {
        value if value == WAVE_FORMAT_IEEE_FLOAT as u16 => RenderSampleFormat::Float32,
        value if value == WAVE_FORMAT_PCM as u16 => pcm_format_from_bits(bits_per_sample)?,
        WAVE_FORMAT_EXTENSIBLE_TAG => {
            let extensible =
                unsafe { &*(format as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE) };
            let sub_format = unsafe { std::ptr::addr_of!(extensible.SubFormat).read_unaligned() };
            if sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                RenderSampleFormat::Float32
            } else if sub_format == KSDATAFORMAT_SUBTYPE_PCM_GUID {
                pcm_format_from_bits(bits_per_sample)?
            } else {
                anyhow::bail!("unsupported WASAPI render sub-format {:?}", sub_format);
            }
        }
        other => anyhow::bail!("unsupported WASAPI render format tag {other}"),
    };

    Ok(RenderFormatSpec {
        channels,
        sample_rate,
        block_align,
        sample_format,
    })
}

#[cfg(windows)]
fn pcm_format_from_bits(bits_per_sample: u16) -> Result<RenderSampleFormat> {
    match bits_per_sample {
        16 => Ok(RenderSampleFormat::Pcm16),
        24 => Ok(RenderSampleFormat::Pcm24),
        32 => Ok(RenderSampleFormat::Pcm32),
        other => anyhow::bail!("unsupported PCM render bit depth {other}"),
    }
}

fn resample_mono_frame(
    mono_samples: &[f32],
    input_frames: usize,
    output_sample_rate: u32,
    input_sample_rate: u32,
) -> Vec<f32> {
    if output_sample_rate == input_sample_rate || input_frames <= 1 {
        return mono_samples.to_vec();
    }

    let output_frames = ((input_frames as u64 * output_sample_rate as u64)
        / input_sample_rate as u64)
        .max(1) as usize;
    let ratio = input_sample_rate as f32 / output_sample_rate as f32;
    let mut output = Vec::with_capacity(output_frames);
    for output_index in 0..output_frames {
        let source_position = output_index as f32 * ratio;
        let left_index = source_position.floor() as usize;
        let right_index = (left_index + 1).min(mono_samples.len().saturating_sub(1));
        let fraction = source_position - left_index as f32;
        let left = mono_samples[left_index.min(mono_samples.len().saturating_sub(1))];
        let right = mono_samples[right_index];
        output.push(left + (right - left) * fraction);
    }
    output
}

fn write_mono_frame_to_output_buffer(
    destination: &mut [u8],
    mono_samples: &[f32],
    format: RenderFormatSpec,
) {
    let channel_count = format.channels as usize;
    let bytes_per_sample = (format.block_align as usize / channel_count.max(1)).max(1);
    let frame_count = destination.len() / format.block_align as usize;

    for frame_index in 0..frame_count {
        let sample = mono_samples
            .get(frame_index)
            .copied()
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0);
        for channel_index in 0..channel_count {
            let byte_offset =
                frame_index * format.block_align as usize + channel_index * bytes_per_sample;
            match format.sample_format {
                RenderSampleFormat::Float32 => {
                    destination[byte_offset..byte_offset + 4]
                        .copy_from_slice(&sample.to_le_bytes());
                }
                RenderSampleFormat::Pcm16 => {
                    let value = (sample * i16::MAX as f32).round() as i16;
                    destination[byte_offset..byte_offset + 2].copy_from_slice(&value.to_le_bytes());
                }
                RenderSampleFormat::Pcm24 => {
                    let value = (sample * 8_388_607.0).round() as i32;
                    let bytes = value.to_le_bytes();
                    destination[byte_offset..byte_offset + 3].copy_from_slice(&bytes[..3]);
                }
                RenderSampleFormat::Pcm32 => {
                    let value = (sample * i32::MAX as f32).round() as i32;
                    destination[byte_offset..byte_offset + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
}

fn soft_limit_monitor(sample: f32) -> f32 {
    sample.clamp(-1.0, 1.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_mono_frame_duplicates_samples_across_channels() {
        let mut destination = vec![0_u8; 24];
        write_mono_frame_to_output_buffer(
            &mut destination,
            &[0.25, -0.5],
            RenderFormatSpec {
                channels: 3,
                sample_rate: SAMPLE_RATE_HZ,
                block_align: 12,
                sample_format: RenderSampleFormat::Float32,
            },
        );

        let samples = destination
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("chunk should be 4 bytes")))
            .collect::<Vec<_>>();
        assert_eq!(samples, vec![0.25, 0.25, 0.25, -0.5, -0.5, -0.5]);
    }

    #[test]
    fn write_mono_frame_zero_fills_tail_when_destination_is_larger() {
        let mut destination = vec![255_u8; 16];
        write_mono_frame_to_output_buffer(
            &mut destination,
            &[0.2],
            RenderFormatSpec {
                channels: 2,
                sample_rate: SAMPLE_RATE_HZ,
                block_align: 8,
                sample_format: RenderSampleFormat::Float32,
            },
        );

        let samples = destination
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("chunk should be 4 bytes")))
            .collect::<Vec<_>>();
        assert_eq!(samples, vec![0.2, 0.2, 0.0, 0.0]);
    }

    #[test]
    fn resample_mono_frame_changes_frame_count_for_non_48k_output() {
        let input = vec![0.0; 480];
        let output = resample_mono_frame(&input, 480, 44_100, 48_000);
        assert_eq!(output.len(), 441);
    }

    #[test]
    fn soft_limit_monitor_caps_hot_samples_without_silencing_them() {
        let sample = soft_limit_monitor(1.6);
        assert_eq!(sample, 1.0);
    }
}
