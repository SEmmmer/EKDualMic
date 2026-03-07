use anyhow::Result;
use app::{PipelineRuntime, init_logging, load_config};
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let config_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "configs/node-a.toml".to_owned());
    let frames = args
        .get(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(120);

    let config = load_config(&config_path)?;
    init_logging(&config.debug.log_level)?;

    let mut runtime = PipelineRuntime::new(config.clone())?;
    for _ in 0..frames {
        runtime.step()?;
    }

    let snapshot = runtime.last_snapshot().clone();
    runtime.shutdown()?;

    println!(
        "runtime smoke finished: node={}, frames={}, input_rms={:.5}, output_rms={:.5}, coherence={:.3}, dump_dir={}",
        snapshot.node_name,
        frames,
        snapshot.input_rms,
        snapshot.output_rms,
        snapshot.coherence,
        config.debug.dump_dir.display()
    );

    Ok(())
}
