use anyhow::{Context, Result, bail};
use common_types::{
    CHANNELS, FRAME_MS, NodeConfig, NodeRole, OutputBackend, OutputRoutingMode,
    SAMPLE_RATE_HZ, SessionMode, TransportBackend,
};
use std::collections::BTreeMap;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

static LOG_SINK: OnceLock<LogSink> = OnceLock::new();
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigImportConflict {
    pub source_name: String,
    pub existing_path: String,
    pub suggested_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigImportPreview {
    pub source_dir: PathBuf,
    pub discovered_files: Vec<String>,
    pub skipped_duplicates: Vec<String>,
    pub conflicts: Vec<ConfigImportConflict>,
    pub importable_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigImportResult {
    pub imported_paths: Vec<String>,
    pub skipped_duplicates: Vec<String>,
    pub skipped_conflicts: Vec<String>,
    pub renamed_imports: Vec<(String, String)>,
}

pub fn load_config(path: impl AsRef<Path>) -> Result<NodeConfig> {
    load_config_from(path.as_ref(), &workspace_search_roots())
}

pub fn save_config(path: impl AsRef<Path>, config: &NodeConfig) -> Result<()> {
    let path = resolve_config_path(path);
    validate_config(config)?;

    let serialized = toml::to_string_pretty(config)
        .with_context(|| format!("failed to serialize config {}", path.display()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    fs::write(&path, serialized).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn resolve_config_path(path: impl AsRef<Path>) -> PathBuf {
    resolve_config_path_from(path.as_ref(), &workspace_search_roots())
}

pub fn discover_config_presets() -> Result<Vec<String>> {
    if let Some(workspace_root) = find_workspace_root() {
        let mut presets = discover_config_presets_from(&workspace_root)?;
        merge_embedded_presets(&mut presets);
        return Ok(presets);
    }

    Ok(embedded_config_preset_names())
}

pub fn preview_import_config_directory(path: impl AsRef<Path>) -> Result<ConfigImportPreview> {
    preview_import_config_directory_from(path.as_ref(), &workspace_search_roots())
}

pub fn import_config_directory(
    path: impl AsRef<Path>,
    import_conflicts_with_rename: bool,
) -> Result<ConfigImportResult> {
    import_config_directory_from(
        path.as_ref(),
        import_conflicts_with_rename,
        &workspace_search_roots(),
    )
}

pub fn validate_config(config: &NodeConfig) -> Result<()> {
    if config.node.transport_backend == TransportBackend::Udp {
        let listen_addr: SocketAddr = config.node.listen_addr.parse().with_context(|| {
            format!(
                "node.listen_addr must be a valid IP:port for udp transport, got `{}`",
                config.node.listen_addr
            )
        })?;
        if listen_addr.port() == 0 {
            bail!("node.listen_addr must use a non-zero port");
        }

        let peer_addr: SocketAddr = config.node.peer_addr.parse().with_context(|| {
            format!(
                "node.peer_addr must be a valid IP:port for udp transport, got `{}`",
                config.node.peer_addr
            )
        })?;
        if peer_addr.port() == 0 {
            bail!("node.peer_addr must use a non-zero port");
        }
    }

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

    validate_pairing_mode(config)?;
    validate_output_routing(config)?;

    if config.cancel.anti_phase_max_gain < 0.0 {
        bail!("cancel.anti_phase_max_gain must be >= 0");
    }

    if !(0.0..1.0).contains(&config.cancel.anti_phase_smoothing) {
        bail!("cancel.anti_phase_smoothing must be in [0, 1)");
    }

    Ok(())
}

fn validate_pairing_mode(config: &NodeConfig) -> Result<()> {
    match (config.node.session_mode, config.node.role) {
        (SessionMode::MasterSlave, NodeRole::Master | NodeRole::Slave) => Ok(()),
        (SessionMode::Peer, NodeRole::Peer) => Ok(()),
        (SessionMode::Both, NodeRole::Peer) => Ok(()),
        (SessionMode::MasterSlave, NodeRole::Peer) => bail!(
            "master_slave mode only allows role=master or role=slave"
        ),
        (SessionMode::Peer | SessionMode::Both, NodeRole::Master | NodeRole::Slave) => bail!(
            "peer/both mode only allows role=peer"
        ),
    }
}

fn validate_output_routing(config: &NodeConfig) -> Result<()> {
    match (config.node.session_mode, config.node.role, config.output.routing) {
        (SessionMode::MasterSlave, NodeRole::Master, OutputRoutingMode::MixToPrimary)
        | (SessionMode::MasterSlave, NodeRole::Master, OutputRoutingMode::SplitLocalPeer)
        | (SessionMode::MasterSlave, NodeRole::Slave, OutputRoutingMode::LocalOnly)
        | (SessionMode::MasterSlave, NodeRole::Slave, OutputRoutingMode::Off)
        | (SessionMode::Peer, NodeRole::Peer, OutputRoutingMode::LocalOnly)
        | (SessionMode::Both, NodeRole::Peer, OutputRoutingMode::MixToPrimary)
        | (SessionMode::Both, NodeRole::Peer, OutputRoutingMode::SplitLocalPeer) => {}
        (SessionMode::MasterSlave, NodeRole::Master, _) => bail!(
            "master mode only allows output.routing = mix_to_primary or split_local_peer"
        ),
        (SessionMode::MasterSlave, NodeRole::Slave, _) => bail!(
            "slave mode only allows output.routing = local_only or off"
        ),
        (SessionMode::Peer, NodeRole::Peer, _) => {
            bail!("peer mode only allows output.routing = local_only")
        }
        (SessionMode::Both, NodeRole::Peer, _) => bail!(
            "both mode only allows output.routing = mix_to_primary or split_local_peer"
        ),
        _ => {}
    }

    if config.output.routing == OutputRoutingMode::SplitLocalPeer {
        if config.output.backend != OutputBackend::VirtualStub {
            bail!("output.routing = split_local_peer requires output.backend = virtual_stub");
        }
        if config
            .output
            .primary_target_device
            .trim()
            .eq_ignore_ascii_case(config.output.secondary_target_device.trim())
        {
            bail!(
                "split_local_peer requires two distinct output devices; use mix_to_primary for a single device mix"
            );
        }
    }

    if matches!(config.output.routing, OutputRoutingMode::MixToPrimary | OutputRoutingMode::LocalOnly)
        && config.output.primary_target_device.trim().is_empty()
        && config.output.backend == OutputBackend::VirtualStub
    {
        bail!("output.primary_target_device must not be empty for live output routing");
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
    app_base_dir().join("logs")
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

    app_base_dir_from_search_roots(search_roots).join(path)
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

fn load_config_from(path: &Path, search_roots: &[PathBuf]) -> Result<NodeConfig> {
    let resolved_path = resolve_config_path_from(path, search_roots);
    let raw = match fs::read_to_string(&resolved_path) {
        Ok(raw) => raw,
        Err(error) => {
            if let Some(embedded) =
                embedded_config_contents(path).or_else(|| embedded_config_contents(&resolved_path))
            {
                embedded.to_owned()
            } else {
                return Err(error)
                    .with_context(|| format!("failed to read config {}", resolved_path.display()));
            }
        }
    };

    let mut config: NodeConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {}", resolved_path.display()))?;
    normalize_relative_paths(&mut config, &app_base_dir_from_search_roots(search_roots));
    validate_config(&config)?;
    Ok(config)
}

fn normalize_relative_paths(config: &mut NodeConfig, base_dir: &Path) {
    if config.output.wav_path.is_relative() {
        config.output.wav_path = base_dir.join(&config.output.wav_path);
    }

    if config.debug.dump_dir.is_relative() {
        config.debug.dump_dir = base_dir.join(&config.debug.dump_dir);
    }
}

fn app_base_dir() -> PathBuf {
    app_base_dir_from_search_roots(&workspace_search_roots())
}

fn config_storage_dir_from_search_roots(search_roots: &[PathBuf]) -> PathBuf {
    app_base_dir_from_search_roots(search_roots).join("configs")
}

fn app_base_dir_from_search_roots(search_roots: &[PathBuf]) -> PathBuf {
    workspace_root_from_search_roots(search_roots)
        .or_else(executable_directory)
        .or_else(|| search_roots.first().cloned())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn executable_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|current_exe| current_exe.parent().map(Path::to_path_buf))
}

fn merge_embedded_presets(presets: &mut Vec<String>) {
    for preset in embedded_config_preset_names() {
        if !presets.iter().any(|existing| existing == &preset) {
            presets.push(preset);
        }
    }
    presets.sort();
}

fn embedded_config_preset_names() -> Vec<String> {
    embedded_config_presets()
        .iter()
        .map(|preset| preset.path.to_owned())
        .collect()
}

fn embedded_config_contents(path: &Path) -> Option<&'static str> {
    let path_text = path.to_string_lossy().replace('\\', "/");
    let file_name = path.file_name()?.to_string_lossy();

    embedded_config_presets()
        .iter()
        .find(|preset| {
            preset.path == path_text
                || preset.path.ends_with(path_text.as_str())
                || preset.path.ends_with(file_name.as_ref())
        })
        .map(|preset| preset.contents)
}

fn embedded_config_presets() -> &'static [EmbeddedConfigPreset] {
    &[
        EmbeddedConfigPreset {
            path: "configs/master.toml",
            contents: include_str!("../../../configs/master.toml"),
        },
        EmbeddedConfigPreset {
            path: "configs/slave.toml",
            contents: include_str!("../../../configs/slave.toml"),
        },
        EmbeddedConfigPreset {
            path: "configs/peer.toml",
            contents: include_str!("../../../configs/peer.toml"),
        },
    ]
}

struct EmbeddedConfigPreset {
    path: &'static str,
    contents: &'static str,
}

#[derive(Clone, Debug)]
struct ImportCandidate {
    source_name: String,
    destination_path: String,
    contents: String,
}

fn preview_import_config_directory_from(
    path: &Path,
    search_roots: &[PathBuf],
) -> Result<ConfigImportPreview> {
    let source_dir = path.to_path_buf();
    let existing_catalog = existing_config_catalog(search_roots)?;
    let candidates = collect_import_candidates(&source_dir)?;
    let mut known_paths: Vec<String> = existing_catalog.keys().cloned().collect();
    let mut known_contents: Vec<String> = existing_catalog.values().cloned().collect();

    let mut discovered_files = Vec::new();
    let mut skipped_duplicates = Vec::new();
    let mut conflicts = Vec::new();
    let mut importable_paths = Vec::new();

    for candidate in candidates {
        discovered_files.push(candidate.source_name.clone());
        if known_contents
            .iter()
            .any(|existing| existing == &candidate.contents)
        {
            skipped_duplicates.push(candidate.source_name);
            continue;
        }

        if known_paths
            .iter()
            .any(|existing| existing == &candidate.destination_path)
        {
            let suggested_path = next_available_config_path(
                &candidate.destination_path,
                known_paths.iter().map(String::as_str),
            );
            conflicts.push(ConfigImportConflict {
                source_name: candidate.source_name,
                existing_path: candidate.destination_path.clone(),
                suggested_path: suggested_path.clone(),
            });
            known_paths.push(suggested_path);
            known_contents.push(candidate.contents);
            continue;
        }

        importable_paths.push(candidate.destination_path.clone());
        known_paths.push(candidate.destination_path);
        known_contents.push(candidate.contents);
    }

    Ok(ConfigImportPreview {
        source_dir,
        discovered_files,
        skipped_duplicates,
        conflicts,
        importable_paths,
    })
}

fn import_config_directory_from(
    path: &Path,
    import_conflicts_with_rename: bool,
    search_roots: &[PathBuf],
) -> Result<ConfigImportResult> {
    let storage_dir = config_storage_dir_from_search_roots(search_roots);
    let existing_catalog = existing_config_catalog(search_roots)?;
    let candidates = collect_import_candidates(path)?;

    fs::create_dir_all(&storage_dir)
        .with_context(|| format!("failed to create config storage {}", storage_dir.display()))?;

    let mut known_paths: Vec<String> = existing_catalog.keys().cloned().collect();
    let mut known_contents: Vec<String> = existing_catalog.values().cloned().collect();
    let mut imported_paths = Vec::new();
    let mut skipped_duplicates = Vec::new();
    let mut skipped_conflicts = Vec::new();
    let mut renamed_imports = Vec::new();

    for candidate in candidates {
        if known_contents
            .iter()
            .any(|existing| existing == &candidate.contents)
        {
            skipped_duplicates.push(candidate.source_name);
            continue;
        }

        let mut destination_path = candidate.destination_path.clone();
        if known_paths
            .iter()
            .any(|existing| existing == &destination_path)
        {
            if !import_conflicts_with_rename {
                skipped_conflicts.push(candidate.source_name);
                continue;
            }

            let renamed_path = next_available_config_path(
                &destination_path,
                known_paths.iter().map(String::as_str),
            );
            renamed_imports.push((destination_path.clone(), renamed_path.clone()));
            destination_path = renamed_path;
        }

        let destination_disk_path = storage_dir.join(
            Path::new(&destination_path)
                .file_name()
                .expect("config destination should have a file name"),
        );
        fs::write(&destination_disk_path, &candidate.contents).with_context(|| {
            format!(
                "failed to write imported config {}",
                destination_disk_path.display()
            )
        })?;

        known_paths.push(destination_path.clone());
        known_contents.push(candidate.contents);
        imported_paths.push(destination_path);
    }

    Ok(ConfigImportResult {
        imported_paths,
        skipped_duplicates,
        skipped_conflicts,
        renamed_imports,
    })
}

fn collect_import_candidates(path: &Path) -> Result<Vec<ImportCandidate>> {
    let entries = fs::read_dir(path)
        .with_context(|| format!("failed to read config import directory {}", path.display()))?;
    let mut candidates = Vec::new();

    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry from {}", path.display()))?;
        let source_path = entry.path();
        if source_path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("toml")
        {
            continue;
        }

        let source_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid config file name {}", source_path.display()))?
            .to_owned();
        let contents = fs::read_to_string(&source_path)
            .with_context(|| format!("failed to read {}", source_path.display()))?;
        let config: NodeConfig = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", source_path.display()))?;
        validate_config(&config)
            .with_context(|| format!("invalid config {}", source_path.display()))?;

        candidates.push(ImportCandidate {
            source_name: source_name.clone(),
            destination_path: format!("configs/{source_name}"),
            contents,
        });
    }

    candidates.sort_by(|left, right| left.source_name.cmp(&right.source_name));
    Ok(candidates)
}

fn existing_config_catalog(search_roots: &[PathBuf]) -> Result<BTreeMap<String, String>> {
    let mut entries = BTreeMap::new();
    let storage_dir = config_storage_dir_from_search_roots(search_roots);
    if storage_dir.is_dir() {
        let read_dir = fs::read_dir(&storage_dir)
            .with_context(|| format!("failed to read {}", storage_dir.display()))?;
        for entry in read_dir {
            let entry = entry.with_context(|| {
                format!("failed to read an entry from {}", storage_dir.display())
            })?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow::anyhow!("invalid config file name {}", path.display()))?;
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            entries.insert(format!("configs/{file_name}"), contents);
        }
    }

    for preset in embedded_config_presets() {
        entries
            .entry(preset.path.to_owned())
            .or_insert_with(|| preset.contents.to_owned());
    }

    Ok(entries)
}

fn next_available_config_path<'a>(
    path: &str,
    existing_paths: impl IntoIterator<Item = &'a str>,
) -> String {
    let existing: Vec<&str> = existing_paths.into_iter().collect();
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(file_name);
    let extension = Path::new(file_name)
        .extension()
        .and_then(|name| name.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let (base_stem, start_index) = split_numeric_suffix(stem);

    let mut next_index = start_index.unwrap_or(0) + 1;
    loop {
        let candidate_file_name = format!("{base_stem}-{next_index}{extension}");
        let candidate_path = format!("configs/{candidate_file_name}");
        if existing
            .iter()
            .all(|existing_path| existing_path != &candidate_path)
        {
            return candidate_path;
        }
        next_index += 1;
    }
}

fn split_numeric_suffix(stem: &str) -> (&str, Option<u32>) {
    let Some((base, suffix)) = stem.rsplit_once('-') else {
        return (stem, None);
    };

    match suffix.parse::<u32>() {
        Ok(index) => (base, Some(index)),
        Err(_) => (stem, None),
    }
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
        config.output.primary_target_device = "CABLE Input (VB-Audio Virtual Cable)".to_owned();

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
            reloaded.output.primary_target_device,
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
        fs::write(configs_dir.join("master.toml"), "").expect("config file should write");

        let resolved = resolve_config_path_from(Path::new("configs/master.toml"), &[search_root]);
        assert_eq!(resolved, workspace.join("configs").join("master.toml"));

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

    #[test]
    fn load_config_falls_back_to_embedded_preset_when_file_missing() {
        let isolated = unique_temp_dir("embedded_preset");
        fs::create_dir_all(&isolated).expect("isolated temp dir should create");

        let config = load_config_from(Path::new("configs/peer.toml"), &[isolated.clone()])
            .expect("embedded preset should load without on-disk config");

        assert_eq!(config.node.name, "peer");
        assert!(config.debug.dump_dir.ends_with(Path::new("artifacts/peer")));
        assert!(
            config
                .output
                .wav_path
                .ends_with(Path::new("artifacts/peer/processed-output.wav"))
        );

        fs::remove_dir_all(isolated).expect("isolated temp dir should be removable");
    }

    #[test]
    fn save_config_creates_parent_directories_for_relative_preset_paths() {
        let isolated = unique_temp_dir("save_dirs");
        fs::create_dir_all(&isolated).expect("isolated temp dir should create");

        let mut config = NodeConfig::default();
        config.audio.input_device = "default".to_owned();
        let path = isolated.join("configs").join("portable.toml");

        save_config(&path, &config).expect("save_config should create parent directories");
        assert!(path.is_file(), "saved config file should exist");

        fs::remove_dir_all(isolated).expect("isolated temp dir should be removable");
    }

    #[test]
    fn discover_config_presets_uses_embedded_list_without_workspace() {
        let presets = embedded_config_preset_names();
        assert!(presets.contains(&"configs/master.toml".to_owned()));
        assert!(presets.contains(&"configs/peer.toml".to_owned()));
    }

    #[test]
    fn next_available_config_path_increments_numeric_suffix_without_nesting() {
        let candidate = next_available_config_path(
            "configs/node-a.toml",
            ["configs/node-a.toml", "configs/node-a-1.toml"].into_iter(),
        );
        assert_eq!(candidate, "configs/node-a-2.toml");

        let candidate = next_available_config_path(
            "configs/node-a-1.toml",
            ["configs/node-a.toml", "configs/node-a-1.toml"].into_iter(),
        );
        assert_eq!(candidate, "configs/node-a-2.toml");
    }

    #[test]
    fn preview_import_config_directory_detects_duplicates_and_conflicts() {
        let workspace = unique_temp_dir("import_preview_workspace");
        let configs_dir = workspace.join("configs");
        let import_dir = workspace.join("incoming");
        fs::create_dir_all(&configs_dir).expect("configs dir should create");
        fs::create_dir_all(&import_dir).expect("import dir should create");
        fs::create_dir_all(workspace.join("crates")).expect("crates dir should create");
        fs::write(workspace.join("Cargo.toml"), "[workspace]\n").expect("Cargo.toml should write");

        let base = include_str!("../../../configs/master.toml").to_owned();
        let mut different_config = NodeConfig::default();
        different_config.node.name = "incoming-master".to_owned();
        different_config.node.session_mode = SessionMode::MasterSlave;
        different_config.node.role = NodeRole::Master;
        different_config.output.routing = OutputRoutingMode::MixToPrimary;
        different_config.audio.input_device = "default".to_owned();
        let different =
            toml::to_string_pretty(&different_config).expect("different config should serialize");
        fs::write(configs_dir.join("master.toml"), &base).expect("existing config should write");
        fs::write(import_dir.join("copy.toml"), &base).expect("duplicate config should write");
        fs::write(import_dir.join("master.toml"), &different)
            .expect("conflict config should write");

        let preview =
            preview_import_config_directory_from(&import_dir, std::slice::from_ref(&workspace))
                .expect("preview import should succeed");

        assert_eq!(preview.skipped_duplicates, vec!["copy.toml"]);
        assert_eq!(preview.conflicts.len(), 1);
        assert_eq!(preview.conflicts[0].existing_path, "configs/master.toml");
        assert_eq!(preview.conflicts[0].suggested_path, "configs/master-1.toml");

        fs::remove_dir_all(workspace).expect("temp workspace should be removable");
    }

    #[test]
    fn validate_config_rejects_invalid_udp_socket_addresses() {
        let mut config = NodeConfig::default();
        config.node.transport_backend = TransportBackend::Udp;
        config.node.listen_addr = "not-an-endpoint".to_owned();

        let error = validate_config(&config).expect_err("invalid listen address should fail");
        assert!(
            error
                .to_string()
                .contains("node.listen_addr must be a valid IP:port"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_config_accepts_explicit_udp_ip_addresses() {
        let mut config = NodeConfig::default();
        config.node.transport_backend = TransportBackend::Udp;
        config.node.listen_addr = "0.0.0.0:38001".to_owned();
        config.node.peer_addr = "192.168.1.22:38001".to_owned();

        validate_config(&config).expect("explicit UDP IP addresses should validate");
    }

    #[test]
    fn validate_config_rejects_invalid_anti_phase_settings() {
        let mut config = NodeConfig::default();
        config.cancel.anti_phase_smoothing = 1.0;

        let error = validate_config(&config).expect_err("anti-phase smoothing at 1.0 should fail");
        assert!(
            error
                .to_string()
                .contains("cancel.anti_phase_smoothing must be in [0, 1)"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_config_rejects_invalid_role_for_session_mode() {
        let mut config = NodeConfig::default();
        config.node.session_mode = SessionMode::MasterSlave;
        config.node.role = NodeRole::Peer;

        let error = validate_config(&config).expect_err("peer role in master_slave should fail");
        assert!(
            error
                .to_string()
                .contains("master_slave mode only allows role=master or role=slave"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_config_rejects_invalid_routing_for_peer_mode() {
        let mut config = NodeConfig::default();
        config.node.session_mode = SessionMode::Peer;
        config.node.role = NodeRole::Peer;
        config.output.routing = OutputRoutingMode::MixToPrimary;

        let error = validate_config(&config).expect_err("peer mode mix should fail");
        assert!(
            error
                .to_string()
                .contains("peer mode only allows output.routing = local_only"),
            "unexpected error: {error}"
        );
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ek_dual_mic_{label}_{unique}"))
    }
}
