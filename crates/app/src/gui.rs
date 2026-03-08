use crate::{
    ConfigImportPreview, PipelineRuntime, discover_config_presets, import_config_directory,
    init_logging, load_config, preview_import_config_directory, resolve_config_path, save_config,
};
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
#[cfg(windows)]
use rfd::FileDialog;
use std::collections::VecDeque;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

const WORKER_RECOVERY_DELAY: Duration = Duration::from_millis(750);
const WORKER_SLEEP_SLICE: Duration = Duration::from_millis(50);
const METRIC_HISTORY_LIMIT: usize = 240;
const WINDOWS_UI_REGULAR_FAMILY: &str = "windows_ui_regular";
const WINDOWS_UI_BOLD_FAMILY: &str = "windows_ui_bold";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum UiLanguage {
    English,
    #[default]
    Chinese,
}

impl UiLanguage {
    fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Chinese => "中文",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MetricsPanelSize {
    #[default]
    Compact,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MainTab {
    #[default]
    Metrics,
    RecordingTest,
}

impl MainTab {
    fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::Metrics => localized(language, "Metrics", "指标"),
            Self::RecordingTest => localized(language, "Recording Test", "录制测试"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct MetricsLayout {
    stat_columns: usize,
    panel_columns: usize,
    stat_card_height: f32,
    chart_height: f32,
    progress_bar_height: f32,
}

#[derive(Default)]
struct NoiseControlUiState {
    changed: bool,
    apply_clicked: bool,
    monitor_processed_rect: Option<egui::Rect>,
    update_threshold_rect: Option<egui::Rect>,
    anti_phase_depth_rect: Option<egui::Rect>,
    residual_strength_rect: Option<egui::Rect>,
}

impl MetricsPanelSize {
    fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::Compact => localized(language, "Small", "小"),
            Self::Medium => localized(language, "Medium", "中"),
            Self::Large => localized(language, "Large", "大"),
        }
    }

    fn layout(self) -> MetricsLayout {
        match self {
            Self::Compact => MetricsLayout {
                stat_columns: 4,
                panel_columns: 4,
                stat_card_height: 72.0,
                chart_height: 92.0,
                progress_bar_height: 14.0,
            },
            Self::Medium => MetricsLayout {
                stat_columns: 4,
                panel_columns: 2,
                stat_card_height: 82.0,
                chart_height: 132.0,
                progress_bar_height: 18.0,
            },
            Self::Large => MetricsLayout {
                stat_columns: 2,
                panel_columns: 1,
                stat_card_height: 92.0,
                chart_height: 172.0,
                progress_bar_height: 22.0,
            },
        }
    }
}

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
    language: UiLanguage,
    config_path: String,
    config_presets: Vec<String>,
    status: String,
    active_tab: MainTab,
    latest: Option<RuntimeSnapshot>,
    metric_history: VecDeque<RuntimeSnapshot>,
    metrics_panel_size: MetricsPanelSize,
    worker: Option<WorkerHandle>,
    capture_devices: Vec<AudioDeviceInfo>,
    render_devices: Vec<AudioDeviceInfo>,
    listen_addr_value: String,
    peer_addr_value: String,
    input_device_value: String,
    target_device_value: String,
    monitor_processed_output_value: bool,
    cancel_step_size_value: f32,
    cancel_update_threshold_value: f32,
    anti_phase_enabled_value: bool,
    anti_phase_max_gain_value: f32,
    anti_phase_smoothing_value: f32,
    residual_enabled_value: bool,
    residual_strength_value: f32,
    config_feedback: Option<String>,
    config_discovery_error: Option<String>,
    pending_config_import: Option<ConfigImportPreview>,
    config_mode_summary: Option<String>,
    config_mode_warning: Option<String>,
    loaded_node_name: Option<String>,
    loaded_dump_dir: Option<String>,
    loaded_wav_path: Option<String>,
    loaded_audio_backend: Option<AudioBackend>,
    loaded_transport_backend: Option<TransportBackend>,
    loaded_output_backend: Option<OutputBackend>,
    device_probe_error: Option<String>,
}

impl Default for NodeGuiApp {
    fn default() -> Self {
        let mut app = Self {
            language: UiLanguage::Chinese,
            config_path: "configs/node-a.toml".to_owned(),
            config_presets: Vec::new(),
            status: "Idle".to_owned(),
            active_tab: MainTab::Metrics,
            latest: None,
            metric_history: VecDeque::with_capacity(METRIC_HISTORY_LIMIT),
            metrics_panel_size: MetricsPanelSize::Compact,
            worker: None,
            capture_devices: Vec::new(),
            render_devices: Vec::new(),
            listen_addr_value: String::new(),
            peer_addr_value: String::new(),
            input_device_value: String::new(),
            target_device_value: String::new(),
            monitor_processed_output_value: true,
            cancel_step_size_value: 0.06,
            cancel_update_threshold_value: 0.48,
            anti_phase_enabled_value: true,
            anti_phase_max_gain_value: 1.45,
            anti_phase_smoothing_value: 0.72,
            residual_enabled_value: true,
            residual_strength_value: 0.72,
            config_feedback: None,
            config_discovery_error: None,
            pending_config_import: None,
            config_mode_summary: None,
            config_mode_warning: None,
            loaded_node_name: None,
            loaded_dump_dir: None,
            loaded_wav_path: None,
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
    fn ui_text<'a>(&self, english: &'a str, chinese: &'a str) -> &'a str {
        localized(self.language, english, chinese)
    }

    fn set_language(&mut self, language: UiLanguage) {
        self.language = language;
        if let Ok(config) = load_config(&self.config_path) {
            self.sync_loaded_config_metadata(&config);
        }
    }

    fn metrics_layout(&self) -> MetricsLayout {
        self.metrics_panel_size.layout()
    }

    fn normalized_device_field(value: &str) -> String {
        value.trim().to_owned()
    }

    fn normalized_socket_field(value: &str, fallback: &str) -> String {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        if trimmed.parse::<SocketAddr>().is_ok() {
            return trimmed.to_owned();
        }

        if trimmed.parse::<IpAddr>().is_ok() {
            if let Some(port) = socket_address_port(fallback) {
                return format!("{trimmed}:{port}");
            }
        }

        trimmed.to_owned()
    }

    fn sync_loaded_config_metadata(&mut self, config: &common_types::NodeConfig) {
        self.loaded_node_name = Some(config.node.name.clone());
        self.loaded_dump_dir = Some(config.debug.dump_dir.display().to_string());
        self.loaded_wav_path = Some(config.output.wav_path.display().to_string());
        self.loaded_audio_backend = Some(config.audio.backend);
        self.loaded_transport_backend = Some(config.node.transport_backend);
        self.loaded_output_backend = Some(config.output.backend);
        info!(
            node = %config.node.name,
            listen = %config.node.listen_addr,
            peer = %config.node.peer_addr,
            audio = backend_label_audio(config.audio.backend),
            transport = backend_label_transport(config.node.transport_backend),
            output = backend_label_output(config.output.backend),
            "GUI loaded config metadata"
        );
        self.config_mode_summary = Some(format!(
            "node={}, listen={}, peer={}, audio={}, transport={}, output={}",
            config.node.name,
            config.node.listen_addr,
            config.node.peer_addr,
            backend_label_audio(config.audio.backend),
            backend_label_transport(config.node.transport_backend),
            backend_label_output(config.output.backend),
        ));
        self.config_mode_warning = if config.audio.backend == AudioBackend::Mock {
            Some(
                self.ui_text(
                    "Current config uses mock audio input. Live microphone signal is ignored.",
                    "当前配置使用 mock 音频输入，真实麦克风信号会被忽略。",
                )
                .to_owned(),
            )
        } else if config.node.transport_backend == TransportBackend::Mock {
            Some(
                self.ui_text(
                    "Current config uses mock transport. `listen_addr` and `peer_addr` are ignored.",
                    "当前配置使用 mock 传输，`listen_addr` 和 `peer_addr` 会被忽略。",
                )
                .to_owned(),
            )
        } else if config.output.backend != OutputBackend::VirtualStub {
            Some(
                self.ui_text(
                    "Current config does not write to a live output device. `target_device` is ignored in this mode.",
                    "当前配置不会写入实时输出设备，`target_device` 在该模式下会被忽略。",
                )
                .to_owned(),
            )
        } else {
            None
        };
    }

    fn capture_selection_enabled(&self) -> bool {
        self.loaded_audio_backend != Some(AudioBackend::Mock)
    }

    fn transport_selection_enabled(&self) -> bool {
        self.loaded_transport_backend != Some(TransportBackend::Mock)
    }

    fn render_selection_enabled(&self) -> bool {
        !matches!(
            self.loaded_output_backend,
            Some(OutputBackend::Null | OutputBackend::WavDump)
        )
    }

    fn selected_device_text(&self, value: &str) -> String {
        if value.trim().is_empty() {
            self.ui_text("Select a device", "选择设备").to_owned()
        } else {
            value.to_owned()
        }
    }

    fn draw_capture_device_dropdown(&mut self, ui: &mut egui::Ui) -> bool {
        let default_label = self.ui_text("Default Capture", "默认输入").to_owned();
        let before = self.input_device_value.clone();
        egui::ComboBox::from_id_salt("capture_device_combo")
            .selected_text(self.selected_device_text(&self.input_device_value))
            .width(safe_available_width(ui, 220.0))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.input_device_value,
                    "default".to_owned(),
                    default_label,
                );
                for device in &self.capture_devices {
                    ui.selectable_value(
                        &mut self.input_device_value,
                        device.name.clone(),
                        format_device_label(device),
                    );
                }
            });
        self.input_device_value != before
    }

    fn draw_render_device_dropdown(&mut self, ui: &mut egui::Ui) -> bool {
        let default_label = self.ui_text("Default Render", "默认输出").to_owned();
        let before = self.target_device_value.clone();
        egui::ComboBox::from_id_salt("render_device_combo")
            .selected_text(self.selected_device_text(&self.target_device_value))
            .width(safe_available_width(ui, 220.0))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.target_device_value,
                    "default".to_owned(),
                    default_label,
                );
                for device in &self.render_devices {
                    ui.selectable_value(
                        &mut self.target_device_value,
                        device.name.clone(),
                        format_device_label(device),
                    );
                }
        });
        self.target_device_value != before
    }

    fn draw_noise_reduction_controls(&mut self, ui: &mut egui::Ui) -> NoiseControlUiState {
        let mut state = NoiseControlUiState::default();
        let monitor_processed_label = self
            .ui_text("Monitor processed output", "监听处理后输出")
            .to_owned();
        let anti_phase_label = self
            .ui_text("Enable anti-phase", "启用反向波抵消")
            .to_owned();
        let residual_enabled_label = self
            .ui_text("Enable residual suppressor", "启用残余抑制")
            .to_owned();
        let adaptation_speed_label = self.ui_text("Adaptation speed", "自适应速度").to_owned();
        let update_threshold_label = self.ui_text("Update threshold", "更新阈值").to_owned();
        let anti_phase_depth_label = self.ui_text("Anti-phase depth", "反向波深度").to_owned();
        let anti_phase_smoothing_label = self
            .ui_text("Anti-phase smoothing", "反向波平滑")
            .to_owned();
        let residual_strength_label = self
            .ui_text("Residual strength", "残余抑制强度")
            .to_owned();
        let monitor_processed_hint = self.ui_text(
            "On: hear the processed result. Off: hear the raw capture path for troubleshooting.",
            "打开：听处理后的结果。关闭：听原始采集链，适合排查问题。",
        );
        let anti_phase_enabled_hint = self.ui_text(
            "Usually keep this on. It subtracts the peer reference before the main adaptive filter finishes converging.",
            "通常建议打开。它会先用对端参考做一轮前置抵消，再交给主自适应滤波器继续压。",
        );
        let residual_enabled_hint = self.ui_text(
            "Usually keep this on. It mainly suppresses the leftover low-level leakage after the main cancel stage.",
            "通常建议打开。它主要负责把主消除之后剩下的低电平残留继续压下去。",
        );
        let adaptation_speed_hint = self.ui_text(
            "Right: learns peer leakage faster, usually cancels more aggressively. Too far right can make your own voice thinner or twitchy.",
            "向右：更快学习对端泄漏，通常压得更狠。拉得过右时，自己声音可能会变薄或发飘。",
        );
        let update_threshold_hint = self.ui_text(
            "Left: easier to enter update mode, usually stronger cancellation. Right: more conservative and safer, but may leave more peer voice behind.",
            "向左：更容易进入更新状态，通常消除更强。向右：更保守、更稳，但更容易残留对端声音。",
        );
        let anti_phase_depth_hint = self.ui_text(
            "Right: stronger direct anti-phase subtraction of the peer voice. Too far right can cause pumping or hollow artifacts.",
            "向右：更强地直接抵消对端声音。拉得过右时，可能出现抽动感或空洞感。",
        );
        let anti_phase_smoothing_hint = self.ui_text(
            "Right: changes more smoothly but reacts slower. Left: follows changes faster but may become less stable.",
            "向右：变化更平稳，但跟随更慢。向左：跟随更快，但也更容易不稳。",
        );
        let residual_strength_hint = self.ui_text(
            "Right: more aggressively suppresses the last bit of peer leakage and hiss. Too far right can make the sound dull.",
            "向右：更狠地压最后那一点对端残留和底噪。拉得过右时，声音可能会发闷。",
        );

        let monitor_response = ui.checkbox(
            &mut self.monitor_processed_output_value,
            monitor_processed_label,
        );
        state.changed |= monitor_response.changed();
        state.monitor_processed_rect = Some(monitor_response.rect);
        ui.small(monitor_processed_hint);

        state.changed |= ui
            .checkbox(&mut self.anti_phase_enabled_value, anti_phase_label)
            .changed();
        ui.small(anti_phase_enabled_hint);
        state.changed |= ui
            .checkbox(&mut self.residual_enabled_value, residual_enabled_label)
            .changed();
        ui.small(residual_enabled_hint);

        state.changed |= ui
            .add(
                egui::Slider::new(&mut self.cancel_step_size_value, 0.005..=0.12)
                    .text(adaptation_speed_label),
            )
            .changed();
        ui.small(adaptation_speed_hint);

        let update_threshold_response = ui.add(
            egui::Slider::new(&mut self.cancel_update_threshold_value, 0.20..=0.95)
                .text(update_threshold_label),
        );
        state.changed |= update_threshold_response.changed();
        state.update_threshold_rect = Some(update_threshold_response.rect);
        ui.small(update_threshold_hint);

        let anti_phase_depth_response = ui.add(
            egui::Slider::new(&mut self.anti_phase_max_gain_value, 0.20..=2.00)
                .text(anti_phase_depth_label),
        );
        state.changed |= anti_phase_depth_response.changed();
        state.anti_phase_depth_rect = Some(anti_phase_depth_response.rect);
        ui.small(anti_phase_depth_hint);

        state.changed |= ui
            .add(
                egui::Slider::new(&mut self.anti_phase_smoothing_value, 0.10..=0.95)
                    .text(anti_phase_smoothing_label),
            )
            .changed();
        ui.small(anti_phase_smoothing_hint);

        let residual_strength_response = ui.add(
            egui::Slider::new(&mut self.residual_strength_value, 0.0..=1.0)
                .text(residual_strength_label),
        );
        state.changed |= residual_strength_response.changed();
        state.residual_strength_rect = Some(residual_strength_response.rect);
        ui.small(residual_strength_hint);

        ui.small(self.ui_text(
            "Lower update threshold and higher anti-phase/residual values usually suppress more peer leakage, but can also make your own voice sound thinner.",
            "更低的更新阈值和更高的反向波 / 残余抑制通常会更强地压制对端泄漏，但也可能让自己声音变薄。",
        ));
        let apply_response = ui.button(self.ui_text("Apply Noise Controls", "应用降噪参数"));
        state.apply_clicked = apply_response.clicked();
        if state.changed {
            ui.small(self.ui_text(
                "Pending noise-control changes are waiting to be saved.",
                "降噪参数已修改，等待保存生效。",
            ));
        }

        state
    }

    fn handle_live_device_selection_change(&mut self) {
        let changed = match self.persist_ui_runtime_fields_to_config() {
            Ok(changed) => changed,
            Err(()) => return,
        };

        if self.worker.is_some() {
            if changed {
                self.request_runtime_reload();
                self.config_feedback = Some(
                    self.ui_text(
                        "Device selection applied; runtime reload requested",
                        "设备切换已应用；已请求重新加载运行时",
                    )
                    .to_owned(),
                );
            }
        } else if changed {
            self.config_feedback = Some(
                self.ui_text("Device selection applied to config", "设备切换已写入配置")
                    .to_owned(),
            );
        }
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

    fn config_import_initial_directory(&self) -> PathBuf {
        let resolved = resolve_config_path(&self.config_path);
        if resolved.is_dir() {
            return resolved;
        }

        resolved
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn pick_and_import_config_folder(&mut self) {
        #[cfg(windows)]
        {
            let Some(selected_dir) = FileDialog::new()
                .set_directory(self.config_import_initial_directory())
                .pick_folder()
            else {
                return;
            };

            self.preview_config_folder_import(selected_dir);
        }
    }

    fn preview_config_folder_import(&mut self, folder: PathBuf) {
        match preview_import_config_directory(&folder) {
            Ok(preview) => {
                if preview.discovered_files.is_empty() {
                    self.config_feedback = Some(
                        self.ui_text(
                            "No .toml config files were found in the selected folder",
                            "所选文件夹中未找到 .toml 配置文件",
                        )
                        .to_owned(),
                    );
                } else if preview.conflicts.is_empty() {
                    self.apply_config_import(folder, false);
                } else {
                    self.pending_config_import = Some(preview);
                    self.config_feedback = Some(format!(
                        "{} {}",
                        self.ui_text(
                            "Config name conflicts detected in",
                            "在以下目录中检测到配置文件名冲突：",
                        ),
                        folder.display()
                    ));
                }
            }
            Err(error) => {
                let details = format_error_chain(&error);
                self.config_feedback = Some(format!(
                    "{}: {details}",
                    self.ui_text("Config folder import failed", "配置文件夹导入失败",)
                ));
                self.status = format!(
                    "{}: {details}",
                    self.ui_text("Config folder import failed", "配置文件夹导入失败",)
                );
            }
        }
    }

    fn apply_config_import(&mut self, folder: PathBuf, import_conflicts_with_rename: bool) {
        match import_config_directory(&folder, import_conflicts_with_rename) {
            Ok(result) => {
                self.pending_config_import = None;
                self.refresh_config_presets();
                if let Some(first_imported) = result.imported_paths.first() {
                    self.config_path = first_imported.clone();
                    self.reload_config_fields();
                }

                let mut summary = Vec::new();
                if !result.imported_paths.is_empty() {
                    summary.push(format!(
                        "{} {}",
                        self.ui_text("Imported", "已导入"),
                        result.imported_paths.join(", ")
                    ));
                }
                if !result.skipped_duplicates.is_empty() {
                    summary.push(format!(
                        "{} {}",
                        self.ui_text("Skipped duplicates", "已跳过重复项"),
                        result.skipped_duplicates.join(", ")
                    ));
                }
                if !result.renamed_imports.is_empty() {
                    let renamed = result
                        .renamed_imports
                        .iter()
                        .map(|(from, to)| format!("{from} -> {to}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    summary.push(format!(
                        "{} {renamed}",
                        self.ui_text("Renamed conflicts", "已重命名冲突项")
                    ));
                }
                if !result.skipped_conflicts.is_empty() {
                    summary.push(format!(
                        "{} {}",
                        self.ui_text("Skipped conflicts", "已跳过冲突项"),
                        result.skipped_conflicts.join(", ")
                    ));
                }

                let message = if summary.is_empty() {
                    self.ui_text("No new configs were imported", "没有导入新的配置文件")
                        .to_owned()
                } else {
                    summary.join(" | ")
                };
                self.config_feedback = Some(message.clone());
                self.status = message;
            }
            Err(error) => {
                let details = format_error_chain(&error);
                self.config_feedback = Some(format!(
                    "{}: {details}",
                    self.ui_text("Config folder import failed", "配置文件夹导入失败",)
                ));
                self.status = format!(
                    "{}: {details}",
                    self.ui_text("Config folder import failed", "配置文件夹导入失败",)
                );
            }
        }
    }

    fn reload_config_fields(&mut self) {
        match load_config(&self.config_path) {
            Ok(config) => {
                self.sync_loaded_config_metadata(&config);
                self.listen_addr_value = config.node.listen_addr;
                self.peer_addr_value = config.node.peer_addr;
                self.input_device_value = config.audio.input_device;
                self.target_device_value = config.output.target_device;
                self.monitor_processed_output_value = config.output.monitor_processed_output;
                self.cancel_step_size_value = config.cancel.step_size;
                self.cancel_update_threshold_value = config.cancel.update_threshold;
                self.anti_phase_enabled_value = config.cancel.anti_phase_enabled;
                self.anti_phase_max_gain_value = config.cancel.anti_phase_max_gain;
                self.anti_phase_smoothing_value = config.cancel.anti_phase_smoothing;
                self.residual_enabled_value = config.residual.enabled;
                self.residual_strength_value = config.residual.strength;
                info!(config_path = %self.config_path, "GUI loaded config into form");
                self.config_feedback = Some(format!(
                    "{} {}",
                    self.ui_text("Config loaded from", "已从以下路径加载配置："),
                    self.config_path
                ));
                self.status = format!(
                    "{}: {}",
                    self.ui_text("Config loaded", "配置已加载"),
                    self.config_path
                );
                if self.request_runtime_reload() {
                    self.config_feedback = Some(format!(
                        "{} {}; {}",
                        self.ui_text("Config loaded from", "已从以下路径加载配置："),
                        self.config_path,
                        self.ui_text("runtime reload requested", "已请求重新加载运行时",)
                    ));
                }
            }
            Err(error) => {
                let details = format_error_chain(&error);
                warn!(config_path = %self.config_path, error = %details, "GUI failed to load config into form");
                self.config_feedback = Some(format!(
                    "{}: {details}",
                    self.ui_text("Config load failed", "配置加载失败")
                ));
                self.status = format!(
                    "{}: {details}",
                    self.ui_text("Config load failed", "配置加载失败")
                );
            }
        }
    }

    fn persist_ui_runtime_fields_to_config(&mut self) -> Result<bool, ()> {
        match load_config(&self.config_path) {
            Ok(mut config) => {
                let listen_addr = Self::normalized_socket_field(
                    &self.listen_addr_value,
                    &config.node.listen_addr,
                );
                let peer_addr =
                    Self::normalized_socket_field(&self.peer_addr_value, &config.node.peer_addr);
                let input_device = Self::normalized_device_field(&self.input_device_value);
                let target_device = Self::normalized_device_field(&self.target_device_value);
                let monitor_processed_output = self.monitor_processed_output_value;
                let cancel_step_size = self.cancel_step_size_value.clamp(0.001, 0.2);
                let cancel_update_threshold = self.cancel_update_threshold_value.clamp(0.0, 0.99);
                let anti_phase_enabled = self.anti_phase_enabled_value;
                let anti_phase_max_gain = self.anti_phase_max_gain_value.clamp(0.0, 2.0);
                let anti_phase_smoothing = self.anti_phase_smoothing_value.clamp(0.0, 0.99);
                let residual_enabled = self.residual_enabled_value;
                let residual_strength = self.residual_strength_value.clamp(0.0, 1.0);
                let changed = config.node.listen_addr != listen_addr
                    || config.node.peer_addr != peer_addr
                    || config.audio.input_device != input_device
                    || config.output.target_device != target_device
                    || config.output.monitor_processed_output != monitor_processed_output
                    || (config.cancel.step_size - cancel_step_size).abs() > f32::EPSILON
                    || (config.cancel.update_threshold - cancel_update_threshold).abs()
                        > f32::EPSILON
                    || config.cancel.anti_phase_enabled != anti_phase_enabled
                    || (config.cancel.anti_phase_max_gain - anti_phase_max_gain).abs()
                        > f32::EPSILON
                    || (config.cancel.anti_phase_smoothing - anti_phase_smoothing).abs()
                        > f32::EPSILON
                    || config.residual.enabled != residual_enabled
                    || (config.residual.strength - residual_strength).abs() > f32::EPSILON;

                config.node.listen_addr = listen_addr.clone();
                config.node.peer_addr = peer_addr.clone();
                config.audio.input_device = input_device;
                config.output.target_device = target_device;
                config.output.monitor_processed_output = monitor_processed_output;
                config.cancel.step_size = cancel_step_size;
                config.cancel.update_threshold = cancel_update_threshold;
                config.cancel.anti_phase_enabled = anti_phase_enabled;
                config.cancel.anti_phase_max_gain = anti_phase_max_gain;
                config.cancel.anti_phase_smoothing = anti_phase_smoothing;
                config.residual.enabled = residual_enabled;
                config.residual.strength = residual_strength;
                self.listen_addr_value = listen_addr;
                self.peer_addr_value = peer_addr;
                self.cancel_step_size_value = cancel_step_size;
                self.cancel_update_threshold_value = cancel_update_threshold;
                self.anti_phase_max_gain_value = anti_phase_max_gain;
                self.anti_phase_smoothing_value = anti_phase_smoothing;
                self.residual_strength_value = residual_strength;

                match save_config(&self.config_path, &config) {
                    Ok(()) => {
                        self.sync_loaded_config_metadata(&config);
                        info!(config_path = %self.config_path, "GUI saved runtime form fields into config");
                        self.config_feedback = None;
                        return Ok(changed);
                    }
                    Err(error) => {
                        let details = format_error_chain(&error);
                        warn!(config_path = %self.config_path, error = %details, "GUI failed to save runtime form fields");
                        self.config_feedback = Some(format!(
                            "{}: {details}",
                            self.ui_text("Config save failed", "配置保存失败")
                        ));
                        self.status = format!(
                            "{}: {details}",
                            self.ui_text("Config save failed", "配置保存失败")
                        );
                    }
                }
            }
            Err(error) => {
                let details = format_error_chain(&error);
                warn!(config_path = %self.config_path, error = %details, "GUI could not load config before saving runtime form fields");
                self.config_feedback = Some(format!(
                    "{}: {details}",
                    self.ui_text("Config load failed", "配置加载失败")
                ));
                self.status = format!(
                    "{}: {details}",
                    self.ui_text("Config load failed", "配置加载失败")
                );
            }
        }

        Err(())
    }

    fn save_runtime_fields(&mut self) {
        let changed = match self.persist_ui_runtime_fields_to_config() {
            Ok(changed) => changed,
            Err(()) => return,
        };

        if self.request_runtime_reload() {
            self.config_feedback = Some(if changed {
                self.ui_text(
                    "Runtime fields saved; runtime reload requested",
                    "运行时字段已保存；已请求重新加载运行时",
                )
                .to_owned()
            } else {
                self.ui_text(
                    "Runtime fields unchanged; runtime reload requested",
                    "运行时字段未变化；已请求重新加载运行时",
                )
                .to_owned()
            });
        } else {
            self.config_feedback = Some(if changed {
                self.ui_text("Runtime fields saved", "运行时字段已保存")
                    .to_owned()
            } else {
                self.ui_text(
                    "Runtime fields already matched the config",
                    "运行时字段已与配置一致",
                )
                .to_owned()
            });
            self.status = format!(
                "{}: {}",
                self.ui_text("Config ready", "配置就绪"),
                self.config_path
            );
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
            self.status = self
                .ui_text("Reload requested", "已请求重新加载")
                .to_owned();
            true
        } else {
            warn!(config_path = %self.config_path, "GUI failed to queue runtime reload request");
            self.status = self
                .ui_text("Reload request failed", "重新加载请求失败")
                .to_owned();
            false
        }
    }

    fn start(&mut self) {
        if self.worker.is_some() {
            return;
        }

        let changed = match self.persist_ui_runtime_fields_to_config() {
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
        self.status = format!(
            "{}: {}",
            self.ui_text("Starting", "正在启动"),
            self.config_path
        );
        self.config_feedback = Some(if changed {
            self.ui_text(
                "Runtime will use the current devices and network fields from this config",
                "运行时将使用当前配置中的设备和网络字段",
            )
            .to_owned()
        } else {
            self.ui_text(
                "Runtime will use the devices and network fields already stored in this config",
                "运行时将使用该配置中已保存的设备和网络字段",
            )
            .to_owned()
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
        self.status = self.ui_text("Stopped", "已停止").to_owned();
    }

    fn poll_worker(&mut self) {
        let mut should_clear_worker = false;

        if let Some(worker) = self.worker.as_ref() {
            let events: Vec<_> = worker.rx.try_iter().collect();
            for event in events {
                match event {
                    WorkerEvent::Snapshot(snapshot) => {
                        self.status = format!(
                            "{}: {} {}",
                            self.ui_text("Running", "运行中"),
                            self.ui_text("frame", "帧"),
                            snapshot.sequence
                        );
                        self.record_snapshot(snapshot);
                    }
                    WorkerEvent::Recovering(message) => {
                        warn!(message = %message, "GUI worker entered recovering state");
                        self.status =
                            format!("{}: {message}", self.ui_text("Recovering", "恢复中"));
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
        let layout = self.metrics_layout();
        self.draw_metrics_stat_cards(ui, snapshot, layout);
        ui.add_space(6.0);
        self.draw_metrics_panel_grid(ui, snapshot, layout);
    }

    fn draw_metrics_stat_cards(
        &self,
        ui: &mut egui::Ui,
        snapshot: &RuntimeSnapshot,
        layout: MetricsLayout,
    ) {
        self.draw_metric_row(ui, layout.stat_columns.min(4), |column_index, column| {
            match column_index {
                0 => draw_stat_card(
                    column,
                    self.ui_text("Node", "节点"),
                    &snapshot.node_name,
                    self.ui_text("active config", "当前配置"),
                    layout.stat_card_height,
                ),
                1 => draw_stat_card(
                    column,
                    self.ui_text("Sequence", "序号"),
                    &snapshot.sequence.to_string(),
                    self.ui_text("latest frame", "最新帧"),
                    layout.stat_card_height,
                ),
                2 => draw_stat_card(
                    column,
                    self.ui_text("Delay", "延迟"),
                    &format!("{:.1} ms", snapshot.coarse_delay_ms),
                    self.ui_text("coarse sync offset", "粗同步偏移"),
                    layout.stat_card_height,
                ),
                3 => draw_stat_card(
                    column,
                    self.ui_text("Frame Time", "帧耗时"),
                    &format!("{} us", snapshot.processing_time_us),
                    self.ui_text("processing cost", "处理开销"),
                    layout.stat_card_height,
                ),
                _ => {}
            }
        });
    }

    fn draw_metrics_panel_grid(
        &self,
        ui: &mut egui::Ui,
        snapshot: &RuntimeSnapshot,
        layout: MetricsLayout,
    ) {
        for row_start in (0..4).step_by(layout.panel_columns) {
            let row_len = layout.panel_columns.min(4 - row_start);
            self.draw_metric_row(ui, row_len, |column_index, column| {
                match row_start + column_index {
                    0 => self.draw_audio_level_panel(column, snapshot, layout),
                    1 => self.draw_sync_quality_panel(column, snapshot, layout),
                    2 => self.draw_transport_panel(column, snapshot, layout),
                    3 => self.draw_timing_panel(column, snapshot, layout),
                    _ => {}
                }
            });

            if row_start + row_len < 4 {
                ui.add_space(6.0);
            }
        }
    }

    fn draw_metric_row(
        &self,
        ui: &mut egui::Ui,
        column_count: usize,
        mut draw: impl FnMut(usize, &mut egui::Ui),
    ) {
        ui.columns(column_count.max(1), |columns| {
            for (index, column) in columns.iter_mut().enumerate() {
                draw(index, column);
            }
        });
    }

    fn draw_audio_level_panel(
        &self,
        ui: &mut egui::Ui,
        snapshot: &RuntimeSnapshot,
        layout: MetricsLayout,
    ) {
        let input_history = self.metric_history_values(|entry| entry.input_rms);
        let output_history = self.metric_history_values(|entry| entry.output_rms);
        let crosstalk_history = self.metric_history_values(|entry| entry.estimated_crosstalk_rms);
        let level_max =
            max_series_value([&input_history, &output_history, &crosstalk_history], 0.05);

        draw_history_card(
            ui,
            self.ui_text("Audio Levels", "音频电平"),
            &[
                HistoryLine {
                    label: self.ui_text("Input RMS", "输入 RMS"),
                    color: egui::Color32::from_rgb(82, 196, 26),
                    values: &input_history,
                },
                HistoryLine {
                    label: self.ui_text("Output RMS", "输出 RMS"),
                    color: egui::Color32::from_rgb(250, 173, 20),
                    values: &output_history,
                },
                HistoryLine {
                    label: self.ui_text("Crosstalk", "串音"),
                    color: egui::Color32::from_rgb(255, 120, 117),
                    values: &crosstalk_history,
                },
            ],
            0.0,
            level_max,
            "RMS",
            layout.chart_height,
            self.language,
        );

        let attenuation = attenuation_ratio(snapshot.input_rms, snapshot.output_rms);
        draw_progress_metric(
            ui,
            self.ui_text("Input RMS", "输入 RMS"),
            snapshot.input_rms / level_max.max(f32::EPSILON),
            format!("{:.5}", snapshot.input_rms),
            egui::Color32::from_rgb(82, 196, 26),
            layout.progress_bar_height,
        );
        draw_progress_metric(
            ui,
            self.ui_text("Output RMS", "输出 RMS"),
            snapshot.output_rms / level_max.max(f32::EPSILON),
            format!("{:.5}", snapshot.output_rms),
            egui::Color32::from_rgb(250, 173, 20),
            layout.progress_bar_height,
        );
        draw_progress_metric(
            ui,
            self.ui_text("Attenuation", "衰减"),
            attenuation,
            format!("{:.1}%", attenuation * 100.0),
            egui::Color32::from_rgb(64, 169, 255),
            layout.progress_bar_height,
        );
    }

    fn draw_sync_quality_panel(
        &self,
        ui: &mut egui::Ui,
        snapshot: &RuntimeSnapshot,
        layout: MetricsLayout,
    ) {
        let coherence_history = self.metric_history_values(|entry| entry.coherence);
        let local_vad_history = self.metric_history_values(|entry| entry.local_vad.score);
        let peer_vad_history = self.metric_history_values(|entry| entry.peer_vad.score);

        draw_history_card(
            ui,
            self.ui_text("Sync And Voice Activity", "同步与语音活动"),
            &[
                HistoryLine {
                    label: self.ui_text("Coherence", "相干性"),
                    color: egui::Color32::from_rgb(64, 169, 255),
                    values: &coherence_history,
                },
                HistoryLine {
                    label: self.ui_text("Local VAD", "本地 VAD"),
                    color: egui::Color32::from_rgb(149, 117, 205),
                    values: &local_vad_history,
                },
                HistoryLine {
                    label: self.ui_text("Peer VAD", "对端 VAD"),
                    color: egui::Color32::from_rgb(255, 120, 117),
                    values: &peer_vad_history,
                },
            ],
            0.0,
            1.0,
            "score",
            layout.chart_height,
            self.language,
        );

        draw_progress_metric(
            ui,
            self.ui_text("Coherence", "相干性"),
            snapshot.coherence,
            format!("{:.3}", snapshot.coherence),
            egui::Color32::from_rgb(64, 169, 255),
            layout.progress_bar_height,
        );
        draw_progress_metric(
            ui,
            self.ui_text("Local VAD", "本地 VAD"),
            snapshot.local_vad.score,
            format!("{:.3}", snapshot.local_vad.score),
            egui::Color32::from_rgb(149, 117, 205),
            layout.progress_bar_height,
        );
        draw_progress_metric(
            ui,
            self.ui_text("Peer VAD", "对端 VAD"),
            snapshot.peer_vad.score,
            format!("{:.3}", snapshot.peer_vad.score),
            egui::Color32::from_rgb(255, 120, 117),
            layout.progress_bar_height,
        );

        ui.horizontal_wrapped(|ui| {
            ui.label(self.ui_text("Update State", "更新状态"));
            if snapshot.update_frozen {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 120, 117),
                    self.ui_text("Frozen", "冻结"),
                );
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(82, 196, 26),
                    self.ui_text("Adaptive", "自适应"),
                );
            }
        });
    }

    fn draw_transport_panel(
        &self,
        ui: &mut egui::Ui,
        snapshot: &RuntimeSnapshot,
        layout: MetricsLayout,
    ) {
        let loss_history = self.metric_history_values(|entry| entry.transport_loss_rate);
        let clip_history = self.metric_history_values(|entry| entry.clip_events as f32);
        let clip_max = max_series_value([&clip_history], 1.0);

        draw_history_card(
            ui,
            self.ui_text("Transport Health", "传输健康"),
            &[HistoryLine {
                label: self.ui_text("Loss Rate", "丢包率"),
                color: egui::Color32::from_rgb(255, 120, 117),
                values: &loss_history,
            }],
            0.0,
            1.0,
            "ratio",
            layout.chart_height,
            self.language,
        );

        draw_progress_metric(
            ui,
            self.ui_text("Transport Loss", "传输丢失"),
            snapshot.transport_loss_rate,
            format!("{:.2}%", snapshot.transport_loss_rate * 100.0),
            egui::Color32::from_rgb(255, 120, 117),
            layout.progress_bar_height,
        );
        draw_progress_metric(
            ui,
            self.ui_text("Clip Events", "削波次数"),
            (snapshot.clip_events as f32 / clip_max.max(1.0)).clamp(0.0, 1.0),
            snapshot.clip_events.to_string(),
            egui::Color32::from_rgb(250, 173, 20),
            layout.progress_bar_height,
        );

        ui.separator();
        egui::Grid::new("transport_counts_grid")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label(self.ui_text("Sent", "已发送"));
                ui.monospace(snapshot.sent_packets.to_string());
                ui.end_row();
                ui.label(self.ui_text("Received", "已接收"));
                ui.monospace(snapshot.received_packets.to_string());
                ui.end_row();
                ui.label(self.ui_text("Concealed", "已补偿"));
                ui.monospace(snapshot.concealed_packets.to_string());
                ui.end_row();
            });
    }

    fn draw_timing_panel(
        &self,
        ui: &mut egui::Ui,
        snapshot: &RuntimeSnapshot,
        layout: MetricsLayout,
    ) {
        let delay_history = self.metric_history_values(|entry| entry.coarse_delay_ms);
        let frame_time_history =
            self.metric_history_values(|entry| entry.processing_time_us as f32);
        let max_delay = max_series_value([&delay_history], 20.0);
        let max_frame_time = max_series_value([&frame_time_history], 2_000.0);

        draw_history_card(
            ui,
            self.ui_text("Delay History", "延迟历史"),
            &[HistoryLine {
                label: self.ui_text("Delay ms", "延迟 ms"),
                color: egui::Color32::from_rgb(64, 169, 255),
                values: &delay_history,
            }],
            0.0,
            max_delay,
            "ms",
            layout.chart_height,
            self.language,
        );

        draw_progress_metric(
            ui,
            self.ui_text("Coarse Delay", "粗延迟"),
            snapshot.coarse_delay_ms / max_delay.max(f32::EPSILON),
            format!("{:.2} ms", snapshot.coarse_delay_ms),
            egui::Color32::from_rgb(64, 169, 255),
            layout.progress_bar_height,
        );
        draw_progress_metric(
            ui,
            self.ui_text("Processing Time", "处理时间"),
            snapshot.processing_time_us as f32 / max_frame_time.max(f32::EPSILON),
            format!("{} us", snapshot.processing_time_us),
            egui::Color32::from_rgb(82, 196, 26),
            layout.progress_bar_height,
        );
        draw_history_card(
            ui,
            self.ui_text("Processing Cost", "处理耗时"),
            &[HistoryLine {
                label: self.ui_text("Frame us", "单帧 us"),
                color: egui::Color32::from_rgb(82, 196, 26),
                values: &frame_time_history,
            }],
            0.0,
            max_frame_time,
            "us",
            layout.chart_height,
            self.language,
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(self.ui_text("Drift", "漂移"));
            ui.monospace(format!("{:.2} ppm", snapshot.drift_ppm));
        });
    }

    fn metric_history_values(&self, map: impl Fn(&RuntimeSnapshot) -> f32) -> Vec<f32> {
        self.metric_history.iter().map(map).collect()
    }

    fn draw_recording_test_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.ui_text("Recording Test", "录制测试"));
        ui.small(self.ui_text(
            "Use this page to isolate whether noise comes from the live monitor output path or from the capture/DSP chain.",
            "用这个页面区分噪声究竟来自实时监听输出链路，还是来自采集 / DSP 主链。",
        ));
        if self.live_monitor_same_device_warning() {
            ui.add_space(6.0);
            ui.colored_label(
                egui::Color32::from_rgb(255, 120, 117),
                self.ui_text(
                    "Current live monitor routes processed audio back to the same headset family as the microphone. That often sounds metallic/electrical because you are monitoring your own voice with latency, even when the recorded WAV is already clean.",
                    "当前实时监听把处理后音频又送回了与麦克风同一耳机系列的输出端。这种情况下，即使录下来的 WAV 已经干净，实时监听仍很容易因为自听延迟而听起来发金属音 / 电流音。",
                ),
            );
        }
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            if ui
                .button(self.ui_text("Load Capture-To-WAV Preset", "加载仅录 WAV 预设"))
                .clicked()
            {
                self.load_selected_config_path("configs/node-a-wasapi-wav.toml".to_owned());
            }
            if ui
                .button(self.ui_text("Load Live Monitor Preset", "加载实时监听预设"))
                .clicked()
            {
                self.load_selected_config_path("configs/node-a.toml".to_owned());
            }
            if ui
                .button(self.ui_text("Load Mock Render Preset", "加载 Mock 输出预设"))
                .clicked()
            {
                self.load_selected_config_path("configs/node-a-mock-render.toml".to_owned());
            }
        });

        ui.add_space(8.0);
        ui.columns(2, |columns| {
            egui::Frame::group(columns[0].style()).show(&mut columns[0], |ui| {
                ui.label(self.ui_text("Current Recording Path", "当前录制路径"));
                ui.monospace(&self.config_path);
                if let Some(node_name) = &self.loaded_node_name {
                    ui.small(format!(
                        "{}: {node_name}",
                        self.ui_text("Node", "节点")
                    ));
                }
                if let Some(wav_path) = &self.loaded_wav_path {
                    ui.small(format!(
                        "{}: {wav_path}",
                        self.ui_text("Processed WAV", "处理后 WAV")
                    ));
                }
                if let Some(dump_dir) = &self.loaded_dump_dir {
                    ui.small(format!(
                        "{}: {dump_dir}",
                        self.ui_text("Debug Dump Dir", "调试输出目录")
                    ));
                }
            });

            egui::Frame::group(columns[1].style()).show(&mut columns[1], |ui| {
                ui.label(self.ui_text("Monitor Diagnosis", "监听诊断"));
                ui.small(self.ui_text(
                    "If `virtual_stub` is writing straight to speakers/headphones, static often points to the render path or acoustic monitoring loop rather than DSP failure.",
                    "如果 `virtual_stub` 直接写到扬声器 / 耳机，出现电流音通常更像是 render 路径或声学监听环路问题，而不是 DSP 主链本身失效。",
                ));
                ui.small(self.ui_text(
                    "First test `Capture-To-WAV`. If the WAV sounds clean but live monitor is noisy, the issue is in the monitor/output path.",
                    "建议先测“仅录 WAV”。如果导出的 WAV 干净但实时监听有噪声，问题基本就在监听 / 输出链路。",
                ));
            });
        });

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(self.ui_text("Current I/O", "当前输入输出"));
            ui.small(format!(
                "{}: {}",
                self.ui_text("Input Device", "输入设备"),
                self.input_device_value
            ));
            ui.small(format!(
                "{}: {}",
                self.ui_text("Output Target", "输出目标"),
                self.target_device_value
            ));
            ui.small(format!(
                "{}: {}",
                self.ui_text("Listen Address", "监听地址"),
                self.listen_addr_value
            ));
            ui.small(format!(
                "{}: {}",
                self.ui_text("Peer Address", "对端地址"),
                self.peer_addr_value
            ));
        });

        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(self.ui_text("Live Test Notes", "实时测试说明"));
            ui.small(self.ui_text(
                "1. Prefer headphones or a virtual cable when doing live monitor tests.",
                "1. 做实时监听测试时，优先使用耳机或虚拟声卡，不要直接外放。",
            ));
            ui.small(self.ui_text(
                "2. If clip events rise above zero in Metrics, lower the monitor volume first.",
                "2. 如果 Metrics 里的 clip events 大于 0，先降低监听音量。",
            ));
            ui.small(self.ui_text(
                "3. If you only need a recording proof, use the WAV preset instead of live monitor.",
                "3. 如果只是想确认录音链路是否正常，优先使用 WAV 预设，而不是实时监听。",
            ));
        });

        if let Some(snapshot) = &self.latest {
            ui.add_space(8.0);
            ui.columns(4, |columns| {
                draw_stat_card(
                    &mut columns[0],
                    self.ui_text("Input RMS", "输入 RMS"),
                    &format!("{:.5}", snapshot.input_rms),
                    self.ui_text("mic level", "麦克风电平"),
                    72.0,
                );
                draw_stat_card(
                    &mut columns[1],
                    self.ui_text("Output RMS", "输出 RMS"),
                    &format!("{:.5}", snapshot.output_rms),
                    self.ui_text("monitor level", "监听电平"),
                    72.0,
                );
                draw_stat_card(
                    &mut columns[2],
                    self.ui_text("Clip Events", "削波次数"),
                    &snapshot.clip_events.to_string(),
                    self.ui_text("current runtime", "当前运行时"),
                    72.0,
                );
                draw_stat_card(
                    &mut columns[3],
                    self.ui_text("Transport Loss", "传输丢失"),
                    &format!("{:.2}%", snapshot.transport_loss_rate * 100.0),
                    self.ui_text("reference health", "参考链路健康度"),
                    72.0,
                );
            });
        }
    }

    fn live_monitor_same_device_warning(&self) -> bool {
        self.loaded_audio_backend == Some(AudioBackend::Wasapi)
            && self.loaded_output_backend == Some(OutputBackend::VirtualStub)
            && device_family_hint(&self.input_device_value)
                .zip(device_family_hint(&self.target_device_value))
                .map(|(input_family, output_family)| input_family == output_family)
                .unwrap_or(false)
    }
}

impl eframe::App for NodeGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();
        ctx.request_repaint_after(Duration::from_millis(33));

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button(self.ui_text("Language", "语言"), |ui| {
                    for language in [UiLanguage::English, UiLanguage::Chinese] {
                        if ui
                            .selectable_label(self.language == language, language.label())
                            .clicked()
                        {
                            self.set_language(language);
                            ui.close();
                        }
                    }
                });
                ui.separator();
                ui.heading("EK Dual Mic");
                ui.label(self.ui_text("Windows-only realtime scaffold", "仅限 Windows 的实时框架"));
            });
        });

        egui::SidePanel::left("control_panel")
            .min_width(320.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                .id_salt("control_panel_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut capture_device_changed = false;
                    let mut render_device_changed = false;

                    ui.label(self.ui_text("Config Path", "配置路径"));
                    ui.text_edit_singleline(&mut self.config_path);
                        ui.horizontal(|ui| {
                            ui.menu_button(self.ui_text("Load Config", "加载配置"), |ui| {
                                let presets = self.config_presets.clone();
                                if presets.is_empty() {
                                    ui.small(self.ui_text(
                                        "No config presets found.",
                                        "未找到配置预设。",
                                    ));
                                } else {
                                    for preset in presets {
                                        if ui.button(&preset).clicked() {
                                            self.load_selected_config_path(preset);
                                            ui.close();
                                        }
                                    }
                                }

                                ui.separator();
                                if ui
                                    .button(self.ui_text(
                                        "Import Config Folder",
                                        "导入配置文件夹",
                                    ))
                                    .clicked()
                                {
                                    self.pick_and_import_config_folder();
                                    ui.close();
                                }
                                if ui
                                    .button(self.ui_text("Refresh Config List", "刷新配置列表"))
                                    .clicked()
                                {
                                    self.refresh_config_presets();
                                }
                            });
                            if ui
                                .button(self.ui_text("Refresh Configs", "刷新配置"))
                                .clicked()
                            {
                                self.refresh_config_presets();
                            }
                            if ui
                                .button(self.ui_text(
                                    "Import Config Folder",
                                    "导入配置文件夹",
                                ))
                                .clicked()
                            {
                                self.pick_and_import_config_folder();
                            }
                            if ui
                                .button(self.ui_text("Save Runtime Fields", "保存运行时字段"))
                                .clicked()
                            {
                                self.save_runtime_fields();
                            }
                        });
                        if let Some(error) = &self.config_discovery_error {
                            ui.small(format!(
                                "{}: {error}",
                                self.ui_text("Config discovery error", "配置发现错误")
                            ));
                        } else if !self.config_presets.is_empty() {
                            ui.small(format!(
                                "{}: {}",
                                self.ui_text("Known configs", "已发现配置"),
                                self.config_presets.join(", ")
                            ));
                        }
                        if let Some(summary) = &self.config_mode_summary {
                            ui.small(format!(
                                "{}: {summary}",
                                self.ui_text("Loaded mode", "已加载模式")
                            ));
                        }
                        if let Some(warning) = &self.config_mode_warning {
                            ui.colored_label(egui::Color32::from_rgb(250, 173, 20), warning);
                        }
                        ui.separator();

                        ui.label(self.ui_text("Local Listen Address", "本机监听地址"));
                        ui.add_enabled_ui(self.transport_selection_enabled(), |ui| {
                            ui.text_edit_singleline(&mut self.listen_addr_value);
                        });

                        ui.label(self.ui_text("Peer Address", "对端地址"));
                        ui.add_enabled_ui(self.transport_selection_enabled(), |ui| {
                            ui.text_edit_singleline(&mut self.peer_addr_value);
                        });
                        if !self.transport_selection_enabled() {
                            ui.small(self.ui_text(
                                "Ignored because this config uses the mock transport backend.",
                                "当前配置使用 mock 传输后端，因此该字段会被忽略。",
                            ));
                        } else {
                            ui.small(self.ui_text(
                                "For two devices on the same LAN, keep `0.0.0.0:38001` here and set the peer to the other device IP, for example `192.168.1.22:38001`.",
                                "两台设备在同一局域网时，这里保持 `0.0.0.0:38001`，并把对端地址设成另一台机器的 IP，例如 `192.168.1.22:38001`。",
                            ));
                        }

                        ui.label(self.ui_text("Audio Input Device", "音频输入设备"));
                        ui.add_enabled_ui(self.capture_selection_enabled(), |ui| {
                            capture_device_changed |= self.draw_capture_device_dropdown(ui);
                        });
                        if !self.capture_selection_enabled() {
                            ui.small(self.ui_text(
                                "Ignored because this config uses the mock audio backend.",
                                "当前配置使用 mock 音频后端，因此该字段会被忽略。",
                            ));
                        }

                        ui.label(self.ui_text("Output Target Device", "输出目标设备"));
                        ui.add_enabled_ui(self.render_selection_enabled(), |ui| {
                            render_device_changed |= self.draw_render_device_dropdown(ui);
                        });
                        if !self.render_selection_enabled() {
                            ui.small(self.ui_text(
                                "Ignored because this config writes to WAV/null instead of a live output endpoint.",
                                "当前配置写入的是 WAV/null，而不是实时输出端点，因此该字段会被忽略。",
                            ));
                        }

                        ui.separator();
                        ui.label(self.ui_text("Noise Reduction", "降噪控制"));
                        let noise_control_state = self.draw_noise_reduction_controls(ui);
                        if noise_control_state.apply_clicked {
                            self.save_runtime_fields();
                        }

                        ui.horizontal(|ui| {
                            if ui
                                .button(self.ui_text("Listen On All Interfaces", "监听所有网卡"))
                                .clicked()
                            {
                                self.listen_addr_value = socket_host_with_port(
                                    "0.0.0.0",
                                    &self.listen_addr_value,
                                    38001,
                                );
                            }
                            if ui.button(self.ui_text("Refresh Devices", "刷新设备")).clicked() {
                                self.refresh_device_lists();
                            }
                        });
                        if let Some(message) = &self.config_feedback {
                            ui.small(message);
                        }
                        ui.separator();

                        if self.worker.is_none() {
                            let start_label = egui::RichText::new(self.ui_text("Start", "启动"))
                                .size(18.0)
                                .font(egui::FontId::new(
                                    18.0,
                                    egui::FontFamily::Name(Arc::from(WINDOWS_UI_BOLD_FAMILY)),
                                ));
                            if ui
                                .add(
                                    egui::Button::new(start_label)
                                        .min_size(egui::vec2(120.0, 36.0)),
                                )
                                .clicked()
                            {
                                self.start();
                            }
                        } else {
                            ui.horizontal(|ui| {
                                if ui
                                    .button(self.ui_text("Reload Runtime", "重新加载运行时"))
                                    .clicked()
                                {
                                    self.request_runtime_reload();
                                }
                                if ui.button(self.ui_text("Stop", "停止")).clicked() {
                                    self.stop();
                                }
                            });
                        }

                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui
                                .button(self.ui_text("Refresh Configs", "刷新配置"))
                                .clicked()
                            {
                                self.refresh_config_presets();
                            }
                            if ui
                                .button(self.ui_text("Refresh Devices", "刷新设备"))
                                .clicked()
                            {
                                self.refresh_device_lists();
                            }
                        });
                        ui.label(format!(
                            "{}: {}",
                            self.ui_text("Status", "状态"),
                            self.status
                        ));
                        ui.small(
                            self.ui_text(
                                "WASAPI capture and render-endpoint bridge are available. Built-in virtual mic device creation is still not implemented.",
                                "WASAPI 采集和 render endpoint 输出桥接已可用，但内建虚拟麦设备创建仍未实现。",
                            ),
                        );

                        ui.separator();
                        ui.collapsing(self.ui_text("Capture Devices", "采集设备"), |ui| {
                            if self.capture_devices.is_empty() {
                                ui.small(self.ui_text(
                                    "No capture devices loaded.",
                                    "未加载到采集设备。",
                                ));
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
                                        capture_device_changed = true;
                                    }
                                    ui.small(format!("id: {}", device.id));
                                }
                            }
                        });
                        ui.collapsing(self.ui_text("Render Devices", "渲染设备"), |ui| {
                            if self.render_devices.is_empty() {
                                ui.small(self.ui_text(
                                    "No render devices loaded.",
                                    "未加载到渲染设备。",
                                ));
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
                                        render_device_changed = true;
                                    }
                                    ui.small(format!("id: {}", device.id));
                                }
                            }
                        });
                        if let Some(error) = &self.device_probe_error {
                            ui.separator();
                            ui.small(format!(
                                "{}: {error}",
                                self.ui_text("Device probe error", "设备探测错误")
                            ));
                        }

                        if capture_device_changed || render_device_changed {
                            self.handle_live_device_selection_change();
                        }
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("metrics_panel_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for tab in [MainTab::Metrics, MainTab::RecordingTest] {
                            ui.selectable_value(
                                &mut self.active_tab,
                                tab,
                                tab.label(self.language),
                            );
                        }
                    });
                    ui.separator();

                    match self.active_tab {
                        MainTab::Metrics => {
                            ui.horizontal_wrapped(|ui| {
                                ui.heading(self.ui_text("Realtime Metrics", "实时指标"));
                                ui.separator();
                                ui.label(self.ui_text("Metrics Size", "指标尺寸"));
                                for size in [
                                    MetricsPanelSize::Compact,
                                    MetricsPanelSize::Medium,
                                    MetricsPanelSize::Large,
                                ] {
                                    ui.selectable_value(
                                        &mut self.metrics_panel_size,
                                        size,
                                        size.label(self.language),
                                    );
                                }
                            });
                            ui.small(self.ui_text(
                                "Default small mode shows four metric panels per row.",
                                "默认小尺寸模式下一行显示 4 个指标面板。",
                            ));
                            ui.separator();

                            if let Some(snapshot) = &self.latest {
                                self.draw_metrics_dashboard(ui, snapshot);
                            } else {
                                ui.label(
                                    self.ui_text(
                                        "No runtime snapshot yet.",
                                        "当前还没有运行时快照。",
                                    ),
                                );
                            }
                        }
                        MainTab::RecordingTest => {
                            self.draw_recording_test_tab(ui);
                        }
                    }
                });
        });

        if let Some(preview) = self.pending_config_import.clone() {
            egui::Window::new(self.ui_text("Config Import Conflict", "配置导入冲突"))
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{} {}",
                        self.ui_text(
                            "The selected folder contains config files with the same name but different contents:",
                            "所选文件夹中包含文件名相同但内容不同的配置文件：",
                        ),
                        preview.source_dir.display()
                    ));
                    ui.add_space(6.0);
                    for conflict in &preview.conflicts {
                        ui.label(format!(
                            "{} {}",
                            self.ui_text("Source", "来源"),
                            conflict.source_name
                        ));
                        ui.small(format!(
                            "{} {}",
                            self.ui_text("Existing", "已存在"),
                            conflict.existing_path
                        ));
                        ui.small(format!(
                            "{} {}",
                            self.ui_text("Suggested rename", "建议重命名"),
                            conflict.suggested_path
                        ));
                        ui.separator();
                    }

                    if !preview.skipped_duplicates.is_empty() {
                        ui.small(format!(
                            "{} {}",
                            self.ui_text("Exact duplicates will be skipped", "完全重复项将被跳过"),
                            preview.skipped_duplicates.join(", ")
                        ));
                    }

                    ui.horizontal(|ui| {
                        if ui
                            .button(self.ui_text(
                                "Load And Rename Conflicts",
                                "加载并重命名冲突项",
                            ))
                            .clicked()
                        {
                            self.apply_config_import(preview.source_dir.clone(), true);
                        }
                        if ui
                            .button(self.ui_text("Cancel Import", "取消导入"))
                            .clicked()
                        {
                            self.pending_config_import = None;
                            self.config_feedback = Some(
                                self.ui_text(
                                    "Config folder import cancelled",
                                    "已取消配置文件夹导入",
                                )
                                .to_owned(),
                            );
                        }
                    });
                });
        }
    }
}

struct HistoryLine<'a> {
    label: &'a str,
    color: egui::Color32,
    values: &'a [f32],
}

fn localized<'a>(language: UiLanguage, english: &'a str, chinese: &'a str) -> &'a str {
    match language {
        UiLanguage::English => english,
        UiLanguage::Chinese => chinese,
    }
}

fn device_family_hint(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        return None;
    }

    let hint = trimmed
        .split_once('(')
        .and_then(|(_, tail)| tail.split_once(')'))
        .map(|(family, _)| family)
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_lowercase();
    if hint.is_empty() { None } else { Some(hint) }
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

fn socket_address_port(value: &str) -> Option<u16> {
    value
        .trim()
        .parse::<SocketAddr>()
        .ok()
        .map(|address| address.port())
}

fn socket_host_with_port(host: &str, current_value: &str, default_port: u16) -> String {
    let port = socket_address_port(current_value).unwrap_or(default_port);
    format!("{host}:{port}")
}

fn draw_stat_card(ui: &mut egui::Ui, title: &str, value: &str, subtitle: &str, min_height: f32) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_height(min_height);
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
    bar_height: f32,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(value_text);
        });
    });
    ui.add_sized(
        [safe_available_width(ui, 120.0), bar_height],
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
    chart_height: f32,
    language: UiLanguage,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(title);
        ui.small(format!(
            "{} {METRIC_HISTORY_LIMIT} {}, {}: {unit}",
            localized(language, "Recent", "最近"),
            localized(language, "frames", "帧"),
            localized(language, "unit", "单位"),
        ));
        ui.horizontal_wrapped(|ui| {
            for line in lines {
                ui.colored_label(line.color, line.label);
            }
        });

        let desired_size = egui::vec2(safe_available_width(ui, 120.0), chart_height);
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
    let regular_paths = find_windows_cjk_font_chain(false);
    if regular_paths.is_empty() {
        warn!("no Windows CJK system font fallback found; Chinese text may render as tofu");
        return;
    }
    let bold_paths = find_windows_cjk_font_chain(true);

    let mut fonts = egui::FontDefinitions::default();
    let regular_font_names =
        register_windows_font_chain(&mut fonts, WINDOWS_UI_REGULAR_FAMILY, &regular_paths);
    let bold_font_names = register_windows_font_chain(
        &mut fonts,
        WINDOWS_UI_BOLD_FAMILY,
        if bold_paths.is_empty() {
            &regular_paths
        } else {
            &bold_paths
        },
    );
    if regular_font_names.is_empty() || bold_font_names.is_empty() {
        warn!("failed to load any Windows system UI fonts; Chinese text may render as tofu");
        return;
    }

    fonts.families.insert(
        egui::FontFamily::Name(Arc::from(WINDOWS_UI_REGULAR_FAMILY)),
        regular_font_names.clone(),
    );
    fonts.families.insert(
        egui::FontFamily::Name(Arc::from(WINDOWS_UI_BOLD_FAMILY)),
        bold_font_names.clone(),
    );
    prepend_font_names(
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default(),
        &regular_font_names,
    );
    prepend_font_names(
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default(),
        &regular_font_names,
    );
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    remap_text_style_family(&mut style, egui::TextStyle::Small, WINDOWS_UI_REGULAR_FAMILY);
    remap_text_style_family(&mut style, egui::TextStyle::Body, WINDOWS_UI_REGULAR_FAMILY);
    remap_text_style_family(&mut style, egui::TextStyle::Button, WINDOWS_UI_REGULAR_FAMILY);
    remap_text_style_family(&mut style, egui::TextStyle::Heading, WINDOWS_UI_REGULAR_FAMILY);
    remap_text_style_family(&mut style, egui::TextStyle::Monospace, WINDOWS_UI_REGULAR_FAMILY);
    ctx.set_style(style);

    info!(
        regular = ?regular_paths,
        bold = ?bold_paths,
        "installed Windows system font chain"
    );
}

fn register_windows_font_chain(
    fonts: &mut egui::FontDefinitions,
    family_prefix: &str,
    paths: &[PathBuf],
) -> Vec<String> {
    let mut font_names = Vec::new();
    for (index, font_path) in paths.iter().enumerate() {
        let font_bytes = match fs::read(font_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(
                    path = %font_path.display(),
                    %error,
                    "failed to read Windows system font"
                );
                continue;
            }
        };
        let font_name = format!(
            "{family_prefix}_{index}_{}",
            font_path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("fallback")
        );
        fonts.font_data.insert(
            font_name.clone(),
            egui::FontData::from_owned(font_bytes).into(),
        );
        font_names.push(font_name);
    }
    font_names
}

fn prepend_font_names(target: &mut Vec<String>, preferred_names: &[String]) {
    for name in preferred_names.iter().rev() {
        if !target.iter().any(|existing| existing == name) {
            target.insert(0, name.clone());
        }
    }
}

fn remap_text_style_family(
    style: &mut egui::Style,
    text_style: egui::TextStyle,
    family_name: &str,
) {
    if let Some(current) = style.text_styles.get(&text_style).cloned() {
        style.text_styles.insert(
            text_style,
            egui::FontId::new(current.size, egui::FontFamily::Name(Arc::from(family_name))),
        );
    }
}

fn find_windows_cjk_font_chain(bold: bool) -> Vec<PathBuf> {
    let windows_dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let fonts_dir = windows_dir.join("Fonts");

    windows_cjk_font_candidates(&fonts_dir, bold)
        .into_iter()
        .filter(|path| path.is_file())
        .collect()
}

fn windows_cjk_font_candidates(fonts_dir: &Path, bold: bool) -> Vec<PathBuf> {
    let names: &[&str] = if bold {
        &[
            "msyhbd.ttc",
            "NotoSansSC-VF.ttf",
            "simhei.ttf",
            "simsunb.ttf",
            "msyh.ttc",
            "simsun.ttc",
        ]
    } else {
        &[
            "NotoSansSC-VF.ttf",
            "msyh.ttc",
            "simhei.ttf",
            "simsun.ttc",
            "NotoSerifSC-VF.ttf",
            "simfang.ttf",
            "simkai.ttf",
            "simsunb.ttf",
        ]
    };
    names.iter().map(|name| fonts_dir.join(name)).collect()
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
        config.node.listen_addr = "0.0.0.0:38101".to_owned();
        config.node.peer_addr = "192.168.1.22:38101".to_owned();
        config.audio.input_device = "default".to_owned();
        config.output.target_device = "Speaker A".to_owned();
        config.output.monitor_processed_output = false;
        config.cancel.step_size = 0.08;
        config.cancel.update_threshold = 0.41;
        config.cancel.anti_phase_enabled = false;
        config.cancel.anti_phase_max_gain = 1.72;
        config.cancel.anti_phase_smoothing = 0.55;
        config.residual.enabled = false;
        config.residual.strength = 0.91;
        save_config(&path, &config).expect("load-config test config should save");

        let mut app = test_app(path.as_path());
        app.listen_addr_value = "stale-listen".to_owned();
        app.peer_addr_value = "stale-peer".to_owned();
        app.input_device_value = "stale-input".to_owned();
        app.target_device_value = "stale-output".to_owned();

        app.reload_config_fields();

        assert_eq!(app.listen_addr_value, "0.0.0.0:38101");
        assert_eq!(app.peer_addr_value, "192.168.1.22:38101");
        assert_eq!(app.input_device_value, "default");
        assert_eq!(app.target_device_value, "Speaker A");
        assert!(!app.monitor_processed_output_value);
        assert!((app.cancel_step_size_value - 0.08).abs() < f32::EPSILON);
        assert!((app.cancel_update_threshold_value - 0.41).abs() < f32::EPSILON);
        assert!(!app.anti_phase_enabled_value);
        assert!((app.anti_phase_max_gain_value - 1.72).abs() < f32::EPSILON);
        assert!((app.anti_phase_smoothing_value - 0.55).abs() < f32::EPSILON);
        assert!(!app.residual_enabled_value);
        assert!((app.residual_strength_value - 0.91).abs() < f32::EPSILON);
        assert_eq!(
            app.config_feedback.as_deref(),
            Some(
                format!(
                    "{} {}",
                    app.ui_text("Config loaded from", "已从以下路径加载配置："),
                    path.display()
                )
                .as_str()
            )
        );
        assert_eq!(
            app.status,
            format!(
                "{}: {}",
                app.ui_text("Config loaded", "配置已加载"),
                path.display()
            )
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persist_ui_runtime_fields_to_config_writes_selected_values() {
        let path = unique_test_config_path("persist-runtime");
        save_config(&path, &test_config("persist-runtime"))
            .expect("persist-runtime test config should save");

        let mut app = test_app(path.as_path());
        app.listen_addr_value = " 0.0.0.0:39001 ".to_owned();
        app.peer_addr_value = " 192.168.1.22 ".to_owned();
        app.input_device_value = " default ".to_owned();
        app.target_device_value = " Speaker B ".to_owned();
        app.monitor_processed_output_value = false;
        app.cancel_step_size_value = 0.09;
        app.cancel_update_threshold_value = 0.37;
        app.anti_phase_enabled_value = false;
        app.anti_phase_max_gain_value = 1.64;
        app.anti_phase_smoothing_value = 0.52;
        app.residual_enabled_value = false;
        app.residual_strength_value = 0.88;

        assert!(
            app.persist_ui_runtime_fields_to_config()
                .expect("runtime field persistence should succeed"),
            "runtime field persistence should report a config change"
        );

        let reloaded = load_config(&path).expect("persisted config should reload");
        assert_eq!(reloaded.node.listen_addr, "0.0.0.0:39001");
        assert_eq!(reloaded.node.peer_addr, "192.168.1.22:38002");
        assert_eq!(reloaded.audio.input_device, "default");
        assert_eq!(reloaded.output.target_device, "Speaker B");
        assert!(!reloaded.output.monitor_processed_output);
        assert!((reloaded.cancel.step_size - 0.09).abs() < f32::EPSILON);
        assert!((reloaded.cancel.update_threshold - 0.37).abs() < f32::EPSILON);
        assert!(!reloaded.cancel.anti_phase_enabled);
        assert!((reloaded.cancel.anti_phase_max_gain - 1.64).abs() < f32::EPSILON);
        assert!((reloaded.cancel.anti_phase_smoothing - 0.52).abs() < f32::EPSILON);
        assert!(!reloaded.residual.enabled);
        assert!((reloaded.residual.strength - 0.88).abs() < f32::EPSILON);

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

    #[test]
    fn noise_reduction_controls_render_with_interactive_widgets() {
        let path = unique_test_config_path("noise-controls-render");
        let mut app = test_app(path.as_path());
        let ctx = egui::Context::default();
        let state = render_noise_controls_frame(&ctx, &mut app, Vec::new());

        assert!(state.monitor_processed_rect.is_some());
        assert!(state.update_threshold_rect.is_some());
        assert!(state.anti_phase_depth_rect.is_some());
        assert!(state.residual_strength_rect.is_some());
    }

    #[test]
    fn noise_reduction_controls_accept_pointer_interaction() {
        let path = unique_test_config_path("noise-controls-interact");
        let mut app = test_app(path.as_path());
        let ctx = egui::Context::default();
        let initial = render_noise_controls_frame(&ctx, &mut app, Vec::new());

        let monitor_center = initial
            .monitor_processed_rect
            .expect("monitor checkbox should render")
            .center();
        click_pointer(&ctx, &mut app, monitor_center);
        assert!(
            !app.monitor_processed_output_value,
            "monitor checkbox click should toggle processed monitor"
        );

        let update_rect = initial
            .update_threshold_rect
            .expect("update threshold slider should render");
        let initial_update_threshold = app.cancel_update_threshold_value;
        drag_pointer(
            &ctx,
            &mut app,
            update_rect.center(),
            egui::pos2(update_rect.left() + 4.0, update_rect.center().y),
        );
        assert!(
            app.cancel_update_threshold_value < initial_update_threshold,
            "dragging update threshold left should reduce the value"
        );

        let anti_phase_rect = initial
            .anti_phase_depth_rect
            .expect("anti-phase depth slider should render");
        let initial_anti_phase_depth = app.anti_phase_max_gain_value;
        drag_pointer(
            &ctx,
            &mut app,
            anti_phase_rect.center(),
            egui::pos2(anti_phase_rect.right() - 4.0, anti_phase_rect.center().y),
        );
        assert!(
            app.anti_phase_max_gain_value > initial_anti_phase_depth,
            "dragging anti-phase depth right should increase the value"
        );

        let residual_rect = initial
            .residual_strength_rect
            .expect("residual strength slider should render");
        let initial_residual_strength = app.residual_strength_value;
        drag_pointer(
            &ctx,
            &mut app,
            residual_rect.center(),
            egui::pos2(residual_rect.right() - 4.0, residual_rect.center().y),
        );
        assert!(
            app.residual_strength_value > initial_residual_strength,
            "dragging residual strength right should increase the value"
        );
    }

    #[test]
    fn metrics_panel_size_defaults_to_small_four_up_layout() {
        let path = unique_test_config_path("metrics-layout");
        let app = test_app(path.as_path());
        let layout = app.metrics_layout();

        assert_eq!(layout.panel_columns, 4);
        assert_eq!(layout.stat_columns, 4);
        assert_eq!(app.metrics_panel_size, MetricsPanelSize::Compact);
    }

    #[test]
    fn ui_language_defaults_to_chinese_and_translates_core_labels() {
        let path = unique_test_config_path("ui-language");
        let app = test_app(path.as_path());

        assert_eq!(app.language, UiLanguage::Chinese);
        assert_eq!(app.ui_text("Start", "启动"), "启动");
        assert_eq!(localized(UiLanguage::Chinese, "Language", "语言"), "语言");
        assert_eq!(MetricsPanelSize::Compact.label(UiLanguage::Chinese), "小");
    }

    #[test]
    fn device_family_hint_matches_headset_style_names() {
        assert_eq!(
            device_family_hint("Microphone (PRO X 2 LIGHTSPEED)").as_deref(),
            Some("pro x 2 lightspeed")
        );
        assert_eq!(
            device_family_hint("扬声器 (PRO X 2 LIGHTSPEED)").as_deref(),
            Some("pro x 2 lightspeed")
        );
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
            language: UiLanguage::Chinese,
            config_path: config_path.display().to_string(),
            config_presets: vec![config_path.display().to_string()],
            status: "Idle".to_owned(),
            active_tab: MainTab::Metrics,
            latest: None,
            metric_history: VecDeque::with_capacity(METRIC_HISTORY_LIMIT),
            metrics_panel_size: MetricsPanelSize::Compact,
            worker: None,
            capture_devices: Vec::new(),
            render_devices: Vec::new(),
            listen_addr_value: String::new(),
            peer_addr_value: String::new(),
            input_device_value: String::new(),
            target_device_value: String::new(),
            monitor_processed_output_value: true,
            cancel_step_size_value: 0.06,
            cancel_update_threshold_value: 0.48,
            anti_phase_enabled_value: true,
            anti_phase_max_gain_value: 1.45,
            anti_phase_smoothing_value: 0.72,
            residual_enabled_value: true,
            residual_strength_value: 0.72,
            config_feedback: None,
            config_discovery_error: None,
            pending_config_import: None,
            config_mode_summary: None,
            config_mode_warning: None,
            loaded_node_name: None,
            loaded_dump_dir: None,
            loaded_wav_path: None,
            loaded_audio_backend: None,
            loaded_transport_backend: None,
            loaded_output_backend: None,
            device_probe_error: None,
        }
    }

    fn render_noise_controls_frame(
        ctx: &egui::Context,
        app: &mut NodeGuiApp,
        events: Vec<egui::Event>,
    ) -> NoiseControlUiState {
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 2200.0),
            )),
            events,
            ..Default::default()
        };

        let mut state = NoiseControlUiState::default();
        let _ = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                state = app.draw_noise_reduction_controls(ui);
            });
        });
        state
    }

    fn click_pointer(ctx: &egui::Context, app: &mut NodeGuiApp, position: egui::Pos2) {
        let events = vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        let _ = render_noise_controls_frame(ctx, app, events);
    }

    fn drag_pointer(
        ctx: &egui::Context,
        app: &mut NodeGuiApp,
        start: egui::Pos2,
        end: egui::Pos2,
    ) {
        let _ = render_noise_controls_frame(
            ctx,
            app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let _ = render_noise_controls_frame(ctx, app, vec![egui::Event::PointerMoved(end)]);
        let _ = render_noise_controls_frame(
            ctx,
            app,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
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
