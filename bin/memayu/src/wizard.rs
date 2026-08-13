use dialoguer::{theme::ColorfulTheme, Input, Select};
use memayu_config::{config_path, ConfigFile};

fn prompt(label: &str, default: &str) -> String {
    let result = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt(label)
        .default(default.to_string())
        .interact_text()
        .unwrap_or_default();
    result.trim().to_string()
}

pub fn run_wizard() -> Result<String, Box<dyn std::error::Error>> {
    println!(
        "{}",
        console::style("  memayu setup  — first-run configuration wizard")
            .bold()
            .cyan()
    );
    println!();

    // ── Storage ──
    println!("{}", console::style("── Storage ──").bold().dim());
    let backends = vec!["libsql", "postgres"];
    let backend_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Storage backend")
        .items(&backends)
        .default(0)
        .interact()?;
    let backend = backends[backend_idx];

    let libsql_path = if backend == "libsql" {
        prompt("libsql database path", "./memayu.db")
    } else {
        String::new()
    };

    let database_url = if backend == "postgres" {
        prompt("Postgres connection URL", "postgres://localhost/memayu")
    } else {
        String::new()
    };

    println!();

    // ── LLM ──
    println!("{}", console::style("── LLM Provider ──").bold().dim());
    let llm_base_url = prompt("Base URL", "https://api.openai.com/v1");
    let llm_api_key = prompt("API key (optional, press enter to skip)", "");
    let llm_model = prompt("Model", "gpt-4");

    println!();

    // ── Embedder ──
    println!("{}", console::style("── Embedder Provider ──").bold().dim());
    let emb_base_url = prompt("Base URL", "https://api.openai.com/v1");
    let emb_api_key = prompt("API key (optional, press enter to skip)", "");
    let emb_model = prompt("Model", "text-embedding-3-small");

    println!();

    // ── Server ──
    println!("{}", console::style("── Server ──").bold().dim());
    let bind_addr = prompt("Bind address", "127.0.0.1");
    let port_str = prompt("Port", "18080");
    let port: u16 = port_str.parse().unwrap_or(18080);

    println!();

    // ── Behavior ──
    println!("{}", console::style("── Behavior ──").bold().dim());
    let sim_str = prompt("Similarity threshold (0.0-1.0)", "0.65");
    let similarity_threshold: f32 = sim_str.parse().unwrap_or(0.65);

    let modes = vec!["llm", "raw"];
    let mode_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Extraction mode")
        .items(&modes)
        .default(0)
        .interact()?;
    let extraction_mode = modes[mode_idx].to_string();

    // ── Build ConfigFile and serialize ──
    let cf = ConfigFile {
        storage: Some(memayu_config::StorageConfigFile {
            backend: backend.to_string(),
            libsql_path: if libsql_path.is_empty() {
                None
            } else {
                Some(libsql_path)
            },
            database_url: if database_url.is_empty() {
                None
            } else {
                Some(database_url)
            },
        }),
        llm: Some(memayu_config::ProviderConfigFile {
            base_url: Some(llm_base_url),
            api_key: if llm_api_key.is_empty() {
                None
            } else {
                Some(llm_api_key)
            },
            model: Some(llm_model),
        }),
        embedder: Some(memayu_config::ProviderConfigFile {
            base_url: Some(emb_base_url),
            api_key: if emb_api_key.is_empty() {
                None
            } else {
                Some(emb_api_key)
            },
            model: Some(emb_model),
        }),
        server: Some(memayu_config::ServerConfigFile {
            bind_addr: Some(bind_addr),
            port: Some(port),
        }),
        behavior: Some(memayu_config::BehaviorConfigFile {
            similarity_threshold: Some(similarity_threshold),
            extraction_mode: Some(extraction_mode),
        }),
    };

    let toml_str = toml::to_string_pretty(&cf)?;
    let path = config_path();

    // Create parent directory
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, &toml_str)?;

    println!();
    println!(
        "{} Written to {}",
        console::style("✔").green().bold(),
        path.display()
    );
    println!();
    println!(
        "  {}",
        console::style(
            "Run 'memayu serve' to start the server, or 'memayu config show' to review."
        )
        .dim()
    );

    Ok(toml_str)
}

/// Run the wizard with pre-filled values from a previous file (for re-config).
pub fn run_wizard_preseed(skip_intro: bool) -> Result<String, Box<dyn std::error::Error>> {
    if !skip_intro {
        println!(
            "{}",
            console::style("  memayu setup  — re-configure")
                .bold()
                .cyan()
        );
        println!();
    }

    let existing = memayu_config::read_config_file(&config_path())
        .ok()
        .flatten();

    // ── Storage ──
    println!("{}", console::style("── Storage ──").bold().dim());
    let current_backend = existing
        .as_ref()
        .and_then(|e| e.storage.as_ref())
        .and_then(|s| s.backend.parse::<memayu_config::StorageBackend>().ok())
        .map(|b| b.to_string());
    let backends = vec!["libsql", "postgres"];
    let default_idx = match current_backend.as_deref() {
        Some("postgres") => 1,
        _ => 0,
    };
    let backend_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Storage backend")
        .items(&backends)
        .default(default_idx)
        .interact()?;
    let backend = backends[backend_idx];

    let libsql_path = if backend == "libsql" {
        prompt(
            "libsql database path",
            &existing
                .as_ref()
                .and_then(|e| e.storage.as_ref())
                .and_then(|s| s.libsql_path.clone())
                .unwrap_or_else(|| "./memayu.db".into()),
        )
    } else {
        String::new()
    };

    let database_url = if backend == "postgres" {
        prompt(
            "Postgres connection URL",
            &existing
                .as_ref()
                .and_then(|e| e.storage.as_ref())
                .and_then(|s| s.database_url.clone())
                .unwrap_or_default(),
        )
    } else {
        String::new()
    };

    println!();

    // ── LLM ──
    println!("{}", console::style("── LLM Provider ──").bold().dim());
    let llm_base_url = prompt(
        "Base URL",
        &existing
            .as_ref()
            .and_then(|e| e.llm.as_ref())
            .and_then(|l| l.base_url.clone())
            .unwrap_or_else(|| "https://api.openai.com/v1".into()),
    );
    let llm_api_key = prompt(
        "API key (optional, press enter to skip)",
        &existing
            .as_ref()
            .and_then(|e| e.llm.as_ref())
            .and_then(|l| l.api_key.clone())
            .unwrap_or_default(),
    );
    let llm_model = prompt(
        "Model",
        &existing
            .as_ref()
            .and_then(|e| e.llm.as_ref())
            .and_then(|l| l.model.clone())
            .unwrap_or_else(|| "gpt-4".into()),
    );

    println!();

    // ── Embedder ──
    println!("{}", console::style("── Embedder Provider ──").bold().dim());
    let emb_base_url = prompt(
        "Base URL",
        &existing
            .as_ref()
            .and_then(|e| e.embedder.as_ref())
            .and_then(|l| l.base_url.clone())
            .unwrap_or_else(|| "https://api.openai.com/v1".into()),
    );
    let emb_api_key = prompt(
        "API key (optional, press enter to skip)",
        &existing
            .as_ref()
            .and_then(|e| e.embedder.as_ref())
            .and_then(|l| l.api_key.clone())
            .unwrap_or_default(),
    );
    let emb_model = prompt(
        "Model",
        &existing
            .as_ref()
            .and_then(|e| e.embedder.as_ref())
            .and_then(|l| l.model.clone())
            .unwrap_or_else(|| "text-embedding-3-small".into()),
    );

    println!();

    // ── Server ──
    println!("{}", console::style("── Server ──").bold().dim());
    let bind_addr = prompt(
        "Bind address",
        &existing
            .as_ref()
            .and_then(|e| e.server.as_ref())
            .and_then(|s| s.bind_addr.clone())
            .unwrap_or_else(|| "127.0.0.1".into()),
    );
    let prev_port = existing
        .as_ref()
        .and_then(|e| e.server.as_ref())
        .and_then(|s| s.port)
        .map(|p| p.to_string())
        .unwrap_or_else(|| "18080".into());
    let port_str = prompt("Port", &prev_port);
    let port: u16 = port_str.parse().unwrap_or(18080);

    println!();

    // ── Behavior ──
    println!("{}", console::style("── Behavior ──").bold().dim());
    let prev_sim = existing
        .as_ref()
        .and_then(|e| e.behavior.as_ref())
        .and_then(|b| b.similarity_threshold)
        .map(|t| t.to_string())
        .unwrap_or_else(|| "0.65".into());
    let sim_str = prompt("Similarity threshold (0.0-1.0)", &prev_sim);
    let similarity_threshold: f32 = sim_str.parse().unwrap_or(0.65);

    let current_mode = existing
        .as_ref()
        .and_then(|e| e.behavior.as_ref())
        .and_then(|b| b.extraction_mode.clone())
        .unwrap_or_else(|| "llm".into());
    let modes = vec!["llm", "raw"];
    let mode_default_idx = match current_mode.as_str() {
        "raw" => 1,
        _ => 0,
    };
    let mode_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Extraction mode")
        .items(&modes)
        .default(mode_default_idx)
        .interact()?;
    let extraction_mode = modes[mode_idx].to_string();

    // ── Build ConfigFile and serialize ──
    let cf = ConfigFile {
        storage: Some(memayu_config::StorageConfigFile {
            backend: backend.to_string(),
            libsql_path: if libsql_path.is_empty() {
                None
            } else {
                Some(libsql_path)
            },
            database_url: if database_url.is_empty() {
                None
            } else {
                Some(database_url)
            },
        }),
        llm: Some(memayu_config::ProviderConfigFile {
            base_url: Some(llm_base_url),
            api_key: if llm_api_key.is_empty() {
                None
            } else {
                Some(llm_api_key)
            },
            model: Some(llm_model),
        }),
        embedder: Some(memayu_config::ProviderConfigFile {
            base_url: Some(emb_base_url),
            api_key: if emb_api_key.is_empty() {
                None
            } else {
                Some(emb_api_key)
            },
            model: Some(emb_model),
        }),
        server: Some(memayu_config::ServerConfigFile {
            bind_addr: Some(bind_addr),
            port: Some(port),
        }),
        behavior: Some(memayu_config::BehaviorConfigFile {
            similarity_threshold: Some(similarity_threshold),
            extraction_mode: Some(extraction_mode),
        }),
    };

    let toml_str = toml::to_string_pretty(&cf)?;
    let path = config_path();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, &toml_str)?;

    println!();
    println!(
        "{} Written to {}",
        console::style("✔").green().bold(),
        path.display()
    );
    println!();
    println!(
        "  {}",
        console::style(
            "Run 'memayu serve' to start the server, or 'memayu config show' to review."
        )
        .dim()
    );

    Ok(toml_str)
}
