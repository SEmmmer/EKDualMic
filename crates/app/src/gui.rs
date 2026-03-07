use crate::{PipelineRuntime, init_logging, load_config};
use common_types::RuntimeSnapshot;
use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub fn run_native() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1120.0, 760.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EK Dual Mic",
        options,
        Box::new(|_cc| Ok(Box::new(NodeGuiApp::default()))),
    )
}

struct WorkerHandle {
    stop: Arc<AtomicBool>,
    rx: Receiver<WorkerEvent>,
    join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

enum WorkerEvent {
    Snapshot(RuntimeSnapshot),
    Error(String),
    Stopped,
}

pub struct NodeGuiApp {
    config_path: String,
    status: String,
    latest: Option<RuntimeSnapshot>,
    worker: Option<WorkerHandle>,
}

impl Default for NodeGuiApp {
    fn default() -> Self {
        Self {
            config_path: "configs/node-a.toml".to_owned(),
            status: "Idle".to_owned(),
            latest: None,
            worker: None,
        }
    }
}

impl NodeGuiApp {
    fn start(&mut self) {
        if self.worker.is_some() {
            return;
        }

        let config_path = self.config_path.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let (tx, rx) = unbounded();

        let join = thread::spawn(move || {
            let _ = init_logging("info");

            let config = match load_config(&config_path) {
                Ok(config) => config,
                Err(error) => {
                    let _ = tx.send(WorkerEvent::Error(error.to_string()));
                    return;
                }
            };

            let frame_sleep = Duration::from_millis(config.audio.frame_ms as u64);
            let mut runtime = match PipelineRuntime::new(config.clone()) {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = tx.send(WorkerEvent::Error(error.to_string()));
                    return;
                }
            };

            while !stop_worker.load(Ordering::Relaxed) {
                match runtime.step() {
                    Ok(snapshot) => {
                        if tx.send(WorkerEvent::Snapshot(snapshot)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(WorkerEvent::Error(error.to_string()));
                        break;
                    }
                }

                thread::sleep(frame_sleep);
            }

            let _ = runtime.shutdown();
            let _ = tx.send(WorkerEvent::Stopped);
        });

        self.status = "Running".to_owned();
        self.worker = Some(WorkerHandle {
            stop,
            rx,
            join: Some(join),
        });
    }

    fn stop(&mut self) {
        if let Some(worker) = self.worker.as_mut() {
            worker.stop();
        }
        self.worker = None;
        self.status = "Stopped".to_owned();
    }

    fn poll_worker(&mut self) {
        let mut should_clear_worker = false;

        if let Some(worker) = self.worker.as_ref() {
            for event in worker.rx.try_iter() {
                match event {
                    WorkerEvent::Snapshot(snapshot) => {
                        self.status = format!("Running: frame {}", snapshot.sequence);
                        self.latest = Some(snapshot);
                    }
                    WorkerEvent::Error(message) => {
                        self.status = format!("Error: {message}");
                        should_clear_worker = true;
                    }
                    WorkerEvent::Stopped => {
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
                ui.label("Config Path");
                ui.text_edit_singleline(&mut self.config_path);
                ui.separator();

                if self.worker.is_none() {
                    if ui.button("Start").clicked() {
                        self.start();
                    }
                } else if ui.button("Stop").clicked() {
                    self.stop();
                }

                ui.separator();
                ui.label(format!("Status: {}", self.status));
                ui.small(
                    "WASAPI capture and virtual-mic output are still scaffold interfaces in this phase.",
                );
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Realtime Metrics");
            ui.separator();

            if let Some(snapshot) = &self.latest {
                egui::Grid::new("metrics_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Node");
                        ui.monospace(&snapshot.node_name);
                        ui.end_row();

                        ui.label("Sequence");
                        ui.monospace(snapshot.sequence.to_string());
                        ui.end_row();

                        ui.label("Coarse Delay");
                        ui.monospace(format!("{:.2} ms", snapshot.coarse_delay_ms));
                        ui.end_row();

                        ui.label("Coherence");
                        ui.monospace(format!("{:.3}", snapshot.coherence));
                        ui.end_row();

                        ui.label("Local VAD");
                        ui.monospace(format!("{:.3}", snapshot.local_vad.score));
                        ui.end_row();

                        ui.label("Peer VAD");
                        ui.monospace(format!("{:.3}", snapshot.peer_vad.score));
                        ui.end_row();

                        ui.label("Update Frozen");
                        ui.monospace(snapshot.update_frozen.to_string());
                        ui.end_row();

                        ui.label("Transport Loss");
                        ui.monospace(format!("{:.2}%", snapshot.transport_loss_rate * 100.0));
                        ui.end_row();

                        ui.label("Input RMS");
                        ui.monospace(format!("{:.5}", snapshot.input_rms));
                        ui.end_row();

                        ui.label("Output RMS");
                        ui.monospace(format!("{:.5}", snapshot.output_rms));
                        ui.end_row();

                        ui.label("Estimated Crosstalk RMS");
                        ui.monospace(format!("{:.5}", snapshot.estimated_crosstalk_rms));
                        ui.end_row();

                        ui.label("Frame Time");
                        ui.monospace(format!("{} us", snapshot.processing_time_us));
                        ui.end_row();
                    });
            } else {
                ui.label("No runtime snapshot yet.");
            }
        });
    }
}
