use anyhow::{Context, Result, bail};
use common_types::{CHANNELS, FRAME_MS, NodeConfig, SAMPLE_RATE_HZ};
use std::fs;
use std::path::Path;
use tracing_subscriber::EnvFilter;

pub fn load_config(path: impl AsRef<Path>) -> Result<NodeConfig> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: NodeConfig =
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_config(&config)?;
    Ok(config)
}

pub fn validate_config(config: &NodeConfig) -> Result<()> {
    if config.audio.sample_rate != SAMPLE_RATE_HZ {
        bail!(
            "sample_rate must stay at {SAMPLE_RATE_HZ}, got {}",
            config.audio.sample_rate
        );
    }

    if config.audio.channels as usize != CHANNELS {
        bail!(
            "channels must stay at {CHANNELS}, got {}",
            config.audio.channels
        );
    }

    if config.audio.frame_ms as usize != FRAME_MS {
        bail!(
            "frame_ms must stay at {FRAME_MS}, got {}",
            config.audio.frame_ms
        );
    }

    if config.cancel.filter_length == 0 {
        bail!("cancel.filter_length must be > 0");
    }

    Ok(())
}

pub fn init_logging(level: &str) -> Result<()> {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}
