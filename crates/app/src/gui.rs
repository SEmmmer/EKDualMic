use crate::{PipelineRuntime, discover_config_presets, init_logging, load_config, save_config};
use anyhow::Error;
#[cfg(windows)]
use audio_capture::list_capture_devices;
#[cfg(windows)]
use audio_output::list_render_devices;
use common_types::{
    AudioBackend, AudioDeviceInfo, OutputBackend, RuntimeSnapshot, TransportBackend,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

const WORKER_RECOVERY_DELAY: Duration = Duration::from_millis(750);
const WORKER_SLEEP_SLICE: Duration = Duration::from_millis(50);
const METRIC_HISTORY_LIMIT: usize = 240;

pub fn run_native() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1120.0, 760.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EK Dual Mic",
        options,
        Box::new(|cc| {
            install_windows_cjk_font_fallback(&cc.egui_ctx);
            Ok(Box::new(NodeGuiApp::default()))
        }),
    )
}

struct WorkerHandle {
    stop: Arc<AtomicBool>,
    control_tx: Sender<WorkerCommand>,
    rx: Receiver<WorkerEvent>,
    join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    fn request_reload(&self, config_path: String) -> bool {
        self.control_tx
            .send(WorkerCommand::Reload { config_path })
            .is_ok()
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

enum WorkerCommand {
    Reload { config_path: String },
}

enum WorkerEvent {
    Snapshot(RuntimeSnapshot),
    Recovering(String),
    Stopped,
}

pub struct NodeGuiApp {
    config_path: String,
    config_presets: Vec<String>,
    status: String,
    latest: Option<RuntimeSnapshot>,
    metric_history: VecDeque<RuntimeSnapshot>,
    worker: Option<WorkerHandle>,
    capture_devices: Vec<AudioDeviceInfo>,
    render_devices: Vec<AudioDeviceInfo>,
    input_device_value: String,
    target_device_value: String,
    config_feedback: Option<String>,
    config_discovery_error: Option<String>,
    config_mode_summary: Option<String>,
    config_mode_warning: Option<String>,
    loaded_audio_backend: Option<AudioBackend>,
    loaded_transport_backend: Option<TransportBackend>,
    loaded_output_backend: Option<OutputBackend>,
    device_probe_error: Option<String>,
}

impl Default for NodeGuiApp {
    fn default() -> Self {
        let mut app = Self {
            config_path: "configs/node-a.toml".to_owned(),
            config_presets: Vec::new(),
            status: "Idle".to_owned(),
            latest: None,
            metric_history: VecDeque::with_capacity(METRIC_HISTORY_LIMIT),
            worker: None,
            capture_devices: Vec::new(),
            render_devices: Vec::new(),
            input_device_value: String::new(),
            target_device_value: String::new(),
            config_feedback: None,
            config_discovery_error: None,
            config_mode_summary: None,
            config_mode_warning: None,
            loaded_audio_backend: None,
            loaded_transport_backend: None,
            loaded_output_backend: None,
            device_probe_error: None,
        };
        app.refresh_config_presets();
        app.refresh_device_lists();
        app.reload_config_fields();
        app
    }
}

impl NodeGuiApp {
    fn normalized_device_field(value: &str) -> String {
        value.trim().to_owned()
    }

    fn sync_loaded_config_metadata(&mut self, config: &common_types::NodeConfig) {
        self.loaded_audio_backend = Some(config.audio.backend);
        self.loaded_transport_backend = Some(config.node.transport_backend);
        self.loaded_output_backend = Some(config.output.backend);
        info!(
            node = %config.node.name,
            audio = backend_label_audio(config.audio.backend),
            transport = backend_label_transport(config.node.transport_backend),
            output = backend_label_output(config.output.backend),
            "GUI loaded config metadata"
        );
        self.config_mode_summary = Some(format!(
            "node={}, audio={}, transport={}, output={}",
            config.node.name,
            backend_label_audio(config.audio.backend),
            backend_label_transport(config.node.transport_backend),
            backend_label_output(config.output.backend),
        ));
        self.config_mode_warning = if config.audio.backend == AudioBackend::Mock {
            Some(
                "Current config uses mock audio input. Live microphone signal is ignored."
                    .to_owned(),
            )
        } else if config.output.backend != OutputBackend::VirtualStub {
            Some("Current config does not write to a live output device. `target_device` is ignored in this mode.".to_owned())
        } else {
            None
        };
    }

    fn capture_selection_enabled(&self) -> bool {
        self.loaded_audio_backend != Some(AudioBackend::Mock)
    }

    fn render_selection_enabled(&self) -> bool {
        !matches!(
            self.loaded_output_backend,
            Some(OutputBackend::Null | OutputBackend::WavDump)
        )
    }

    fn record_snapshot(&mut self, snapshot: RuntimeSnapshot) {
        self.metric_history.push_back(snapshot.clone());
        while self.metric_history.len() > METRIC_HISTORY_LIMIT {
            self.metric_history.pop_front();
        }
        self.latest = Some(snapshot);
    }

    fn refresh_config_presets(&mut self) {
        match discover_config_presets() {
            Ok(mut presets) => {
                if !self.config_path.is_empty()
                    && !presets.iter().any(|preset| preset == &self.config_path)
                {
                    presets.insert(0, self.config_path.clone());
                }
                self.config_presets = presets;
                self.config_discovery_error = None;
            }
            Err(error) => {
                self.config_presets = vec![self.config_path.clone()];
                self.config_discovery_error = Some(error.to_string());
            }
        }
    }

    fn refresh_device_lists(&mut self) {
        #[cfg(windows)]
        {
            self.capture_devices.clear();
            self.render_devices.clear();
            self.device_probe_error = None;

            match list_capture_devices() {
                Ok(devices) => {
                    self.capture_devices = devices;
                }
                Err(error) => {
                    self.device_probe_error = Some(error.to_string());
                }
            }

            match list_render_devices() {
                Ok(devices) => {
                    self.render_devices = devices;
                }
                Err(error) => {
                    let message = error.to_string();
                    self.device_probe_error = Some(match self.device_probe_error.take() {
                        Some(existing) => format!("{existing}; {message}"),
                        None => message,
                    });
                }
            }
        }
    }

    fn reload_config_fields(&mut self) {
        match load_config(&self.config_path) {
            Ok(config) => {
                self.sync_loaded_config_metadata(&config);
                self.input_device_value = config.audio.input_device;
                self.target_device_value = config.output.target_device;
                info!(config_path = %self.config_path, "GUI loaded config into form");
                self.config_feedback = Some(format!("Config loaded from {}", self.config_path));
                self.status = format!("Config loaded: {}", self.config_path);
                if self.request_runtime_reload() {
                    self.config_feedback = Some(format!(
                        "Config loaded from {}; runtime reload requested",
                        self.config_path
                    ));
                }
            }
            Err(error) => {
                let details = format_error_chain(&error);
                warn!(config_path = %self.config_path, error = %details, "GUI failed to load config into form");
                self.config_feedback = Some(format!("Config load failed: {details}"));
                self.status = format!("Config load failed: {details}");
            }
        }
    }

    fn persist_ui_device_fields_to_config(&mut self) -> Result<bool, ()> {
        match load_config(&self.config_path) {
            Ok(mut config) => {
                let input_device = Self::normalized_device_field(&self.input_device_value);
                let target_device = Self::normalized_device_field(&self.target_device_value);
                let changed = config.audio.input_device != input_device
                    || config.output.target_device != target_device;

                config.audio.input_device = input_device;
                config.output.target_device = target_device;

                match save_config(&self.config_path, &config) {
                    Ok(()) => {
                        self.sync_loaded_config_metadata(&config);
                        info!(config_path = %self.config_path, "GUI saved device fields into config");
                        self.config_feedback = None;
                        return Ok(changed);
                    }
                    Err(error) => {
                        let details = format_error_chain(&error);
                        warn!(config_path = %self.config_path, error = %details, "GUI failed to save device fields");
                        self.config_feedback = Some(format!("Config save failed: {details}"));
                        self.status = format!("Config save failed: {details}");
                    }
                }
            }
            Err(error) => {
                let details = format_error_chain(&error);
                warn!(config_path = %self.config_path, error = %details, "GUI could not load config before saving device fields");
                self.config_feedback = Some(format!("Config load failed: {details}"));
                self.status = format!("Config load failed: {details}");
            }
        }

        Err(())
    }

    fn save_device_fields(&mut self) {
        let changed = match self.persist_ui_device_fields_to_config() {
            Ok(changed) => changed,
            Err(()) => return,
        };

        if self.request_runtime_reload() {
            self.config_feedback = Some(if changed {
                "Device fields saved; runtime reload requested".to_owned()
            } else {
                "Device fields unchanged; runtime reload requested".to_owned()
            });
        } else {
            self.config_feedback = Some(if changed {
                "Device fields saved".to_owned()
            } else {
                "Device fields already matched the config".to_owned()
            });
            self.status = format!("Config ready: {}", self.config_path);
        }
    }

    fn load_selected_config_path(&mut self, config_path: String) {
        self.config_path = config_path;
        self.reload_config_fields();
    }

    fn request_runtime_reload(&mut self) -> bool {
        let Some(worker) = self.worker.as_ref() else {
            return false;
        };

        if worker.request_reload(self.config_path.clone()) {
            info!(config_path = %self.config_path, "GUI requested runtime reload");
            self.status = "Reload requested".to_owned();
            true
        } else {
            warn!(config_path = %self.config_path, "GUI failed to queue runtime reload request");
            self.status = "Reload request failed".to_owned();
            false
        }
    }

    fn start(&mut self) {
        if self.worker.is_some() {
            return;
        }

        let changed = match self.persist_ui_device_fields_to_config() {
            Ok(changed) => changed,
            Err(()) => return,
        };

        let config_path = self.config_path.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let (control_tx, control_rx) = unbounded();
        let (tx, rx) = unbounded();
        let join = thread::spawn(move || run_worker(config_path, stop_worker, control_rx, tx));

        info!(config_path = %self.config_path, "GUI requested runtime start");
        self.latest = None;
        self.metric_history.clear();
        self.status = format!("Starting: {}", self.config_path);
        self.config_feedback = Some(if changed {
            "Runtime will use the current device fields from this config".to_owned()
        } else {
            "Runtime will use the devices already stored in this config".to_owned()
        });
        self.worker = Some(WorkerHandle {
            stop,
            control_tx,
            rx,
            join: Some(join),
        });
    }

    fn stop(&mut self) {
        if let Some(worker) = self.worker.as_mut() {
            worker.stop();
        }
        self.worker = None;
        info!("GUI requested runtime stop");
        self.status = "Stopped".to_owned();
    }

    fn poll_worker(&mut self) {
        let mut should_clear_worker = false;

        if let Some(worker) = self.worker.as_ref() {
            let events: Vec<_> = worker.rx.try_iter().collect();
            for event in events {
                match event {
                    WorkerEvent::Snapshot(snapshot) => {
                        self.status = format!("Running: frame {}", snapshot.sequence);
                        self.record_snapshot(snapshot);
                    }
                    WorkerEvent::Recovering(message) => {
                        warn!(message = %message, "GUI worker entered recovering state");
                        self.status = format!("Recovering: {message}");
                    }
                    WorkerEvent::Stopped => {
                        info!("GUI worker stopped");
                        should_clear_worker = true;
                    }
                }
            }
        }

        if should_clear_worker {
            if let Some(mut worker) = self.worker.take() {
                worker.stop();
            }
        }
    }
}

impl Drop for NodeGuiApp {
    fn drop(&mut self) {
        self.stop();
    }
}

impl NodeGuiApp {
    fn draw_metrics_dashboard(&self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        ui.columns(4, |columns| {
            draw_stat_card(
                &mut columns[0],
                "Node",
                &snapshot.node_name,
                "active config",
            );
            draw_stat_card(
                &mut columns[1],
                "Sequence",
                &snapshot.sequence.to_string(),
                "latest frame",
            );
            draw_stat_card(
                &mut columns[2],
                "Delay",
                &format!("{:.1} ms", snapshot.coarse_delay_ms),
                "coarse sync offset",
            );
            draw_stat_card(
                &mut columns[3],
                "Frame Time",
                &format!("{} us", snapshot.processing_time_us),
                "processing cost",
            );
        });

        ui.add_space(8.0);
        ui.columns(2, |columns| {
            self.draw_audio_level_panel(&mut columns[0], snapshot);
            self.draw_sync_quality_panel(&mut columns[1], snapshot);
        });

        ui.add_space(8.0);
        ui.columns(2, |columns| {
            self.draw_transport_panel(&mut columns[0], snapshot);
            self.draw_timing_panel(&mut columns[1], snapshot);
        });
    }

    fn draw_audio_level_panel(&self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        let input_history = self.metric_history_values(|entry| entry.input_rms);
        let output_history = self.metric_history_values(|entry| entry.output_rms);
        let crosstalk_history = self.metric_history_values(|entry| entry.estimated_crosstalk_rms);
        let level_max =
            max_series_value([&input_history, &output_history, &crosstalk_history], 0.05);

        draw_history_card(
            ui,
            "Audio Levels",
            &[
                HistoryLine {
                    label: "Input RMS",
                    color: egui::Color32::from_rgb(82, 196, 26),
                    values: &input_history,
                },
                HistoryLine {
                    label: "Output RMS",
                    color: egui::Color32::from_rgb(250, 173, 20),
                    values: &output_history,
                },
                HistoryLine {
                    label: "Crosstalk",
                    color: egui::Color32::from_rgb(255, 120, 117),
                    values: &crosstalk_history,
                },
            ],
            0.0,
            level_max,
            "RMS",
        );

        let attenuation = attenuation_ratio(snapshot.input_rms, snapshot.output_rms);
        draw_progress_metric(
            ui,
            "Input RMS",
            snapshot.input_rms / level_max.max(f32::EPSILON),
            format!("{:.5}", snapshot.input_rms),
            egui::Color32::from_rgb(82, 196, 26),
        );
        draw_progress_metric(
            ui,
            "Output RMS",
            snapshot.output_rms / level_max.max(f32::EPSILON),
            format!("{:.5}", snapshot.output_rms),
            egui::Color32::from_rgb(250, 173, 20),
        );
        draw_progress_metric(
            ui,
            "Attenuation",
            attenuation,
            format!("{:.1}%", attenuation * 100.0),
            egui::Color32::from_rgb(64, 169, 255),
        );
    }

    fn draw_sync_quality_panel(&self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        let coherence_history = self.metric_history_values(|entry| entry.coherence);
        let local_vad_history = self.metric_history_values(|entry| entry.local_vad.score);
        let peer_vad_history = self.metric_history_values(|entry| entry.peer_vad.score);

        draw_history_card(
            ui,
            "Sync And Voice Activity",
            &[
                HistoryLine {
                    label: "Coherence",
                    color: egui::Color32::from_rgb(64, 169, 255),
                    values: &coherence_history,
                },
                HistoryLine {
                    label: "Local VAD",
                    color: egui::Color32::from_rgb(149, 117, 205),
                    values: &local_vad_history,
                },
                HistoryLine {
                    label: "Peer VAD",
                    color: egui::Color32::from_rgb(255, 120, 117),
                    values: &peer_vad_history,
                },
            ],
            0.0,
            1.0,
            "score",
        );

        draw_progress_metric(
            ui,
            "Coherence",
            snapshot.coherence,
            format!("{:.3}", snapshot.coherence),
            egui::Color32::from_rgb(64, 169, 255),
        );
        draw_progress_metric(
            ui,
            "Local VAD",
            snapshot.local_vad.score,
            format!("{:.3}", snapshot.local_vad.score),
            egui::Color32::from_rgb(149, 117, 205),
        );
        draw_progress_metric(
            ui,
            "Peer VAD",
            snapshot.peer_vad.score,
            format!("{:.3}", snapshot.peer_vad.score),
            egui::Color32::from_rgb(255, 120, 117),
        );

        ui.horizontal_wrapped(|ui| {
            ui.label("Update State");
            if snapshot.update_frozen {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 117), "Frozen");
            } else {
                ui.colored_label(egui::Color32::from_rgb(82, 196, 26), "Adaptive");
            }
        });
    }

    fn draw_transport_panel(&self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        let loss_history = self.metric_history_values(|entry| entry.transport_loss_rate);
        let clip_history = self.metric_history_values(|entry| entry.clip_events as f32);
        let clip_max = max_series_value([&clip_history], 1.0);

        draw_history_card(
            ui,
            "Transport Health",
            &[HistoryLine {
                label: "Loss Rate",
                color: egui::Color32::from_rgb(255, 120, 117),
                values: &loss_history,
            }],
            0.0,
            1.0,
            "ratio",
        );

        draw_progress_metric(
            ui,
            "Transport Loss",
            snapshot.transport_loss_rate,
            format!("{:.2}%", snapshot.transport_loss_rate * 100.0),
            egui::Color32::from_rgb(255, 120, 117),
        );
        draw_progress_metric(
            ui,
            "Clip Events",
            (snapshot.clip_events as f32 / clip_max.max(1.0)).clamp(0.0, 1.0),
            snapshot.clip_events.to_string(),
            egui::Color32::from_rgb(250, 173, 20),
        );

        ui.separator();
        egui::Grid::new("transport_counts_grid")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Sent");
                ui.monospace(snapshot.sent_packets.to_string());
                ui.end_row();
                ui.label("Received");
                ui.monospace(snapshot.received_packets.to_string());
                ui.end_row();
                ui.label("Concealed");
                ui.monospace(snapshot.concealed_packets.to_string());
                ui.end_row();
            });
    }

    fn draw_timing_panel(&self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        let delay_history = self.metric_history_values(|entry| entry.coarse_delay_ms);
        let frame_time_history =
            self.metric_history_values(|entry| entry.processing_time_us as f32);
        let max_delay = max_series_value([&delay_history], 20.0);
        let max_frame_time = max_series_value([&frame_time_history], 2_000.0);

        draw_history_card(
            ui,
            "Delay History",
            &[HistoryLine {
                label: "Delay ms",
                color: egui::Color32::from_rgb(64, 169, 255),
                values: &delay_history,
            }],
            0.0,
            max_delay,
            "ms",
        );

        draw_progress_metric(
            ui,
            "Coarse Delay",
            snapshot.coarse_delay_ms / max_delay.max(f32::EPSILON),
            format!("{:.2} ms", snapshot.coarse_delay_ms),
            egui::Color32::from_rgb(64, 169, 255),
        );
        draw_progress_metric(
            ui,
            "Processing Time",
            snapshot.processing_time_us as f32 / max_frame_time.max(f32::EPSILON),
            format!("{} us", snapshot.processing_time_us),
            egui::Color32::from_rgb(82, 196, 26),
        );
        draw_history_card(
            ui,
            "Processing Cost",
            &[HistoryLine {
                label: "Frame us",
                color: egui::Color32::from_rgb(82, 196, 26),
                values: &frame_time_history,
            }],
            0.0,
            max_frame_time,
            "us",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Drift");
            ui.monospace(format!("{:.2} ppm", snapshot.drift_ppm));
        });
    }

    fn metric_history_values(&self, map: impl Fn(&RuntimeSnapshot) -> f32) -> Vec<f32> {
        self.metric_history.iter().map(map).collect()
    }
}

impl eframe::App for NodeGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();
        ctx.request_repaint_after(Duration::from_millis(33));

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("EK Dual Mic");
                ui.label("Windows-only realtime scaffold");
            });
        });

        egui::SidePanel::left("control_panel")
            .min_width(320.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("control_panel_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label("Config Path");
                        ui.text_edit_singleline(&mut self.config_path);
                        ui.horizontal(|ui| {
                            ui.menu_button("Load Config", |ui| {
                                let presets = self.config_presets.clone();
                                if presets.is_empty() {
                                    ui.small("No config presets found.");
                                } else {
                                    for preset in presets {
                                        if ui.button(&preset).clicked() {
                                            self.load_selected_config_path(preset);
                                            ui.close();
                                        }
                                    }
                                }

                                ui.separator();
                                if ui.button("Load Current Path").clicked() {
                                    self.reload_config_fields();
                                    ui.close();
                                }
                                if ui.button("Refresh Config List").clicked() {
                                    self.refresh_config_presets();
                                }
                            });
                            if ui.button("Refresh Configs").clicked() {
                                self.refresh_config_presets();
                            }
                            if ui.button("Load Current Path").clicked() {
                                self.reload_config_fields();
                            }
                            if ui.button("Save Device Fields").clicked() {
                                self.save_device_fields();
                            }
                        });
                        if let Some(error) = &self.config_discovery_error {
                            ui.small(format!("Config discovery error: {error}"));
                        } else if !self.config_presets.is_empty() {
                            ui.small(format!(
                                "Known configs: {}",
                                self.config_presets.join(", ")
                            ));
                        }
                        if let Some(summary) = &self.config_mode_summary {
                            ui.small(format!("Loaded mode: {summary}"));
                        }
                        if let Some(warning) = &self.config_mode_warning {
                            ui.colored_label(egui::Color32::from_rgb(250, 173, 20), warning);
                        }
                        ui.separator();

                        ui.label("Audio Input Device");
                        ui.add_enabled_ui(self.capture_selection_enabled(), |ui| {
                            ui.text_edit_singleline(&mut self.input_device_value);
                        });
                        if !self.capture_selection_enabled() {
                            ui.small("Ignored because this config uses the mock audio backend.");
                        }

                        ui.label("Output Target Device");
                        ui.add_enabled_ui(self.render_selection_enabled(), |ui| {
                            ui.text_edit_singleline(&mut self.target_device_value);
                        });
                        if !self.render_selection_enabled() {
                            ui.small("Ignored because this config writes to WAV/null instead of a live output endpoint.");
                        }

                        ui.horizontal(|ui| {
                            if ui.button("Use Default Capture").clicked() {
                                self.input_device_value = "default".to_owned();
                            }
                            if ui.button("Use Default Render").clicked() {
                                self.target_device_value = "default".to_owned();
                            }
                        });
                        if let Some(message) = &self.config_feedback {
                            ui.small(message);
                        }
                        ui.separator();

                        if self.worker.is_none() {
                            if ui.button("Start").clicked() {
                                self.start();
                            }
                        } else {
                            ui.horizontal(|ui| {
                                if ui.button("Reload Runtime").clicked() {
                                    self.request_runtime_reload();
                                }
                                if ui.button("Stop").clicked() {
                                    self.stop();
                                }
                            });
                        }

                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Refresh Configs").clicked() {
                                self.refresh_config_presets();
                            }
                            if ui.button("Refresh Devices").clicked() {
                                self.refresh_device_lists();
                            }
                        });
                        ui.label(format!("Status: {}", self.status));
                        ui.small(
                            "WASAPI capture and render-endpoint bridge are available. Built-in virtual mic device creation is still not implemented.",
                        );

                        ui.separator();
                        ui.collapsing("Capture Devices", |ui| {
                            if self.capture_devices.is_empty() {
                                ui.small("No capture devices loaded.");
                            } else {
                                for device in &self.capture_devices {
                                    let selected = self.input_device_value == device.name;
                                    let label = format_device_label(device);
                                    if ui
                                        .add_enabled(
                                            self.capture_selection_enabled(),
                                            egui::Button::selectable(selected, &label),
                                        )
                                        .clicked()
                                    {
                                        self.input_device_value = device.name.clone();
                                    }
                                    ui.small(format!("id: {}", device.id));
                                }
                            }
                        });
                        ui.collapsing("Render Devices", |ui| {
                            if self.render_devices.is_empty() {
                                ui.small("No render devices loaded.");
                            } else {
                                for device in &self.render_devices {
                                    let selected = self.target_device_value == device.name;
                                    let label = format_device_label(device);
                                    if ui
                                        .add_enabled(
                                            self.render_selection_enabled(),
                                            egui::Button::selectable(selected, &label),
                                        )
                                        .clicked()
                                    {
                                        self.target_device_value = device.name.clone();
                                    }
                                    ui.small(format!("id: {}", device.id));
                                }
                            }
                        });
                        if let Some(error) = &self.device_probe_error {
                            ui.separator();
                            ui.small(format!("Device probe error: {error}"));
                        }
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("metrics_panel_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.heading("Realtime Metrics");
                    ui.separator();

                    if let Some(snapshot) = &self.latest {
                        self.draw_metrics_dashboard(ui, snapshot);
                    } else {
                        ui.label("No runtime snapshot yet.");
                    }
                });
        });
    }
}

struct HistoryLine<'a> {
    label: &'a str,
    color: egui::Color32,
    values: &'a [f32],
}

fn format_device_label(device: &AudioDeviceInfo) -> String {
    if device.is_default {
        format!("{} [default]", device.name)
    } else {
        device.name.clone()
    }
}

fn backend_label_audio(backend: AudioBackend) -> &'static str {
    match backend {
        AudioBackend::Wasapi => "wasapi",
        AudioBackend::Mock => "mock",
    }
}

fn backend_label_transport(backend: TransportBackend) -> &'static str {
    match backend {
        TransportBackend::Udp => "udp",
        TransportBackend::Mock => "mock",
    }
}

fn backend_label_output(backend: OutputBackend) -> &'static str {
    match backend {
        OutputBackend::VirtualStub => "virtual_stub",
        OutputBackend::WavDump => "wav_dump",
        OutputBackend::Null => "null",
    }
}

fn draw_stat_card(ui: &mut egui::Ui, title: &str, value: &str, subtitle: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_height(86.0);
        ui.label(title);
        ui.heading(value);
        ui.small(subtitle);
    });
}

fn draw_progress_metric(
    ui: &mut egui::Ui,
    label: &str,
    progress: f32,
    value_text: String,
    fill: egui::Color32,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(value_text);
        });
    });
    ui.add_sized(
        [safe_available_width(ui, 120.0), 18.0],
        egui::ProgressBar::new(progress.clamp(0.0, 1.0)).fill(fill),
    );
    ui.add_space(4.0);
}

fn draw_history_card(
    ui: &mut egui::Ui,
    title: &str,
    lines: &[HistoryLine<'_>],
    min_y: f32,
    max_y: f32,
    unit: &str,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(title);
        ui.small(format!(
            "Recent {METRIC_HISTORY_LIMIT} frames, unit: {unit}"
        ));
        ui.horizontal_wrapped(|ui| {
            for line in lines {
                ui.colored_label(line.color, line.label);
            }
        });

        let desired_size = egui::vec2(safe_available_width(ui, 120.0), 150.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        paint_history_chart(ui.painter_at(rect), rect, lines, min_y, max_y);
    });
}

fn paint_history_chart(
    painter: egui::Painter,
    rect: egui::Rect,
    lines: &[HistoryLine<'_>],
    min_y: f32,
    max_y: f32,
) {
    let bg = painter.ctx().style().visuals.extreme_bg_color;
    let grid = painter
        .ctx()
        .style()
        .visuals
        .widgets
        .noninteractive
        .bg_stroke
        .color;
    painter.rect_filled(rect, 8.0, bg);
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0, grid),
        egui::StrokeKind::Outside,
    );

    let mid_y = rect.top() + rect.height() * 0.5;
    painter.line_segment(
        [
            egui::pos2(rect.left(), mid_y),
            egui::pos2(rect.right(), mid_y),
        ],
        egui::Stroke::new(1.0, grid.gamma_multiply(0.6)),
    );
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        egui::Stroke::new(1.0, grid.gamma_multiply(0.6)),
    );

    let span = (max_y - min_y).abs().max(1e-6);
    for line in lines {
        if line.values.is_empty() {
            continue;
        }

        if line.values.len() == 1 {
            let y = map_to_chart_y(line.values[0], min_y, span, rect);
            painter.circle_filled(egui::pos2(rect.center().x, y), 3.0, line.color);
            continue;
        }

        let mut points = Vec::with_capacity(line.values.len());
        let last_index = (line.values.len() - 1) as f32;
        for (index, value) in line.values.iter().enumerate() {
            let t = index as f32 / last_index.max(1.0);
            let x = egui::lerp(rect.left()..=rect.right(), t);
            let y = map_to_chart_y(*value, min_y, span, rect);
            points.push(egui::pos2(x, y));
        }

        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(2.0, line.color),
        ));
    }
}

fn map_to_chart_y(value: f32, min_y: f32, span: f32, rect: egui::Rect) -> f32 {
    let normalized = ((value - min_y) / span).clamp(0.0, 1.0);
    egui::lerp(rect.bottom()..=rect.top(), normalized)
}

fn max_series_value<'a>(series: impl IntoIterator<Item = &'a Vec<f32>>, fallback: f32) -> f32 {
    let mut max_value = fallback;
    for values in series {
        for value in values {
            max_value = max_value.max(*value);
        }
    }
    max_value.max(fallback)
}

fn attenuation_ratio(input_rms: f32, output_rms: f32) -> f32 {
    if input_rms <= f32::EPSILON {
        0.0
    } else {
        (1.0 - (output_rms / input_rms)).clamp(0.0, 1.0)
    }
}

fn safe_available_width(ui: &egui::Ui, minimum: f32) -> f32 {
    let width = ui.available_width();
    if width.is_finite() {
        width.max(minimum)
    } else {
        minimum
    }
}

fn format_error_chain(error: &Error) -> String {
    let mut message = String::new();
    for (index, source) in error.chain().enumerate() {
        if index > 0 {
            message.push_str(": ");
        }
        message.push_str(&source.to_string());
    }
    message
}

fn install_windows_cjk_font_fallback(ctx: &egui::Context) {
    let Some(font_path) = find_windows_cjk_font() else {
        warn!("no Windows CJK font fallback found; Chinese text may render as tofu");
        return;
    };

    let font_name = format!(
        "windows_cjk_{}",
        font_path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("fallback")
    );
    let font_bytes = match fs::read(&font_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!(
                path = %font_path.display(),
                %error,
                "failed to read Windows CJK font fallback"
            );
            return;
        }
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        font_name.clone(),
        egui::FontData::from_owned(font_bytes).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push(font_name.clone());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push(font_name.clone());
    ctx.set_fonts(fonts);

    info!(path = %font_path.display(), "installed Windows CJK font fallback");
}

fn find_windows_cjk_font() -> Option<PathBuf> {
    let windows_dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let fonts_dir = windows_dir.join("Fonts");

    windows_cjk_font_candidates(&fonts_dir)
        .into_iter()
        .find(|path| path.is_file())
}

fn windows_cjk_font_candidates(fonts_dir: &Path) -> Vec<PathBuf> {
    [
        "NotoSansSC-VF.ttf",
        "NotoSerifSC-VF.ttf",
        "msyh.ttc",
        "msyhl.ttc",
        "msyhbd.ttc",
        "simhei.ttf",
        "simfang.ttf",
        "simkai.ttf",
        "simsunb.ttf",
        "simsun.ttc",
    ]
    .into_iter()
    .map(|name| fonts_dir.join(name))
    .collect()
}

fn run_worker(
    mut config_path: String,
    stop: Arc<AtomicBool>,
    control_rx: Receiver<WorkerCommand>,
    tx: Sender<WorkerEvent>,
) {
    let _ = init_logging("info");

    let mut runtime: Option<PipelineRuntime> = None;
    let mut frame_sleep = Duration::from_millis(10);
    let mut recovery_attempt = 0_u64;

    while !stop.load(Ordering::Relaxed) {
        while let Ok(command) = control_rx.try_recv() {
            match command {
                WorkerCommand::Reload {
                    config_path: next_path,
                } => {
                    config_path = next_path;
                    if let Some(mut active_runtime) = runtime.take() {
                        let _ = active_runtime.shutdown();
                    }
                    let _ = tx.send(WorkerEvent::Recovering(format!(
                        "reloading runtime from {}",
                        config_path
                    )));
                }
            }
        }

        if runtime.is_none() {
            match load_config(&config_path) {
                Ok(config) => {
                    frame_sleep = Duration::from_millis(config.audio.frame_ms as u64);
                    match PipelineRuntime::new(config) {
                        Ok(active_runtime) => {
                            runtime = Some(active_runtime);
                            recovery_attempt = 0;
                            continue;
                        }
                        Err(error) => {
                            recovery_attempt += 1;
                            let details = format_error_chain(&error);
                            let _ = tx.send(WorkerEvent::Recovering(format!(
                                "attempt {recovery_attempt}: failed to build runtime: {details}"
                            )));
                        }
                    }
                }
                Err(error) => {
                    recovery_attempt += 1;
                    let details = format_error_chain(&error);
                    let _ = tx.send(WorkerEvent::Recovering(format!(
                        "attempt {recovery_attempt}: failed to load config `{config_path}`: {details}"
                    )));
                }
            }

            if !sleep_with_stop(&stop, WORKER_RECOVERY_DELAY) {
                break;
            }
            continue;
        }

        let step_result = runtime
            .as_mut()
            .expect("runtime should exist when stepping")
            .step();
        match step_result {
            Ok(snapshot) => {
                if tx.send(WorkerEvent::Snapshot(snapshot)).is_err() {
                    break;
                }
                if !sleep_with_stop(&stop, frame_sleep) {
                    break;
                }
            }
            Err(error) => {
                recovery_attempt += 1;
                if let Some(mut active_runtime) = runtime.take() {
                    let _ = active_runtime.shutdown();
                }
                let details = format_error_chain(&error);
                let _ = tx.send(WorkerEvent::Recovering(format!(
                    "attempt {recovery_attempt}: runtime step failed: {details}"
                )));
                if !sleep_with_stop(&stop, WORKER_RECOVERY_DELAY) {
                    break;
                }
            }
        }
    }

    if let Some(mut active_runtime) = runtime {
        let _ = active_runtime.shutdown();
    }

    let _ = tx.send(WorkerEvent::Stopped);
}

fn sleep_with_stop(stop: &Arc<AtomicBool>, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            return true;
        }

        let remaining = deadline.saturating_duration_since(now);
        thread::sleep(remaining.min(WORKER_SLEEP_SLICE));
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save_config;
    use common_types::{AudioBackend, NodeConfig, OutputBackend, TransportBackend};
    use crossbeam_channel::RecvTimeoutError;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn worker_reloads_runtime_without_full_gui_restart() {
        let path = unique_test_config_path("reload");
        save_config(&path, &test_config("reload-before")).expect("initial config should save");

        let mut worker = spawn_test_worker(path.to_string_lossy().into_owned());
        let initial_snapshot = wait_for_snapshot_named(&worker.rx, "reload-before", 5);
        assert_eq!(initial_snapshot.node_name, "reload-before");

        save_config(&path, &test_config("reload-after")).expect("updated config should save");
        assert!(
            worker.request_reload(path.to_string_lossy().into_owned()),
            "reload command should reach worker"
        );

        let reloaded_snapshot = wait_for_snapshot_named(&worker.rx, "reload-after", 5);
        assert_eq!(reloaded_snapshot.node_name, "reload-after");

        worker.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn worker_recovers_after_missing_config_is_created() {
        let path = unique_test_config_path("recover");
        let mut worker = spawn_test_worker(path.to_string_lossy().into_owned());

        let recovery_message = wait_for_recovering_message(&worker.rx, 5);
        assert!(
            recovery_message.contains("failed to load config"),
            "expected config-load recovery message, got `{recovery_message}`"
        );

        save_config(&path, &test_config("recover-online")).expect("recovery config should save");
        let snapshot = wait_for_snapshot_named(&worker.rx, "recover-online", 5);
        assert_eq!(snapshot.node_name, "recover-online");

        worker.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reload_config_fields_updates_ui_values_and_status() {
        let path = unique_test_config_path("load-config");
        let mut config = test_config("load-config");
        config.audio.input_device = "default".to_owned();
        config.output.target_device = "Speaker A".to_owned();
        save_config(&path, &config).expect("load-config test config should save");

        let mut app = test_app(path.as_path());
        app.input_device_value = "stale-input".to_owned();
        app.target_device_value = "stale-output".to_owned();

        app.reload_config_fields();

        assert_eq!(app.input_device_value, "default");
        assert_eq!(app.target_device_value, "Speaker A");
        assert_eq!(
            app.config_feedback.as_deref(),
            Some(format!("Config loaded from {}", path.display()).as_str())
        );
        assert_eq!(app.status, format!("Config loaded: {}", path.display()));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persist_ui_device_fields_to_config_writes_selected_devices() {
        let path = unique_test_config_path("persist-devices");
        save_config(&path, &test_config("persist-devices"))
            .expect("persist-devices test config should save");

        let mut app = test_app(path.as_path());
        app.input_device_value = " default ".to_owned();
        app.target_device_value = " Speaker B ".to_owned();

        assert!(
            app.persist_ui_device_fields_to_config()
                .expect("device field persistence should succeed"),
            "device field persistence should report a config change"
        );

        let reloaded = load_config(&path).expect("persisted config should reload");
        assert_eq!(reloaded.audio.input_device, "default");
        assert_eq!(reloaded.output.target_device, "Speaker B");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn record_snapshot_trims_metric_history() {
        let path = unique_test_config_path("metric-history");
        let mut app = test_app(path.as_path());

        for sequence in 0..(METRIC_HISTORY_LIMIT as u64 + 32) {
            let mut snapshot = RuntimeSnapshot::default();
            snapshot.sequence = sequence;
            app.record_snapshot(snapshot);
        }

        assert_eq!(app.metric_history.len(), METRIC_HISTORY_LIMIT);
        assert_eq!(
            app.latest.as_ref().map(|snapshot| snapshot.sequence),
            Some(METRIC_HISTORY_LIMIT as u64 + 31)
        );
        assert_eq!(
            app.metric_history.front().map(|snapshot| snapshot.sequence),
            Some(32)
        );
    }

    #[test]
    fn metrics_dashboard_renders_without_nan_geometry() {
        let path = unique_test_config_path("metrics-render");
        let mut app = test_app(path.as_path());

        for sequence in 0..32_u64 {
            let mut snapshot = RuntimeSnapshot::default();
            snapshot.node_name = "metrics-render".to_owned();
            snapshot.sequence = sequence;
            snapshot.coarse_delay_ms = 20.0;
            snapshot.coherence = 0.95;
            snapshot.local_vad.score = 0.3;
            snapshot.peer_vad.score = 0.8;
            snapshot.input_rms = 0.12;
            snapshot.output_rms = 0.03;
            snapshot.estimated_crosstalk_rms = 0.04;
            snapshot.transport_loss_rate = 0.1;
            snapshot.processing_time_us = 900;
            app.record_snapshot(snapshot);
        }

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };

        let _ = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_metrics_dashboard(
                    ui,
                    app.latest
                        .as_ref()
                        .expect("metrics render test should have a snapshot"),
                );
            });
        });
    }

    fn spawn_test_worker(config_path: String) -> WorkerHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let (control_tx, control_rx) = unbounded();
        let (tx, rx) = unbounded();
        let join = thread::spawn(move || run_worker(config_path, stop_worker, control_rx, tx));

        WorkerHandle {
            stop,
            control_tx,
            rx,
            join: Some(join),
        }
    }

    fn test_app(config_path: &Path) -> NodeGuiApp {
        NodeGuiApp {
            config_path: config_path.display().to_string(),
            config_presets: vec![config_path.display().to_string()],
            status: "Idle".to_owned(),
            latest: None,
            metric_history: VecDeque::with_capacity(METRIC_HISTORY_LIMIT),
            worker: None,
            capture_devices: Vec::new(),
            render_devices: Vec::new(),
            input_device_value: String::new(),
            target_device_value: String::new(),
            config_feedback: None,
            config_discovery_error: None,
            config_mode_summary: None,
            config_mode_warning: None,
            loaded_audio_backend: None,
            loaded_transport_backend: None,
            loaded_output_backend: None,
            device_probe_error: None,
        }
    }

    fn test_config(node_name: &str) -> NodeConfig {
        let mut config = NodeConfig::default();
        config.node.name = node_name.to_owned();
        config.node.transport_backend = TransportBackend::Mock;
        config.audio.backend = AudioBackend::Mock;
        config.output.backend = OutputBackend::Null;
        config.debug.dump_wav = false;
        config.debug.dump_metrics = false;
        config
    }

    fn unique_test_config_path(label: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ek_dual_mic_{label}_{unique}.toml"))
    }

    fn wait_for_snapshot_named(
        rx: &Receiver<WorkerEvent>,
        expected_node_name: &str,
        timeout_seconds: u64,
    ) -> RuntimeSnapshot {
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for snapshot `{expected_node_name}`"
            );

            match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(WorkerEvent::Snapshot(snapshot)) if snapshot.node_name == expected_node_name => {
                    return snapshot;
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("worker channel disconnected before snapshot `{expected_node_name}`")
                }
            }
        }
    }

    fn wait_for_recovering_message(rx: &Receiver<WorkerEvent>, timeout_seconds: u64) -> String {
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for recovery message"
            );

            match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(WorkerEvent::Recovering(message)) => return message,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("worker channel disconnected before recovery message")
                }
            }
        }
    }
}
