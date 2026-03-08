use anyhow::{Context, Result};
use audio_cancel::NlmsCanceller;
use audio_capture::{CaptureSource, build_capture_source};
use audio_output::{OutputSink, WavWriterSink, build_output_sink, default_debug_wav_path};
use audio_residual::ResidualSuppressor;
use audio_sync::SyncAligner;
use audio_transport::{TransportLink, build_transport};
use audio_vad::VoiceActivityDetector;
use common_types::{
    AudioBackend, AudioFrame, NodeConfig, OutputBackend, OutputRoutingMode, RuntimeSnapshot,
    SAMPLES_PER_FRAME, SessionMode, TransportBackend, TransportLossWindow, TransportStats,
    VadDecision, now_micros,
};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

const MOCK_SCENE_CYCLE_FRAMES: u64 = 200;
const MOCK_SCENE_RAMP_FRAMES: u64 = 6;

pub struct PipelineRuntime {
    config: NodeConfig,
    capture: Box<dyn CaptureSource>,
    transport: Box<dyn TransportLink>,
    uses_integrated_mock_scene: bool,
    capture_conditioner: CaptureConditioner,
    sync: SyncAligner,
    vad_local: VoiceActivityDetector,
    vad_peer: VoiceActivityDetector,
    cancel_local: NlmsCanceller,
    cancel_peer: NlmsCanceller,
    residual_local: ResidualSuppressor,
    residual_peer: ResidualSuppressor,
    output: OutputRouter,
    debug: DebugRecorder,
    last_snapshot: RuntimeSnapshot,
}

impl PipelineRuntime {
    pub fn new(config: NodeConfig) -> Result<Self> {
        let (capture, transport, uses_integrated_mock_scene) = build_pipeline_io(&config)?;
        let capture_conditioner = CaptureConditioner::default();
        let sync = SyncAligner::new(&config.sync, config.audio.frame_ms as usize);
        let vad_local = VoiceActivityDetector::new(
            config.vad.enabled,
            config.vad.local_threshold,
            config.vad.smoothing,
        );
        let vad_peer = VoiceActivityDetector::new(
            config.vad.enabled,
            config.vad.peer_threshold,
            config.vad.smoothing,
        );
        let cancel_local = NlmsCanceller::new(&config.cancel);
        let cancel_peer = NlmsCanceller::new(&config.cancel);
        let residual_local = ResidualSuppressor::new(&config.residual);
        let residual_peer = ResidualSuppressor::new(&config.residual);
        let output = OutputRouter::new(&config.output)?;
        let debug = DebugRecorder::new(&config)?;

        info!(
            node = %config.node.name,
            capture = %capture.device_name(),
            "pipeline runtime initialized"
        );

        Ok(Self {
            config,
            capture,
            transport,
            uses_integrated_mock_scene,
            capture_conditioner,
            sync,
            vad_local,
            vad_peer,
            cancel_local,
            cancel_peer,
            residual_local,
            residual_peer,
            output,
            debug,
            last_snapshot: RuntimeSnapshot::default(),
        })
    }

    pub fn step(&mut self) -> Result<RuntimeSnapshot> {
        let started_at = Instant::now();

        let capture_raw = self.capture.read_frame()?;
        let local_raw = self.capture_conditioner.process(capture_raw.clone());
        self.transport.send_frame(&local_raw, None)?;

        let mut peer_raw = self.transport.recv_or_conceal()?;
        if self.config.node.transport_backend == TransportBackend::Mock
            && !self.uses_integrated_mock_scene
        {
            for sample in &mut peer_raw.samples {
                *sample *= self.config.debug.mock_peer_gain;
            }
        }

        let (peer_aligned, sync_report) = self.sync.align(peer_raw.clone(), &local_raw);
        let local_vad = self.vad_local.detect(&local_raw);
        let peer_vad = self.vad_peer.detect(&peer_aligned);

        let allow_update = peer_vad.is_speech
            && !near_end_dominant(&local_raw, &peer_aligned, local_vad, peer_vad)
            && sync_report.coherence >= self.config.cancel.update_threshold * 0.85;
        self.cancel_local.set_update_frozen(!allow_update);
        let allow_peer_update = local_vad.is_speech
            && !near_end_dominant(&peer_aligned, &local_raw, peer_vad, local_vad)
            && sync_report.coherence >= self.config.cancel.update_threshold * 0.85;
        self.cancel_peer.set_update_frozen(!allow_peer_update);

        let (local_canceled, cancel_report) = self.cancel_local.process(&local_raw, &peer_aligned);
        let local_output_frame = self.residual_local.process(
            &local_canceled,
            &peer_aligned,
            local_vad,
            peer_vad,
            sync_report.coherence,
            cancel_report.estimated_crosstalk_rms,
        );
        let (peer_canceled, peer_cancel_report) =
            self.cancel_peer.process(&peer_aligned, &local_raw);
        let peer_output_frame = self.residual_peer.process(
            &peer_canceled,
            &local_raw,
            peer_vad,
            local_vad,
            sync_report.coherence,
            peer_cancel_report.estimated_crosstalk_rms,
        );
        self.output.write_frames(
            if self.config.output.backend == OutputBackend::VirtualStub
                && !self.config.output.monitor_processed_output
            {
                &capture_raw
            } else {
                &local_output_frame
            },
            &peer_output_frame,
            self.config.node.session_mode,
        )?;

        let transport_stats = self.transport.stats();
        let clip_events = local_output_frame
            .samples
            .iter()
            .filter(|sample| sample.abs() >= 0.999)
            .count() as u64;

        let snapshot = RuntimeSnapshot {
            node_name: self.config.node.name.clone(),
            sequence: local_raw.sequence,
            coarse_delay_ms: sync_report.coarse_delay_ms,
            drift_ppm: sync_report.drift_ppm,
            coherence: sync_report.coherence,
            local_vad,
            peer_vad,
            update_frozen: cancel_report.filter_frozen,
            transport_loss_rate: transport_stats.loss_rate(),
            sent_packets: transport_stats.sent_packets,
            received_packets: transport_stats.received_packets,
            concealed_packets: transport_stats.concealed_packets,
            input_rms: local_raw.rms(),
            output_rms: local_output_frame.rms(),
            estimated_crosstalk_rms: cancel_report.estimated_crosstalk_rms,
            clip_events,
            processing_time_us: started_at.elapsed().as_micros() as u64,
        };

        self.debug.record(
            &capture_raw,
            &local_raw,
            &peer_raw,
            &peer_aligned,
            &local_output_frame,
            &snapshot,
        )?;
        self.last_snapshot = snapshot.clone();

        Ok(snapshot)
    }

    pub fn last_snapshot(&self) -> &RuntimeSnapshot {
        &self.last_snapshot
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.output.finalize()?;
        self.debug.finalize()?;
        Ok(())
    }
}

struct OutputRouter {
    routing: OutputRoutingMode,
    primary: Option<Box<dyn OutputSink>>,
    secondary: Option<Box<dyn OutputSink>>,
}

impl OutputRouter {
    fn new(config: &common_types::OutputConfig) -> Result<Self> {
        let primary = if matches!(config.routing, OutputRoutingMode::Off) {
            None
        } else {
            Some(build_output_sink(config)?)
        };
        let secondary = if matches!(config.routing, OutputRoutingMode::SplitLocalPeer) {
            let mut secondary_config = config.clone();
            secondary_config.primary_target_device = config.secondary_target_device.clone();
            Some(build_output_sink(&secondary_config)?)
        } else {
            None
        };

        Ok(Self {
            routing: config.routing,
            primary,
            secondary,
        })
    }

    fn write_frames(
        &mut self,
        local_frame: &AudioFrame,
        peer_frame: &AudioFrame,
        session_mode: SessionMode,
    ) -> Result<()> {
        match self.routing {
            OutputRoutingMode::Off => Ok(()),
            OutputRoutingMode::LocalOnly => {
                if let Some(primary) = self.primary.as_mut() {
                    primary.write_frame(local_frame)?;
                }
                Ok(())
            }
            OutputRoutingMode::MixToPrimary => {
                if let Some(primary) = self.primary.as_mut() {
                    let mixed = mix_frames(local_frame, peer_frame);
                    primary.write_frame(&mixed)?;
                }
                Ok(())
            }
            OutputRoutingMode::SplitLocalPeer => {
                if let Some(primary) = self.primary.as_mut() {
                    primary.write_frame(local_frame)?;
                }
                if let Some(secondary) = self.secondary.as_mut() {
                    secondary.write_frame(peer_frame)?;
                } else if matches!(session_mode, SessionMode::MasterSlave | SessionMode::Both) {
                    anyhow::bail!("split_local_peer routing requires a secondary output sink")
                }
                Ok(())
            }
        }
    }

    fn finalize(&mut self) -> Result<()> {
        if let Some(primary) = self.primary.as_mut() {
            primary.finalize()?;
        }
        if let Some(secondary) = self.secondary.as_mut() {
            secondary.finalize()?;
        }
        Ok(())
    }
}

fn mix_frames(local: &AudioFrame, peer: &AudioFrame) -> AudioFrame {
    let samples = local
        .samples
        .iter()
        .zip(peer.samples.iter())
        .map(|(left, right)| ((left + right) * 0.5).clamp(-1.0, 1.0))
        .collect();
    AudioFrame::new(
        local.sequence,
        local.capture_timestamp_us,
        local.sample_rate,
        samples,
    )
}

#[derive(Default)]
struct CaptureConditioner {
    emergency_hold_frames: u32,
}

impl CaptureConditioner {
    fn process(&mut self, mut frame: AudioFrame) -> AudioFrame {
        const EMERGENCY_LIMIT_THRESHOLD: f32 = 1.50;
        const EMERGENCY_TARGET_PEAK: f32 = 0.98;
        const EMERGENCY_HOLD_FRAMES: u32 = 8;

        let peak = frame
            .samples
            .iter()
            .fold(0.0_f32, |current, sample| current.max(sample.abs()));

        if peak > EMERGENCY_LIMIT_THRESHOLD {
            self.emergency_hold_frames = EMERGENCY_HOLD_FRAMES;
        }

        if self.emergency_hold_frames > 0 && peak > 0.0 {
            let gain = EMERGENCY_TARGET_PEAK / peak.max(EMERGENCY_TARGET_PEAK);
            for sample in &mut frame.samples {
                *sample *= gain;
            }
            self.emergency_hold_frames -= 1;
        }

        frame
    }
}

fn near_end_dominant(
    local_raw: &AudioFrame,
    peer_aligned: &AudioFrame,
    local_vad: VadDecision,
    peer_vad: VadDecision,
) -> bool {
    if !local_vad.is_speech {
        return false;
    }

    let local_rms = local_raw.rms();
    let peer_rms = peer_aligned.rms().max(1.0e-4);
    let speech_score_gap = local_vad.score - peer_vad.score;
    speech_score_gap > 0.18 && local_rms > peer_rms * 1.12
}

fn build_pipeline_io(
    config: &NodeConfig,
) -> Result<(Box<dyn CaptureSource>, Box<dyn TransportLink>, bool)> {
    if config.audio.backend == AudioBackend::Mock
        && config.node.transport_backend == TransportBackend::Mock
    {
        let scene = Arc::new(Mutex::new(MockScene::new(
            config.audio.sample_rate as f32,
            config.audio.frame_ms as usize,
            config.debug.mock_peer_delay_ms,
            config.debug.mock_peer_gain,
        )));
        let capture = Box::new(MockSceneCapture::new(
            config.audio.input_device.clone(),
            Arc::clone(&scene),
        )) as Box<dyn CaptureSource>;
        let transport = Box::new(MockSceneTransport::new(scene)) as Box<dyn TransportLink>;
        return Ok((capture, transport, true));
    }

    let capture = build_capture_source(&config.audio)?;
    let transport = build_transport(
        config.node.transport_backend,
        &config.node.listen_addr,
        &config.node.peer_addr,
        config.sync.jitter_buffer_frames as usize,
        config.audio.frame_ms as usize,
        config.debug.mock_peer_delay_ms,
        config.identity(),
        config.identity().expected_peer(),
    )?;
    Ok((capture, transport, false))
}

type SharedMockScene = Arc<Mutex<MockScene>>;

struct MockSceneCapture {
    device_name: String,
    scene: SharedMockScene,
}

impl MockSceneCapture {
    fn new(device_name: String, scene: SharedMockScene) -> Self {
        Self { device_name, scene }
    }
}

impl CaptureSource for MockSceneCapture {
    fn read_frame(&mut self) -> Result<AudioFrame> {
        Ok(self.scene.lock().capture_local_frame())
    }

    fn device_name(&self) -> &str {
        &self.device_name
    }
}

struct MockSceneTransport {
    scene: SharedMockScene,
    loss_window: TransportLossWindow,
    stats: TransportStats,
    last_frame: Option<AudioFrame>,
}

impl MockSceneTransport {
    fn new(scene: SharedMockScene) -> Self {
        Self {
            scene,
            loss_window: TransportLossWindow::default(),
            stats: TransportStats::default(),
            last_frame: None,
        }
    }
}

impl TransportLink for MockSceneTransport {
    fn send_frame(&mut self, _frame: &AudioFrame, _vad: Option<VadDecision>) -> Result<()> {
        self.stats.sent_packets += 1;
        Ok(())
    }

    fn recv_or_conceal(&mut self) -> Result<AudioFrame> {
        if let Some(frame) = self.scene.lock().take_peer_frame() {
            self.stats.received_packets += 1;
            self.loss_window.record_received();
            self.last_frame = Some(frame.clone());
            return Ok(frame);
        }

        self.stats.concealed_packets += 1;
        self.loss_window.record_concealed();
        Ok(self
            .last_frame
            .clone()
            .map(|frame| frame.with_timestamp(now_micros()))
            .unwrap_or_else(|| AudioFrame::zero(0)))
    }

    fn stats(&self) -> TransportStats {
        self.stats.with_loss_window(&self.loss_window)
    }
}

struct MockScene {
    sample_rate: f32,
    sequence: u64,
    peer_delay_frames: usize,
    peer_gain: f32,
    local_voice: VoiceOscillator,
    peer_voice: VoiceOscillator,
    ambience_phase: f32,
    peer_history: VecDeque<AudioFrame>,
    pending_peer_frames: VecDeque<AudioFrame>,
}

impl MockScene {
    fn new(sample_rate: f32, frame_ms: usize, mock_peer_delay_ms: u16, peer_gain: f32) -> Self {
        let peer_delay_frames = mock_peer_delay_ms as usize / frame_ms.max(1);
        Self {
            sample_rate,
            sequence: 0,
            peer_delay_frames,
            peer_gain: peer_gain.clamp(0.0, 1.0),
            local_voice: VoiceOscillator::new(182.0),
            peer_voice: VoiceOscillator::new(268.0),
            ambience_phase: 0.0,
            peer_history: VecDeque::with_capacity(peer_delay_frames + 2),
            pending_peer_frames: VecDeque::new(),
        }
    }

    fn capture_local_frame(&mut self) -> AudioFrame {
        self.sequence += 1;
        let sequence = self.sequence;
        let timestamp = now_micros();
        let frame_index = (sequence - 1) % MOCK_SCENE_CYCLE_FRAMES;
        let local_gate = local_talker_gain(frame_index);
        let peer_gate = peer_talker_gain(frame_index);

        let peer_samples = self.peer_voice.render_frame(self.sample_rate, peer_gate);
        let peer_frame =
            AudioFrame::new(sequence, timestamp, self.sample_rate as u32, peer_samples);

        self.peer_history.push_back(peer_frame.clone());
        while self.peer_history.len() > self.peer_delay_frames + 3 {
            self.peer_history.pop_front();
        }

        let delayed_peer = self
            .peer_history
            .iter()
            .rev()
            .nth(self.peer_delay_frames)
            .map(|frame| frame.samples.clone())
            .unwrap_or_else(|| vec![0.0; SAMPLES_PER_FRAME]);

        let local_near = self.local_voice.render_frame(self.sample_rate, local_gate);
        let ambience_step = TAU * 31.0 / self.sample_rate.max(1.0);
        let mut local_samples = Vec::with_capacity(SAMPLES_PER_FRAME);
        for index in 0..SAMPLES_PER_FRAME {
            let ambience = (self.ambience_phase + ambience_step * index as f32).sin() * 0.0015;
            let sample = local_near[index] + delayed_peer[index] * self.peer_gain + ambience;
            local_samples.push(sample);
        }
        self.ambience_phase += ambience_step * SAMPLES_PER_FRAME as f32;

        self.pending_peer_frames.push_back(peer_frame);
        AudioFrame::new(sequence, timestamp, self.sample_rate as u32, local_samples)
    }

    fn take_peer_frame(&mut self) -> Option<AudioFrame> {
        self.pending_peer_frames.pop_front()
    }
}

struct VoiceOscillator {
    carrier_hz: f32,
    phase: f32,
    shimmer_phase: f32,
    tremolo_phase: f32,
}

impl VoiceOscillator {
    fn new(carrier_hz: f32) -> Self {
        Self {
            carrier_hz,
            phase: 0.0,
            shimmer_phase: 0.0,
            tremolo_phase: 0.0,
        }
    }

    fn render_frame(&mut self, sample_rate: f32, gate: f32) -> Vec<f32> {
        let carrier_step = TAU * self.carrier_hz / sample_rate.max(1.0);
        let shimmer_step = TAU * 2.1 / sample_rate.max(1.0);
        let tremolo_step = TAU * 1.6 / sample_rate.max(1.0);
        let mut samples = Vec::with_capacity(SAMPLES_PER_FRAME);

        for _ in 0..SAMPLES_PER_FRAME {
            let articulation = 0.7 + 0.3 * self.tremolo_phase.sin().abs();
            let shimmer = self.shimmer_phase.sin() * 0.28;
            let sample = gate
                * articulation
                * ((self.phase).sin() * 0.12
                    + (self.phase * 2.0 + shimmer).sin() * 0.045
                    + (self.phase * 3.0 - shimmer * 0.5).sin() * 0.02);
            samples.push(sample);

            self.phase += carrier_step;
            self.shimmer_phase += shimmer_step;
            self.tremolo_phase += tremolo_step;
        }

        samples
    }
}

fn local_talker_gain(frame_index: u64) -> f32 {
    segment_gain(frame_index, 90, 170)
}

fn peer_talker_gain(frame_index: u64) -> f32 {
    segment_gain(frame_index, 20, 90).max(segment_gain(frame_index, 130, 200))
}

fn segment_gain(frame_index: u64, start: u64, end: u64) -> f32 {
    if frame_index < start || frame_index >= end {
        return 0.0;
    }

    let fade_in_end = start + MOCK_SCENE_RAMP_FRAMES;
    if frame_index < fade_in_end {
        return (frame_index - start + 1) as f32 / MOCK_SCENE_RAMP_FRAMES as f32;
    }

    let fade_out_start = end.saturating_sub(MOCK_SCENE_RAMP_FRAMES);
    if frame_index >= fade_out_start {
        return (end - frame_index) as f32 / MOCK_SCENE_RAMP_FRAMES as f32;
    }

    1.0
}

struct DebugRecorder {
    capture_raw: Option<WavWriterSink>,
    local_raw: Option<WavWriterSink>,
    peer_raw: Option<WavWriterSink>,
    peer_aligned: Option<WavWriterSink>,
    output: Option<WavWriterSink>,
    metrics_writer: Option<BufWriter<File>>,
}

impl DebugRecorder {
    fn new(config: &NodeConfig) -> Result<Self> {
        if config.debug.dump_wav || config.debug.dump_metrics {
            fs::create_dir_all(&config.debug.dump_dir).with_context(|| {
                format!(
                    "failed to create debug directory {}",
                    config.debug.dump_dir.display()
                )
            })?;
        }

        let capture_raw = if config.debug.dump_wav {
            Some(WavWriterSink::create(default_debug_wav_path(
                &config.debug.dump_dir,
                "capture_raw",
            ))?)
        } else {
            None
        };
        let local_raw = if config.debug.dump_wav {
            Some(WavWriterSink::create(default_debug_wav_path(
                &config.debug.dump_dir,
                "local_raw",
            ))?)
        } else {
            None
        };
        let peer_raw = if config.debug.dump_wav {
            Some(WavWriterSink::create(default_debug_wav_path(
                &config.debug.dump_dir,
                "peer_raw",
            ))?)
        } else {
            None
        };
        let peer_aligned = if config.debug.dump_wav {
            Some(WavWriterSink::create(default_debug_wav_path(
                &config.debug.dump_dir,
                "peer_aligned",
            ))?)
        } else {
            None
        };
        let output = if config.debug.dump_wav {
            Some(WavWriterSink::create(default_debug_wav_path(
                &config.debug.dump_dir,
                "output",
            ))?)
        } else {
            None
        };

        let metrics_writer = if config.debug.dump_metrics {
            let path: PathBuf = config.debug.dump_dir.join("metrics.tsv");
            let mut writer =
                BufWriter::new(File::create(&path).with_context(|| {
                    format!("failed to create metrics file {}", path.display())
                })?);
            writeln!(
                writer,
                "sequence\tcoarse_delay_ms\tdrift_ppm\tcoherence\tlocal_vad\tpeer_vad\tfrozen\tloss_rate\tinput_rms\toutput_rms\testimated_crosstalk_rms\tclip_events\tprocessing_time_us"
            )
            .context("failed to write metrics header")?;
            Some(writer)
        } else {
            None
        };

        Ok(Self {
            capture_raw,
            local_raw,
            peer_raw,
            peer_aligned,
            output,
            metrics_writer,
        })
    }

    fn record(
        &mut self,
        capture_raw: &AudioFrame,
        local_raw: &AudioFrame,
        peer_raw: &AudioFrame,
        peer_aligned: &AudioFrame,
        output: &AudioFrame,
        snapshot: &RuntimeSnapshot,
    ) -> Result<()> {
        if let Some(writer) = self.capture_raw.as_mut() {
            writer.write_frame(capture_raw)?;
        }
        if let Some(writer) = self.local_raw.as_mut() {
            writer.write_frame(local_raw)?;
        }
        if let Some(writer) = self.peer_raw.as_mut() {
            writer.write_frame(peer_raw)?;
        }
        if let Some(writer) = self.peer_aligned.as_mut() {
            writer.write_frame(peer_aligned)?;
        }
        if let Some(writer) = self.output.as_mut() {
            writer.write_frame(output)?;
        }
        if let Some(writer) = self.metrics_writer.as_mut() {
            writeln!(
                writer,
                "{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}\t{:.4}\t{:.5}\t{:.5}\t{:.5}\t{}\t{}",
                snapshot.sequence,
                snapshot.coarse_delay_ms,
                snapshot.drift_ppm,
                snapshot.coherence,
                snapshot.local_vad.score,
                snapshot.peer_vad.score,
                snapshot.update_frozen,
                snapshot.transport_loss_rate,
                snapshot.input_rms,
                snapshot.output_rms,
                snapshot.estimated_crosstalk_rms,
                snapshot.clip_events,
                snapshot.processing_time_us,
            )
            .context("failed to append metrics row")?;
        }

        Ok(())
    }

    fn finalize(&mut self) -> Result<()> {
        if let Some(writer) = self.capture_raw.as_mut() {
            writer.finalize()?;
        }
        if let Some(writer) = self.local_raw.as_mut() {
            writer.finalize()?;
        }
        if let Some(writer) = self.peer_raw.as_mut() {
            writer.finalize()?;
        }
        if let Some(writer) = self.peer_aligned.as_mut() {
            writer.finalize()?;
        }
        if let Some(writer) = self.output.as_mut() {
            writer.finalize()?;
        }
        if let Some(writer) = self.metrics_writer.as_mut() {
            writer.flush().context("failed to flush metrics file")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_types::{OutputBackend, TransportBackend};

    #[test]
    fn integrated_mock_scene_updates_and_reduces_peer_only_energy() {
        let mut config = NodeConfig::default();
        config.node.transport_backend = TransportBackend::Mock;
        config.audio.backend = AudioBackend::Mock;
        config.output.backend = OutputBackend::Null;
        config.debug.dump_wav = false;
        config.debug.dump_metrics = false;
        config.debug.mock_peer_delay_ms = 20;
        config.debug.mock_peer_gain = 0.35;

        let mut runtime = PipelineRuntime::new(config).expect("mock runtime should build");
        let mut snapshots = Vec::new();

        for _ in 0..180 {
            snapshots.push(runtime.step().expect("mock step should succeed"));
        }

        runtime.shutdown().expect("mock shutdown should succeed");

        assert!(
            snapshots.iter().any(|snapshot| !snapshot.update_frozen),
            "mock scene never opened an adaptive-update window"
        );
        assert!(
            snapshots
                .iter()
                .skip(40)
                .any(|snapshot| snapshot.coarse_delay_ms >= 20.0),
            "mock scene never surfaced the configured peer delay"
        );

        let strong_reduction_frames = snapshots
            .iter()
            .skip(60)
            .filter(|snapshot| {
                !snapshot.update_frozen
                    && snapshot.coherence > 0.95
                    && snapshot.output_rms < snapshot.input_rms * 0.65
            })
            .count();

        assert!(
            strong_reduction_frames >= 10,
            "expected repeated peer-only attenuation after convergence, got {strong_reduction_frames} frames"
        );
    }

    #[test]
    fn capture_conditioner_reduces_clipped_samples() {
        let mut conditioner = CaptureConditioner::default();
        let frame = AudioFrame::new(1, now_micros(), 48_000, vec![2.0; SAMPLES_PER_FRAME]);
        let conditioned = conditioner.process(frame);
        let peak = conditioned
            .samples
            .iter()
            .fold(0.0_f32, |current, sample| current.max(sample.abs()));
        assert!(
            peak < 0.99,
            "expected conditioner to reduce peak, got {peak}"
        );
    }

    #[test]
    fn near_end_dominance_allows_peer_only_leakage_updates() {
        let local = AudioFrame::new(1, now_micros(), 48_000, vec![0.08; SAMPLES_PER_FRAME]);
        let peer = AudioFrame::new(1, now_micros(), 48_000, vec![0.09; SAMPLES_PER_FRAME]);

        assert!(
            !near_end_dominant(
                &local,
                &peer,
                VadDecision {
                    score: 0.7,
                    is_speech: true,
                },
                VadDecision {
                    score: 0.9,
                    is_speech: true,
                },
            ),
            "peer-dominant leakage should still allow adaptive updates"
        );
    }
}
