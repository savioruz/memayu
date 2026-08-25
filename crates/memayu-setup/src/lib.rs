//! Shared first-run setup flow for the configuration wizard (#54 / #55).
//!
//! Three presenters — the dialoguer CLI (`memayu setup`, default), the ratatui
//! TUI (`memayu setup --tui`), and the web dashboard (`GET /setup`) — drive the
//! exact same ordered set of [`SetupStep`]s and collect into the same
//! [`SetupAnswers`]. The only thing that differs between them is *how* a step is
//! rendered (plain text prompt, ratatui widget, or an HTML form); the questions,
//! their order, and the end result (config file + admin account + API key) are
//! identical.
//!
//! The step list in [`SETUP_STEPS`] is the single source of truth all presenters
//! iterate, so no presenter can silently add or drop a step.

use memayu_config::{config_path, read_config_file, ConfigFile, StorageBackend, StorageConfig};
use memayu_llm_client::local_embedder::DEFAULT_MODEL_ID;
use sysinfo::{Disks, System};

/// Read the config file at the canonical path, returning `None` when it does
/// not exist or cannot be parsed. Used by presenters to prefill defaults.
pub fn read_config_file_if_any() -> Option<ConfigFile> {
    read_config_file(&config_path()).ok().flatten()
}

/// Every question the wizard asks, in the order it is asked. All presenters
/// iterate this exact list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    /// Device capability report (arch/RAM/free disk) shown first; gates whether
    /// local (on-device) embedding is offered at all.
    DeviceCheck,
    /// Storage backend (libsql default / postgres).
    StorageBackend,
    /// libsql path (libsql only) or Postgres URL (postgres only).
    StoragePath,
    /// Embedder: local (on-device Candle) or http (bring-your-own-key).
    EmbedderBackend,
    /// Local model picker (only when embedder is `local`).
    LocalModel,
    /// HTTP embedder base URL + model (only when embedder is `http`).
    EmbedderConfig,
    /// Extraction mode: llm or raw.
    ExtractionMode,
    /// LLM provider config (only when extraction mode is `llm`).
    LlmConfig,
    /// Admin email.
    AdminEmail,
    /// Admin password.
    AdminPassword,
    /// Admin password confirmation.
    AdminConfirm,
    /// Bind address.
    BindAddr,
    /// Server port.
    Port,
    /// API key label.
    ApiKeyLabel,
}

/// The single ordered list of steps shared by all presenters.
pub const SETUP_STEPS: &[SetupStep] = &[
    SetupStep::DeviceCheck,
    SetupStep::StorageBackend,
    SetupStep::StoragePath,
    SetupStep::EmbedderBackend,
    SetupStep::LocalModel,
    SetupStep::EmbedderConfig,
    SetupStep::ExtractionMode,
    SetupStep::LlmConfig,
    SetupStep::AdminEmail,
    SetupStep::AdminPassword,
    SetupStep::AdminConfirm,
    SetupStep::BindAddr,
    SetupStep::Port,
    SetupStep::ApiKeyLabel,
];

/// All answers collected by the wizard. The same struct is filled by every
/// presenter, then handed to [`finalize`].
#[derive(Debug, Clone)]
pub struct SetupAnswers {
    pub storage_backend: StorageBackend,
    pub libsql_path: String,
    pub database_url: String,
    pub embedder_backend: String, // "local" | "remote"
    pub embedder_base_url: String,
    pub embedder_api_key: String,
    pub embedder_model: String,
    pub extraction_mode: String, // "llm" | "raw"
    pub llm_base_url: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub admin_email: String,
    pub admin_password: String,
    pub bind_addr: String,
    pub port: u16,
    pub api_key_label: String,
    /// Device capability report computed once at wizard start; gates whether
    /// the `local` embedder option is offered.
    pub device: DeviceReport,
    /// Embedding dimension determined by setup: from the local model catalog,
    /// or (for a remote embedder) from a one-shot dimension probe performed in
    /// [`finalize`]. Persisted to the config file so server startup never needs
    /// to probe the embedder.
    pub embedding_dim: Option<usize>,
}

impl Default for SetupAnswers {
    fn default() -> Self {
        SetupAnswers {
            storage_backend: StorageBackend::Libsql,
            libsql_path: "./memayu.db".into(),
            database_url: String::new(),
            embedder_backend: "local".into(),
            embedder_base_url: "https://api.openai.com/v1".into(),
            embedder_api_key: String::new(),
            embedder_model: DEFAULT_MODEL_ID.to_string(),
            extraction_mode: "raw".into(),
            llm_base_url: "https://api.openai.com/v1".into(),
            llm_api_key: String::new(),
            llm_model: "gpt-4".into(),
            admin_email: String::new(),
            admin_password: String::new(),
            bind_addr: "127.0.0.1".into(),
            port: 18080,
            api_key_label: "default".into(),
            device: DeviceReport::default(),
            embedding_dim: None,
        }
    }
}

/// One local (on-device Candle) embedding model offered by the picker.
///
/// All entries are BERT-architecture checkpoints, which is what the Candle
/// embedder loads. `EmbeddingGemma-300M` is intentionally absent: it is a
/// Gemma (decoder-only) model and will be added as a follow-up once the
/// embedder grows a Gemma code path.
pub struct LocalModelSpec {
    /// Hugging Face model id, as written to `config.toml` / passed to Candle.
    pub id: &'static str,
    /// Short name shown in the picker.
    pub name: &'static str,
    /// Output embedding dimension.
    pub dim: u16,
    /// fp32 safetensors size.
    pub fp32_size_mb: u32,
    /// quantized (int8) safetensors size.
    pub int8_size_mb: u32,
    /// Minimum RAM for a comfortable load.
    pub min_ram_mb: u32,
    /// Minimum free disk needed (download + on-disk weights, headroom).
    pub min_disk_mb: u32,
    /// CPU notes (speed, arch support).
    pub cpu_notes: &'static str,
    /// Language coverage.
    pub langs: &'static str,
}

/// The local models offered in the picker, in display order. The default model
/// ([`DEFAULT_MODEL_ID`]) is always present.
pub const LOCAL_MODELS: &[LocalModelSpec] = &[
    LocalModelSpec {
        id: "sentence-transformers/all-MiniLM-L6-v2",
        name: "all-MiniLM-L6-v2",
        dim: 384,
        fp32_size_mb: 90,
        int8_size_mb: 23,
        min_ram_mb: 300,
        min_disk_mb: 500,
        cpu_notes: "Very fast, ARMv8 OK",
        langs: "English",
    },
    LocalModelSpec {
        id: "BAAI/bge-small-en-v1.5",
        name: "bge-small-en-v1.5",
        dim: 384,
        fp32_size_mb: 130,
        int8_size_mb: 33,
        min_ram_mb: 350,
        min_disk_mb: 600,
        cpu_notes: "Fast, ARMv8 OK",
        langs: "English",
    },
    LocalModelSpec {
        id: "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
        name: "paraphrase-multilingual-MiniLM-L12-v2",
        dim: 384,
        fp32_size_mb: 470,
        int8_size_mb: 120,
        min_ram_mb: 700,
        min_disk_mb: 1200,
        cpu_notes: "Slower (2× layers), ARMv8 OK",
        langs: "50+ langs",
    },
    LocalModelSpec {
        id: "nomic-ai/nomic-embed-text-v1.5",
        name: "nomic-embed-text-v1.5",
        dim: 768,
        fp32_size_mb: 540,
        int8_size_mb: 140,
        min_ram_mb: 800,
        min_disk_mb: 1400,
        cpu_notes: "Heavier, slower on Pi",
        langs: "Multilingual",
    },
];

/// The short names of [`LOCAL_MODELS`], in the same order — used directly as a
/// select option list by every presenter.
pub const LOCAL_MODEL_NAMES: &[&str] = &[
    "all-MiniLM-L6-v2",
    "bge-small-en-v1.5",
    "paraphrase-multilingual-MiniLM-L12-v2",
    "nomic-embed-text-v1.5",
];

/// Index of the default model within [`LOCAL_MODELS`].
pub const DEFAULT_MODEL_INDEX: usize = 2;

/// Best-effort device capability report used to decide whether on-device (local)
/// embedding is viable, and shown to the user by the DeviceCheck step.
#[derive(Debug, Clone)]
pub struct DeviceReport {
    /// Target architecture string, e.g. `x86_64`, `aarch64`, `arm` (32-bit).
    pub arch: String,
    /// Target OS string, e.g. `linux`, `macos`, `windows`.
    pub os: String,
    /// CPU model name, e.g. `Apple M1`, `12th Gen Intel(R) Core(TM) i7-12700H`.
    pub cpu_name: String,
    /// Physical CPU core count (None when it could not be determined).
    pub cpu_cores: Option<usize>,
    /// Logical CPU thread count (None when it could not be determined).
    pub cpu_threads: Option<usize>,
    /// Total system RAM in bytes.
    pub ram_bytes: u64,
    /// Free space on the disk holding the model cache dir, in bytes.
    pub free_disk_bytes: u64,
    /// Whether local embedding is viable on this device.
    pub local_supported: bool,
    /// Human-readable issues that make local embedding unsupported (or explain
    /// warnings). Empty when everything is fine.
    pub issues: Vec<String>,
}

impl Default for DeviceReport {
    fn default() -> Self {
        DeviceReport {
            arch: std::env::consts::ARCH.to_string(),
            os: std::env::consts::OS.to_string(),
            cpu_name: String::new(),
            cpu_cores: None,
            cpu_threads: None,
            ram_bytes: 0,
            free_disk_bytes: 0,
            local_supported: true,
            issues: Vec::new(),
        }
    }
}

/// Format a byte count for human-readable display (e.g. `3.2 GB`).
pub fn fmt_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Human-readable CPU summary, e.g. `Apple M1 — 8 cores / 8 threads`. Falls back
/// to just the brand (or empty) when core/thread counts are unavailable.
pub fn fmt_cpu(d: &DeviceReport) -> String {
    let mut s = d.cpu_name.clone();
    let counts = match (d.cpu_cores, d.cpu_threads) {
        (Some(c), Some(t)) => format!(" — {c} cores / {t} threads"),
        (Some(c), None) => format!(" — {c} cores"),
        _ => String::new(),
    };
    s.push_str(&counts);
    s
}

/// Probe the device and produce a [`DeviceReport`] deciding whether the on-device
/// (local) embedder can run.
///
/// Local embedding needs a 64-bit CPU (32-bit ARM, i.e. `armv7`, is not
/// supported by the Candle backend), enough RAM to hold the smallest model, and
/// enough free disk to download its weights. Any probe that fails is treated
/// conservatively and reported as an issue rather than crashing the wizard.
pub fn check_device() -> DeviceReport {
    let arch = std::env::consts::ARCH.to_string();
    let os = std::env::consts::OS.to_string();

    let mut sys = System::new_all();
    sys.refresh_memory();
    let ram_bytes = sys.total_memory();

    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    let cpu_cores = sys.physical_core_count();
    let cpu_threads = Some(sys.cpus().len()).filter(|&n| n > 0);

    let model_dir = memayu_config::model_dir();
    let mut free_disk_bytes: u64 = 0;
    let disks = Disks::new_with_refreshed_list();
    let mut best_mount_len: usize = 0;
    for disk in &disks {
        let mount = disk.mount_point();
        // The mount point that is the longest prefix of the model dir is the
        // filesystem the model would land on.
        if model_dir.starts_with(mount) && mount.as_os_str().len() > best_mount_len {
            best_mount_len = mount.as_os_str().len();
            free_disk_bytes = disk.available_space();
        }
    }

    let mut issues: Vec<String> = Vec::new();
    let mut local_supported = true;

    // 32-bit ARM (armv7 and older) is not supported by the Candle build.
    if arch == "arm" {
        issues.push(
            "CPU is 32-bit ARM (armv7), which the on-device embedder does not support".to_string(),
        );
        local_supported = false;
    }

    let smallest_ram = LOCAL_MODELS
        .iter()
        .map(|m| m.min_ram_mb)
        .min()
        .unwrap_or(300);
    if ram_bytes < u64::from(smallest_ram) * 1024 * 1024 {
        issues.push(format!(
            "RAM ({}) is below the {} MB the smallest local model needs",
            fmt_bytes(ram_bytes),
            smallest_ram
        ));
        local_supported = false;
    }

    let smallest_disk = LOCAL_MODELS
        .iter()
        .map(|m| m.min_disk_mb)
        .min()
        .unwrap_or(500);
    if free_disk_bytes > 0 && free_disk_bytes < u64::from(smallest_disk) * 1024 * 1024 {
        issues.push(format!(
            "free disk ({}) is below the {} MB the smallest local model needs",
            fmt_bytes(free_disk_bytes),
            smallest_disk
        ));
        local_supported = false;
    }

    DeviceReport {
        arch,
        os,
        cpu_name,
        cpu_cores,
        cpu_threads,
        ram_bytes,
        free_disk_bytes,
        local_supported,
        issues,
    }
}

/// Seed defaults from an existing config file (for re-configuration), so the
/// wizard presents the current values as the default answers.
pub fn preseed(existing: Option<&ConfigFile>) -> SetupAnswers {
    let mut a = SetupAnswers::default();
    if let Some(e) = existing {
        if let Some(s) = &e.storage {
            a.storage_backend = s
                .backend
                .parse::<StorageBackend>()
                .unwrap_or(StorageBackend::Libsql);
            if let Some(p) = &s.libsql_path {
                a.libsql_path = p.clone();
            }
            if let Some(u) = &s.database_url {
                a.database_url = u.clone();
            }
        }
        if let Some(em) = &e.embedder {
            if let Some(b) = &em.backend {
                a.embedder_backend = b.clone();
            }
            if let Some(u) = &em.base_url {
                a.embedder_base_url = u.clone();
            }
            if let Some(k) = &em.api_key {
                a.embedder_api_key = k.clone();
            }
            if let Some(m) = &em.model {
                a.embedder_model = m.clone();
            }
        }
        if let Some(b) = &e.behavior {
            if let Some(m) = &b.extraction_mode {
                a.extraction_mode = m.clone();
            }
        }
        if let Some(l) = &e.llm {
            if let Some(u) = &l.base_url {
                a.llm_base_url = u.clone();
            }
            if let Some(k) = &l.api_key {
                a.llm_api_key = k.clone();
            }
            if let Some(m) = &l.model {
                a.llm_model = m.clone();
            }
        }
        if let Some(s) = &e.server {
            if let Some(b) = &s.bind_addr {
                a.bind_addr = b.clone();
            }
            if let Some(p) = s.port {
                a.port = p;
            }
        }
    }
    a
}

/// Whether a step should be shown given the answers collected so far.
///
/// Branches: storage path depends on backend; embedder config only for `http`;
/// LLM config only for `llm` extraction. Non-branch steps are always shown.
pub fn step_active(step: SetupStep, a: &SetupAnswers) -> bool {
    match step {
        SetupStep::DeviceCheck => true,
        SetupStep::StoragePath => true,
        SetupStep::LocalModel => a.embedder_backend == "local",
        SetupStep::EmbedderConfig => a.embedder_backend == "remote",
        SetupStep::LlmConfig => a.extraction_mode == "llm",
        _ => true,
    }
}

/// The label shown for a step (used by every presenter for consistent headers).
pub fn step_title(step: SetupStep) -> &'static str {
    match step {
        SetupStep::DeviceCheck => "Device check",
        SetupStep::StorageBackend => "Storage backend",
        SetupStep::StoragePath => "Storage path",
        SetupStep::EmbedderBackend => "Embedder backend",
        SetupStep::LocalModel => "Local model",
        SetupStep::EmbedderConfig => "Embedder provider",
        SetupStep::ExtractionMode => "Extraction mode",
        SetupStep::LlmConfig => "LLM provider",
        SetupStep::AdminEmail => "Admin email",
        SetupStep::AdminPassword => "Admin password",
        SetupStep::AdminConfirm => "Confirm password",
        SetupStep::BindAddr => "Bind address",
        SetupStep::Port => "Port",
        SetupStep::ApiKeyLabel => "API key name",
    }
}

fn storage_config(a: &SetupAnswers) -> StorageConfig {
    StorageConfig {
        backend: a.storage_backend,
        libsql_path: a.libsql_path.clone(),
        database_url: if a.database_url.is_empty() {
            None
        } else {
            Some(a.database_url.clone())
        },
    }
}

/// The outcome of [`finalize`]: where the config was written and the freshly
/// generated API key (shown exactly once by the caller).
#[derive(Debug, Clone)]
pub struct SetupResult {
    pub config_path: std::path::PathBuf,
    pub api_key: String,
}

/// Serialize the [`SetupAnswers`] into a [`ConfigFile`]. Used by every
/// presenter before writing, and exposed so callers can inspect the TOML.
pub fn config_file_from_answers(a: &SetupAnswers) -> ConfigFile {
    // Raw mode never calls an LLM, so it must not carry a placeholder/default
    // LLM block. A raw config writes an empty (no base_url/model/key) block;
    // `llm` mode writes the configured provider. This keeps a raw instance from
    // persisting a misleading gpt-4 placeholder in the config file or DB.
    let raw_mode = a.extraction_mode == "raw";
    ConfigFile {
        storage: Some(memayu_config::StorageConfigFile {
            backend: a.storage_backend.to_string(),
            libsql_path: if a.libsql_path.is_empty() {
                None
            } else {
                Some(a.libsql_path.clone())
            },
            database_url: if a.database_url.is_empty() {
                None
            } else {
                Some(a.database_url.clone())
            },
        }),
        llm: Some(memayu_config::ProviderConfigFile {
            backend: Some("remote".to_string()),
            base_url: if raw_mode {
                None
            } else {
                Some(a.llm_base_url.clone())
            },
            api_key: if raw_mode || a.llm_api_key.is_empty() {
                None
            } else {
                Some(a.llm_api_key.clone())
            },
            model: if raw_mode {
                None
            } else {
                Some(a.llm_model.clone())
            },
        }),
        embedder: Some(memayu_config::ProviderConfigFile {
            backend: Some(a.embedder_backend.clone()),
            base_url: if a.embedder_base_url.is_empty() {
                None
            } else {
                Some(a.embedder_base_url.clone())
            },
            api_key: if a.embedder_api_key.is_empty() {
                None
            } else {
                Some(a.embedder_api_key.clone())
            },
            model: Some(a.embedder_model.clone()),
        }),
        server: Some(memayu_config::ServerConfigFile {
            bind_addr: Some(a.bind_addr.clone()),
            port: Some(a.port),
        }),
        behavior: Some(memayu_config::BehaviorConfigFile {
            similarity_threshold: Some(0.65),
            extraction_mode: Some(a.extraction_mode.clone()),
        }),
        embedding_dim: effective_embedding_dimension(a),
    }
}

/// The dimension to persist: the probed/catalog value recorded on `a` first,
/// falling back to the local model catalog lookup for the `local` backend.
/// Remote backends rely on the probe done in [`finalize`]; when that probe is
/// unavailable (e.g. it was skipped), this returns `None` and the config simply
/// omits `embedding_dim`, leaving dimension resolution to server startup.
pub fn effective_embedding_dimension(a: &SetupAnswers) -> Option<usize> {
    a.embedding_dim.or_else(|| embedding_dimension(a))
}

/// The embedding dimension known statically from the selected model.
///
/// For the local (Candle) backend the dimension is known from the model catalog
/// ([`LOCAL_MODELS`]). For the remote (HTTP) backend the dimension is *not*
/// known statically — [`finalize`] performs a one-shot dimension probe instead,
/// so this returns `None` and the probed value is recorded on `a.embedding_dim`.
pub fn embedding_dimension(a: &SetupAnswers) -> Option<usize> {
    if a.embedder_backend == "local" {
        LOCAL_MODELS
            .iter()
            .find(|m| m.id == a.embedder_model)
            .map(|m| m.dim as usize)
    } else {
        None
    }
}

/// Map wizard answers into the DB-persisted Category B slice: the LLM and
/// embedder provider configs plus the extraction mode. Shared by the CLI/TUI
/// wizard (`finalize`) and the web `/setup` handler so all three persist the
/// exact same values.
pub fn provider_configs_from_answers(
    a: &SetupAnswers,
) -> (
    memayu_config::ProviderConfig,
    memayu_config::ProviderConfig,
    memayu_config::ExtractionMode,
) {
    let raw_mode = a.extraction_mode == "raw";
    let llm = memayu_config::ProviderConfig {
        backend: memayu_config::EmbedderBackend::Remote,
        base_url: if raw_mode {
            String::new()
        } else {
            a.llm_base_url.clone()
        },
        api_key: if raw_mode || a.llm_api_key.is_empty() {
            None
        } else {
            Some(a.llm_api_key.clone())
        },
        model: if raw_mode {
            String::new()
        } else {
            a.llm_model.clone()
        },
    };
    let embedder = memayu_config::ProviderConfig {
        backend: a.embedder_backend.parse().unwrap_or_default(),
        base_url: a.embedder_base_url.clone(),
        api_key: if a.embedder_api_key.is_empty() {
            None
        } else {
            Some(a.embedder_api_key.clone())
        },
        model: a.embedder_model.clone(),
    };
    let mode = a.extraction_mode.parse().unwrap_or_default();
    (llm, embedder, mode)
}

/// Complete setup: write the config file, create/resolve the admin account, and
/// generate the API key. Shared by all presenters, so the CLI, TUI, and web
/// paths produce identical results. Rendering the returned values is the
/// caller's job.
pub async fn finalize(a: &SetupAnswers) -> Result<SetupResult, Box<dyn std::error::Error>> {
    // 0. Determine the embedding dimension. Local backends know it from the
    //    model catalog; remote backends require a one-shot probe. The result is
    //    recorded on the answers and persisted to the config file below, so
    //    server startup never has to probe the embedder again (#55).
    let mut a = a.clone();
    if a.embedder_backend != "local" {
        let (_, embedder_cfg, _) = provider_configs_from_answers(&a);
        let embedder = memayu_llm_client::build_embedder(&embedder_cfg);
        match embedder.embed("dimension probe").await {
            Ok(vec) => a.embedding_dim = Some(vec.len()),
            Err(e) => eprintln!(
                "[memayu] warning: could not probe embedder dimension ({}); \
                 startup will need MEMAYU_EMBEDDER_DIM or an existing store",
                e
            ),
        }
    }

    // 1. Serialize and write the config file.
    let cf = config_file_from_answers(&a);
    let toml_str = toml::to_string_pretty(&cf)?;
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &toml_str)?;

    // 2. Create or resolve the admin account (re-config already has one).
    let storage = storage_config(&a);
    let admin_id = match memayu_identity::resolve_self_hosted_account_id(&storage).await {
        Ok(id) => id,
        Err(memayu_identity::IdentityError::NoAdminAccount) => {
            memayu_identity::create_admin_account(
                &storage,
                &a.admin_email,
                &a.admin_password,
                &a.admin_password,
            )
            .await
            .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?
        }
        Err(e) => return Err(Box::from(e.to_string())),
    };

    // 3. Generate the API key (the caller prints/shows it exactly once).
    let key = memayu_identity::generate_api_key(&storage, &admin_id, &a.api_key_label)
        .await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

    // 4. Persist Category B (LLM + embedder + extraction mode) to the DB via the
    //    shared write path. The DB is authoritative after first boot, so the
    //    choices made here survive subsequent env-only starts. Mirrored by the
    //    web `/setup` handler.
    let (llm, embedder, mode) = provider_configs_from_answers(&a);
    let db = memayu_api::DbClient::open(&storage)
        .await
        .map_err(Box::<dyn std::error::Error>::from)?;
    memayu_api::WebServices::new(db)
        .setup_persist(&llm, &embedder, mode)
        .await
        .map_err(Box::<dyn std::error::Error>::from)?;

    Ok(SetupResult {
        config_path: path,
        api_key: key.key,
    })
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn device_check_reports_cpu() {
        let d = check_device();
        // The probe should find a CPU brand and core/thread counts on a real
        // machine; an empty report would indicate the probe failed silently.
        assert!(!d.cpu_name.is_empty(), "cpu_name was empty: {d:?}");
        assert!(d.cpu_cores.is_some(), "cpu_cores was None: {d:?}");
        assert!(d.cpu_threads.is_some(), "cpu_threads was None: {d:?}");
        assert!(d.cpu_threads.unwrap() >= 1);
    }

    #[test]
    fn fmt_cpu_summarises_counts() {
        let d = DeviceReport {
            cpu_name: "Apple M1".to_string(),
            cpu_cores: Some(8),
            cpu_threads: Some(8),
            ..DeviceReport::default()
        };
        assert_eq!(fmt_cpu(&d), "Apple M1 — 8 cores / 8 threads");
    }

    #[test]
    fn embedding_dimension_local_looks_up_catalog() {
        let mut a = SetupAnswers::default();
        a.embedder_backend = "local".into();
        a.embedder_model = "BAAI/bge-small-en-v1.5".into();
        assert_eq!(embedding_dimension(&a), Some(384));
    }

    #[test]
    fn embedding_dimension_local_unknown_model_none() {
        let mut a = SetupAnswers::default();
        a.embedder_backend = "local".into();
        a.embedder_model = "unknown/model".into();
        assert_eq!(embedding_dimension(&a), None);
    }

    #[test]
    fn embedding_dimension_remote_is_none() {
        let mut a = SetupAnswers::default();
        a.embedder_backend = "remote".into();
        a.embedder_model = "text-embedding-3-small".into();
        assert_eq!(embedding_dimension(&a), None);
    }

    #[test]
    fn step_branching_matches_presenters() {
        let mut a = SetupAnswers::default();
        // Local embedder → LocalModel active, EmbedderConfig inactive.
        a.embedder_backend = "local".into();
        assert!(step_active(SetupStep::LocalModel, &a));
        assert!(!step_active(SetupStep::EmbedderConfig, &a));
        // Remote embedder → inverse.
        a.embedder_backend = "remote".into();
        assert!(!step_active(SetupStep::LocalModel, &a));
        assert!(step_active(SetupStep::EmbedderConfig, &a));
        // Raw extraction → LLM config inactive.
        a.extraction_mode = "raw".into();
        assert!(!step_active(SetupStep::LlmConfig, &a));
        a.extraction_mode = "llm".into();
        assert!(step_active(SetupStep::LlmConfig, &a));
    }

    #[test]
    fn config_file_reflects_backend_and_mode() {
        let mut a = SetupAnswers::default();
        a.embedder_backend = "remote".into();
        a.embedder_base_url = "https://emb.example.com/v1".into();
        a.embedder_model = "text-embedding-3-small".into();
        a.extraction_mode = "llm".into();
        a.llm_base_url = "https://llm.example.com/v1".into();
        a.llm_model = "gpt-4".into();
        let cf = config_file_from_answers(&a);
        assert_eq!(
            cf.embedder.as_ref().unwrap().backend.as_deref(),
            Some("remote")
        );
        assert_eq!(
            cf.embedder.as_ref().unwrap().base_url.as_deref(),
            Some("https://emb.example.com/v1")
        );
        assert_eq!(
            cf.behavior.as_ref().unwrap().extraction_mode.as_deref(),
            Some("llm")
        );
        assert_eq!(cf.llm.as_ref().unwrap().model.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn config_file_local_embedder_still_writes_backend() {
        let mut a = SetupAnswers::default();
        a.embedder_backend = "local".into();
        a.embedder_model = "sentence-transformers/all-MiniLM-L6-v2".into();
        let cf = config_file_from_answers(&a);
        assert_eq!(
            cf.embedder.as_ref().unwrap().backend.as_deref(),
            Some("local")
        );
        assert_eq!(
            cf.embedder.as_ref().unwrap().model.as_deref(),
            Some("sentence-transformers/all-MiniLM-L6-v2")
        );
    }

    #[test]
    fn raw_mode_provider_configs_carry_no_llm() {
        // The wizard defaults to `raw` and retains placeholder LLM answers
        // (gpt-4). In raw mode the DB write must not carry a placeholder LLM
        // provider, otherwise the instance could appear to be in llm mode.
        let a = SetupAnswers::default();
        assert_eq!(a.extraction_mode, "raw");
        assert_eq!(a.llm_base_url, "https://api.openai.com/v1");
        assert_eq!(a.llm_model, "gpt-4");
        let (llm, _embedder, mode) = provider_configs_from_answers(&a);
        assert_eq!(mode, memayu_config::ExtractionMode::Raw);
        assert_eq!(
            llm.base_url, "",
            "raw mode must not persist an LLM base_url"
        );
        assert_eq!(llm.model, "", "raw mode must not persist an LLM model");
        assert!(
            llm.api_key.is_none(),
            "raw mode must not persist an LLM key"
        );
    }

    #[test]
    fn llm_mode_provider_configs_keep_llm() {
        let mut a = SetupAnswers::default();
        a.extraction_mode = "llm".into();
        a.llm_base_url = "https://llm.example.com/v1".into();
        a.llm_api_key = "sk-test".into();
        a.llm_model = "gpt-4".into();
        let (llm, _embedder, mode) = provider_configs_from_answers(&a);
        assert_eq!(mode, memayu_config::ExtractionMode::Llm);
        assert_eq!(llm.base_url, "https://llm.example.com/v1");
        assert_eq!(llm.api_key.as_deref(), Some("sk-test"));
        assert_eq!(llm.model, "gpt-4");
    }

    #[test]
    fn raw_mode_config_file_has_no_llm_placeholder() {
        let a = SetupAnswers::default();
        let cf = config_file_from_answers(&a);
        assert_eq!(
            cf.behavior.as_ref().unwrap().extraction_mode.as_deref(),
            Some("raw")
        );
        let llm = cf.llm.as_ref().unwrap();
        assert_eq!(
            llm.base_url, None,
            "raw config must not write an LLM base_url"
        );
        assert_eq!(llm.model, None, "raw config must not write an LLM model");
        assert_eq!(llm.api_key, None, "raw config must not write an LLM key");
    }
}
