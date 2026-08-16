mod cli;
mod doctor;
mod service;
mod setup_flow;
#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "tui")]
mod tui_setup;
mod wizard;

#[cfg(feature = "web")]
use memayu_config::StorageBackend;
use memayu_config::{config_path, Config};
#[cfg(feature = "web")]
use memayu_core::MemoryService;
#[cfg(feature = "web")]
use std::io::IsTerminal;
#[cfg(any(feature = "web", feature = "mcp"))]
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    let _bin = args.next();
    let subcommand = args.next().unwrap_or_else(|| "auto".into());

    match subcommand.as_str() {
        // Version reporting (no config required).
        "--version" | "-V" | "version" => {
            cli::cmd_version();
        }
        // Non-interactive memory commands (single-user, in-process).
        "add" => {
            let config = Config::load()?;
            run_cli(cli::cmd_add(&config, args).await);
        }
        "search" => {
            let config = Config::load()?;
            run_cli(cli::cmd_search(&config, args).await);
        }
        "list" => {
            let config = Config::load()?;
            run_cli(cli::cmd_list(&config, args).await);
        }
        "get" => {
            let config = Config::load()?;
            run_cli(cli::cmd_get(&config, args).await);
        }
        "delete" => {
            let config = Config::load()?;
            run_cli(cli::cmd_delete(&config, args).await);
        }
        // Diagnostics: never requires a fully valid config, so it can report
        // exactly what is wrong.
        "doctor" => {
            match Config::load() {
                Ok(config) => std::process::exit(doctor::cmd_doctor(&config).await),
                Err(e) => {
                    eprintln!(
                        "{} Could not load config: {e}",
                        console::style("✘").red().bold()
                    );
                    eprintln!("{} hint: run `memayu setup` to create a config, or set the MEMAYU_* env vars", console::style("→").yellow());
                    std::process::exit(1);
                }
            }
        }
        #[cfg(feature = "web")]
        "serve" => {
            let config = Config::load()?;
            cmd_serve(config).await?;
        }
        #[cfg(feature = "mcp")]
        "mcp" => {
            let config = Config::load()?;
            cmd_mcp(config).await?;
        }
        #[cfg(feature = "tui")]
        "tui" => {
            let config = Config::load()?;
            cmd_default(config).await?;
        }
        "setup" => {
            // #54: default is the CLI-interactive wizard (dialoguer, plain
            // stdin/stdout, agent-friendly). `--tui` opts into the ratatui
            // presentation of the identical flow.
            let flag = args.next().unwrap_or_default();
            match flag.as_str() {
                "--tui" => {
                    #[cfg(feature = "tui")]
                    {
                        tui_setup::run_full_tui_setup().await?;
                    }
                    #[cfg(not(feature = "tui"))]
                    {
                        eprintln!(
                            "{} error: `setup --tui` requires the 'tui' feature (rebuild with --features tui)",
                            console::style("✘").red().bold()
                        );
                        std::process::exit(1);
                    }
                }
                _ => {
                    wizard::run_cli_setup(true).await?;
                }
            }
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
        "auto" | "" => {
            // No subcommand: default frontend is the TUI when it is compiled in,
            // otherwise the web dashboard.
            //
            // #45: if there is no TTY (a headless/piped invocation), the TUI
            // cannot render — instead of hanging, fall back to serve mode.
            #[cfg(feature = "web")]
            let headless = !std::io::stdin().is_terminal() && !std::io::stdout().is_terminal();

            #[cfg(feature = "web")]
            if headless {
                eprintln!(
                    "{} No TTY detected; falling back to serve mode",
                    console::style("→").yellow()
                );
                match Config::load() {
                    Ok(config) => {
                        cmd_serve(config).await?;
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!(
                            "{} No valid config in headless mode: {e}",
                            console::style("✘").red().bold()
                        );
                        eprintln!(
                            "{} hint: run `memayu setup` (works without a TTY) or set the MEMAYU_* env vars",
                            console::style("→").yellow()
                        );
                        std::process::exit(1);
                    }
                }
            }

            let cp = config_path();
            if cp.exists() {
                let config = Config::load()?;
                cmd_default(config).await?;
            } else {
                match Config::load() {
                    Ok(config) => {
                        println!(
                            "[memayu] using env vars (no config file at {})",
                            cp.display()
                        );
                        cmd_default(config).await?;
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
                        wizard::run_cli_setup(false).await?;
                        println!();
                        println!("{}", console::style("Starting with new config…").cyan());
                        let config = Config::load()?;
                        cmd_default(config).await?;
                    }
                }
            }
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            eprintln!("{}", usage());
            std::process::exit(1);
        }
    }

    Ok(())
}

fn run_cli(result: Result<(), String>) {
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn usage() -> String {
    let mut subcommands: Vec<&str> = Vec::new();
    subcommands.push("setup");
    subcommands.push("config");
    subcommands.push("add");
    subcommands.push("search");
    subcommands.push("list");
    subcommands.push("get");
    subcommands.push("delete");
    subcommands.push("doctor");
    #[cfg(feature = "tui")]
    subcommands.push("tui");
    #[cfg(feature = "web")]
    subcommands.push("serve");
    #[cfg(feature = "mcp")]
    subcommands.push("mcp");
    format!("usage: memayu [{}]", subcommands.join("|"))
}

/// Default frontend: TUI when available, otherwise the web dashboard.
async fn cmd_default(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "tui")]
    {
        tui::run(config).await
    }
    #[cfg(all(not(feature = "tui"), feature = "web"))]
    {
        cmd_serve(config).await
    }
    #[cfg(all(not(feature = "tui"), not(feature = "web")))]
    {
        let _ = config;
        eprintln!("error: this build has no TUI or web frontend enabled");
        eprintln!("hint: rebuild with --features tui,web");
        std::process::exit(1)
    }
}

// ── web dashboard ──

#[cfg(feature = "web")]
async fn cmd_serve(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    if config.api_url.is_some() {
        eprintln!(
            "error: 'serve' subcommand requires local config — MEMAYU_API_URL is set (cloud mode)"
        );
        std::process::exit(1);
    }

    let service = build_web_service(&config).await?;

    let api_db = memayu_api::open_db(&config.storage).await?;
    let registry =
        memayu_api::load_registry(&api_db, config.llm.clone(), config.embedder.clone()).await?;

    // One-time migration: re-assign any legacy placeholder memory rows
    // (e.g. "default") to the admin account so every frontend sees the same
    // store (#32). On a fresh instance there is no admin yet — setup creates
    // one — so a missing account is expected and not an error here.
    if let Ok(admin_id) = memayu_identity::resolve_self_hosted_account_id(&config.storage).await {
        if let Ok(n) =
            memayu_identity::backfill_placeholder_memories(&config.storage, &admin_id).await
        {
            if n > 0 {
                println!("[memayu] reassigned {n} legacy memory rows to the admin account");
            }
        }
    }

    let api = memayu_api::build_api_router(api_db.clone(), service.clone(), registry.clone());
    let web = memayu_web::build_web_router(api_db, service, registry);

    let app = axum::Router::new().merge(api).merge(web);

    let addr = format!("{}:{}", config.server.bind_addr, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[memayu] listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

// ── MCP stdio ──

#[cfg(feature = "mcp")]
async fn cmd_mcp(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    use memayu_mcp::{Backend, MemoryBackend};

    let backend: Arc<dyn MemoryBackend> = if let Some(api_url) = config.api_url {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("reqwest::Client should build with timeouts");
        Arc::new(Backend::Cloud {
            base_url: api_url,
            api_key: config.api_key,
            client,
        })
    } else {
        let (service, _) = service::build_service(&config).await?;
        // In-process MCP shares the instance's single admin account (#32).
        let account_id = memayu_identity::bootstrap(&config.storage).await?;
        Arc::new(Backend::Local {
            service,
            account_id,
        })
    };

    memayu_mcp::run(backend).await;
    Ok(())
}

// ── shared service builders ──

/// Build the dashboard service via the API registry, which powers runtime
/// provider reconfiguration in the web UI.
#[cfg(feature = "web")]
async fn build_web_service(
    config: &Config,
) -> Result<Arc<MemoryService>, Box<dyn std::error::Error>> {
    use memayu_core::{EmbedderProvider, StorageProvider};

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
    Ok(Arc::new(
        MemoryService::new(storage, Arc::new(embedder), Arc::new(llm))
            .with_extraction_mode(config.extraction_mode),
    ))
}
