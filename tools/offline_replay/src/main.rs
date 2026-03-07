use anyhow::Result;
use app::{PipelineRuntime, init_logging, load_config};
use common_types::{AudioBackend, OutputBackend, TransportBackend};
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let config_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "configs/node-a.toml".to_owned());
    let frames = args
        .get(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(600);

    let mut config = load_config(&config_path)?;
    config.audio.backend = AudioBackend::Mock;
    config.node.transport_backend = TransportBackend::Mock;
    config.output.backend = OutputBackend::WavDump;
    config.output.wav_path = PathBuf::from("artifacts/offline/processed-output.wav");
    config.debug.dump_wav = true;
    config.debug.dump_metrics = true;
    config.debug.dump_dir = PathBuf::from("artifacts/offline");

    init_logging(&config.debug.log_level)?;

    let mut runtime = PipelineRuntime::new(config.clone())?;
    for _ in 0..frames {
        runtime.step()?;
    }
    let snapshot = runtime.last_snapshot().clone();
    runtime.shutdown()?;

    println!(
        "offline replay finished: frames={}, output_rms={:.5}, coherence={:.3}, dump_dir={}",
        frames,
        snapshot.output_rms,
        snapshot.coherence,
        config.debug.dump_dir.display()
    );

    Ok(())
}
