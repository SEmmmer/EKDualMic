use anyhow::{Context, Result, bail};
use common_types::{CHANNELS, FRAME_MS, NodeConfig, SAMPLE_RATE_HZ};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

static LOG_SINK: OnceLock<LogSink> = OnceLock::new();
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

pub fn load_config(path: impl AsRef<Path>) -> Result<NodeConfig> {
    let path = resolve_config_path(path);
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: NodeConfig =
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_config(&config)?;
    Ok(config)
}

pub fn save_config(path: impl AsRef<Path>, config: &NodeConfig) -> Result<()> {
    let path = resolve_config_path(path);
    validate_config(config)?;

    let serialized = toml::to_string_pretty(config)
        .with_context(|| format!("failed to serialize config {}", path.display()))?;
    fs::write(&path, serialized).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn resolve_config_path(path: impl AsRef<Path>) -> PathBuf {
    resolve_config_path_from(path.as_ref(), &workspace_search_roots())
}

pub fn discover_config_presets() -> Result<Vec<String>> {
    let workspace_root = find_workspace_root()
        .context("failed to locate workspace root for config preset discovery")?;
    discover_config_presets_from(&workspace_root)
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
    let sink = shared_log_sink()?;
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .with_writer(sink.writer.clone())
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
    install_panic_hook(&sink.path);
    info!(path = %sink.path.display(), "file logging initialized");
    Ok(())
}

#[derive(Clone)]
struct FileLogWriter {
    file: Arc<Mutex<File>>,
}

impl<'a> MakeWriter<'a> for FileLogWriter {
    type Writer = LockedFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LockedFileWriter {
            file: Arc::clone(&self.file),
        }
    }
}

struct LockedFileWriter {
    file: Arc<Mutex<File>>,
}

impl Write for LockedFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file
            .lock()
            .expect("log file mutex should not be poisoned")
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file
            .lock()
            .expect("log file mutex should not be poisoned")
            .flush()
    }
}

struct LogSink {
    writer: FileLogWriter,
    path: PathBuf,
}

fn create_log_sink() -> Result<LogSink> {
    let log_dir = log_directory();
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log directory {}", log_dir.display()))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = log_dir.join(format!("app-{}-{}.log", std::process::id(), timestamp));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open log file {}", path.display()))?;

    Ok(LogSink {
        writer: FileLogWriter {
            file: Arc::new(Mutex::new(file)),
        },
        path,
    })
}

fn shared_log_sink() -> Result<&'static LogSink> {
    if let Some(sink) = LOG_SINK.get() {
        return Ok(sink);
    }

    let sink = create_log_sink()?;
    let _ = LOG_SINK.set(sink);
    Ok(LOG_SINK
        .get()
        .expect("log sink should be available after initialization"))
}

fn install_panic_hook(log_path: &Path) {
    let path = log_path.to_path_buf();
    let _ = PANIC_HOOK_INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(file, "panic: {panic_info}");
                let _ = writeln!(
                    file,
                    "backtrace:\n{}",
                    std::backtrace::Backtrace::force_capture()
                );
            }
            previous(panic_info);
        }));
    });
}

fn log_directory() -> PathBuf {
    find_workspace_root()
        .map(|root| root.join("logs"))
        .or_else(|| std::env::current_dir().ok().map(|dir| dir.join("logs")))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

fn resolve_config_path_from(path: &Path, search_roots: &[PathBuf]) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if path.exists() {
        return path.to_path_buf();
    }

    for root in search_roots {
        for ancestor in root.ancestors() {
            let candidate = ancestor.join(path);
            if candidate.exists() {
                return candidate;
            }
        }
    }

    workspace_root_from_search_roots(search_roots)
        .map(|root| root.join(path))
        .or_else(|| search_roots.first().map(|root| root.join(path)))
        .unwrap_or_else(|| path.to_path_buf())
}

fn discover_config_presets_from(workspace_root: &Path) -> Result<Vec<String>> {
    let configs_dir = workspace_root.join("configs");
    let entries = fs::read_dir(&configs_dir).with_context(|| {
        format!(
            "failed to read config preset directory {}",
            configs_dir.display()
        )
    })?;

    let mut presets = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read an entry from config preset directory {}",
                configs_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }

        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            presets.push(format!("configs/{name}"));
        }
    }

    presets.sort();
    Ok(presets)
}

fn find_workspace_root() -> Option<PathBuf> {
    workspace_root_from_search_roots(&workspace_search_roots())
}

fn workspace_root_from_search_roots(search_roots: &[PathBuf]) -> Option<PathBuf> {
    for start in search_roots {
        for candidate in start.ancestors() {
            if looks_like_workspace_root(candidate) {
                return Some(candidate.to_path_buf());
            }
        }
    }
    None
}

fn workspace_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(current_dir) = std::env::current_dir() {
        roots.push(current_dir);
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }

    roots
}

fn looks_like_workspace_root(path: &Path) -> bool {
    path.join("Cargo.toml").is_file()
        && path.join("configs").is_dir()
        && path.join("crates").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn save_and_reload_config_round_trip_preserves_device_fields() {
        let mut config = NodeConfig::default();
        config.audio.input_device = "default".to_owned();
        config.output.target_device = "CABLE Input (VB-Audio Virtual Cable)".to_owned();

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ek_dual_mic_config_{unique}.toml"));

        save_config(&path, &config).expect("config should save");
        let reloaded = load_config(&path).expect("config should reload");
        std::fs::remove_file(&path).expect("temp config should be removable");

        assert_eq!(reloaded.audio.input_device, "default");
        assert_eq!(
            reloaded.output.target_device,
            "CABLE Input (VB-Audio Virtual Cable)"
        );
    }

    #[test]
    fn resolve_config_path_from_workspace_root_for_relative_paths() {
        let workspace = unique_temp_dir("config_resolve");
        let configs_dir = workspace.join("configs");
        let search_root = workspace.join("target").join("debug");
        fs::create_dir_all(&configs_dir).expect("configs dir should create");
        fs::create_dir_all(workspace.join("crates")).expect("crates dir should create");
        fs::create_dir_all(&search_root).expect("search root should create");
        fs::write(workspace.join("Cargo.toml"), "[workspace]\n").expect("Cargo.toml should write");
        fs::write(configs_dir.join("node-a.toml"), "").expect("config file should write");

        let resolved = resolve_config_path_from(Path::new("configs/node-a.toml"), &[search_root]);
        assert_eq!(resolved, workspace.join("configs").join("node-a.toml"));

        fs::remove_dir_all(workspace).expect("temp workspace should be removable");
    }

    #[test]
    fn discover_config_presets_from_workspace_lists_toml_files() {
        let workspace = unique_temp_dir("config_presets");
        let configs_dir = workspace.join("configs");
        fs::create_dir_all(&configs_dir).expect("configs dir should create");
        fs::create_dir_all(workspace.join("crates")).expect("crates dir should create");
        fs::write(workspace.join("Cargo.toml"), "[workspace]\n").expect("Cargo.toml should write");
        fs::write(configs_dir.join("z.toml"), "").expect("z preset should write");
        fs::write(configs_dir.join("a.toml"), "").expect("a preset should write");
        fs::write(configs_dir.join("ignore.txt"), "").expect("ignore file should write");

        let presets = discover_config_presets_from(&workspace)
            .expect("config preset discovery should succeed");
        assert_eq!(presets, vec!["configs/a.toml", "configs/z.toml"]);

        fs::remove_dir_all(workspace).expect("temp workspace should be removable");
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ek_dual_mic_{label}_{unique}"))
    }
}
