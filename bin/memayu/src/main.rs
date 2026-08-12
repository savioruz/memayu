mod wizard;

use memayu_config::{config_path, Config, StorageBackend};
use memayu_core::{EmbedderProvider, MemoryService, StorageProvider};
use memayu_mcp::{Backend, MemoryBackend};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    let _bin = args.next();
    let subcommand = args.next().unwrap_or_else(|| "auto".into());

    match subcommand.as_str() {
        "serve" | "mcp" => {
            // Load config (file + env merge)
            let config = Config::load()?;
            match subcommand.as_str() {
                "serve" => cmd_serve(config).await?,
                "mcp" => cmd_mcp(config).await?,
                _ => unreachable!(),
            }
        }
        "setup" => {
            wizard::run_wizard_preseed(true)?;
        }
        "config" => {
            let sub = args.next().unwrap_or_else(|| "show".into());
            match sub.as_str() {
                "show" => {
                    let config = Config::load()?;
                    println!("Config file: {}", config_path().display());
                    println!();
                    print!("{}", config.show());
                }
                "check" => {
                    let config = Config::load()?;
                    let issues = config.check();
                    if issues.is_empty() {
                        println!("{} Config is valid.", console::style("✔").green().bold());
                    } else {
                        eprintln!("{} Config has issues:\n", console::style("✘").red().bold());
                        for issue in &issues {
                            eprintln!("  • {issue}");
                        }
                        std::process::exit(1);
                    }
                }
                other => {
                    eprintln!("unknown config subcommand: {other}");
                    eprintln!("usage: memayu config <show|check>");
                    std::process::exit(1);
                }
            }
        }
        "auto" => {
            // First-run: no subcommand given. Check if config file exists.
            // If not → run wizard; otherwise → serve.
            let cp = config_path();
            if cp.exists() {
                let config = Config::load()?;
                cmd_serve(config).await?;
            } else {
                // Check if env vars provide enough
                match Config::load() {
                    Ok(config) => {
                        // Env vars work — serve
                        println!(
                            "[memayu] using env vars (no config file at {})",
                            cp.display()
                        );
                        cmd_serve(config).await?;
                    }
                    Err(e) => {
                        eprintln!(
                            "{} No config file found at {}, and env vars insufficient: {e}",
                            console::style("→").yellow().bold(),
                            cp.display()
                        );
                        println!();
                        println!("{}", console::style("Starting setup wizard…").cyan());
                        println!();
                        wizard::run_wizard()?;
                        println!();
                        println!(
                            "{}",
                            console::style("Starting server with new config…").cyan()
                        );
                        let config = Config::load()?;
                        cmd_serve(config).await?;
                    }
                }
            }
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            eprintln!("usage: memayu [serve|mcp|setup|config]");
            std::process::exit(1);
        }
    }

    Ok(())
}

// ── serve ──

async fn cmd_serve(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    if config.api_url.is_some() {
        eprintln!(
            "error: 'serve' subcommand requires local config — MEMAYU_API_URL is set (cloud mode)"
        );
        std::process::exit(1);
    }

    let service = build_service(&config).await?;

    let api_db = memayu_api::open_db(&config.storage).await?;
    let registry =
        memayu_api::load_registry(&api_db, config.llm.clone(), config.embedder.clone()).await?;

    let api = memayu_api::build_api_router(api_db.clone(), service.clone(), registry.clone());
    let web = memayu_web::build_web_router(api_db, service, registry);

    let app = axum::Router::new().merge(api).merge(web);

    let addr = format!("{}:{}", config.server.bind_addr, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[memayu] listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

// ── mcp ──

async fn cmd_mcp(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let backend: Arc<dyn MemoryBackend> = if let Some(api_url) = config.api_url {
        Arc::new(Backend::Cloud {
            base_url: api_url,
            api_key: config.api_key,
            client: reqwest::Client::new(),
        })
    } else {
        let service = build_service(&config).await?;
        Arc::new(Backend::Local(service))
    };

    memayu_mcp::run(backend).await;
    Ok(())
}

// ── shared build ──

async fn build_service(config: &Config) -> Result<Arc<MemoryService>, Box<dyn std::error::Error>> {
    let api_db = memayu_api::open_db(&config.storage).await?;
    let registry =
        memayu_api::load_registry(&api_db, config.llm.clone(), config.embedder.clone()).await?;

    let embedder = memayu_api::EmbedderConfigProvider::new(registry.clone());
    let detected_dim = match config.dimension {
        Some(d) => d,
        None => embedder.embed("dimension probe").await?.len(),
    };
    println!(
        "[memayu] embedder dimension = {detected_dim} (from {} {})",
        config.embedder.base_url, config.embedder.model
    );

    let storage: Arc<dyn StorageProvider> = match config.storage.backend {
        StorageBackend::Libsql => Arc::new(
            memayu_storage_libsql::LibsqlProvider::open(&config.storage.libsql_path, detected_dim)
                .await?,
        ),
        StorageBackend::Postgres => Arc::new(
            memayu_storage_postgres::PostgresProvider::connect(
                config
                    .storage
                    .database_url
                    .as_deref()
                    .ok_or("missing postgres url")?,
                detected_dim,
            )
            .await?,
        ),
    };

    let llm = memayu_api::LlmConfigProvider::new(registry.clone());
    Ok(Arc::new(MemoryService::new(
        storage,
        Arc::new(embedder),
        Arc::new(llm),
    )))
}
