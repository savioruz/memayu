//! `memayu doctor` — diagnose a local install.
//!
//! Runs a battery of non-destructive checks against the effective config:
//! config validity, storage reachability/schema, and LLM/embedder connectivity
//! (key validity + model availability). Exits 0 when everything is healthy and
//! 1 when problems are found.
//!
//! Nothing here mutates state: storage is inspected, not opened-for-write, and
//! providers are probed via `GET /models` rather than issuing completions.

use console::style;
use memayu_config::{Config, StorageBackend};
use memayu_core::ExtractionMode;

/// Run the doctor checks and return the process exit code (0 healthy, 1 issues).
pub async fn cmd_doctor(config: &Config) -> i32 {
    // Cloud mode has no local providers/storage to inspect.
    if config.api_url.is_some() {
        println!("{} Cloud-mode install", section("mode"));
        println!(
            "  {} connected to {} (local checks skipped)",
            style("→").yellow(),
            config.api_url.as_deref().unwrap_or("")
        );
        println!();
        println!("{}", style("no local configuration to diagnose").dim());
        return 0;
    }

    let mut ok = true;
    let mut warnings = 0usize;

    // ── 1. Configuration ──
    println!("{} Checking configuration", section("config"));
    let issues = config.check();
    if issues.is_empty() {
        println!("  {} config is valid", style("✔").green().bold());
    } else {
        ok = false;
        for issue in &issues {
            println!("  {} {issue}", style("✘").red().bold());
        }
    }
    print_config(config);

    // ── 2. Storage ──
    println!("\n{} Checking storage", section("storage"));
    match inspect_storage(config).await {
        Ok(info) => {
            ok &= info.passed;
            warnings += info.warnings;
            info.print();
        }
        Err(e) => {
            ok = false;
            println!("  {} {e}", style("✘").red().bold());
        }
    }

    // ── 3. Providers ──
    if config.extraction_mode != ExtractionMode::Raw {
        println!("\n{} Checking LLM provider", section("llm"));
        let (passed, warned) = check_provider(
            "LLM",
            &config.llm.base_url,
            config.llm.api_key.as_deref(),
            &config.llm.model,
        )
        .await;
        ok &= passed;
        warnings += usize::from(warned);
    }

    println!("\n{} Checking embedder", section("embedder"));
    let (passed, warned) = check_provider(
        "embedder",
        &config.embedder.base_url,
        config.embedder.api_key.as_deref(),
        &config.embedder.model,
    )
    .await;
    ok &= passed;
    warnings += usize::from(warned);

    // ── Summary ──
    println!();
    if ok {
        if warnings > 0 {
            println!(
                "{} healthy with {warnings} warning{}",
                style("✔").green().bold(),
                if warnings == 1 { "" } else { "s" }
            );
        } else {
            println!("{} all checks passed", style("✔").green().bold());
        }
        0
    } else {
        println!("{} problems found", style("✘").red().bold());
        1
    }
}

/// Print the effective config (secrets redacted) relevant to diagnosis.
fn print_config(config: &Config) {
    let redact = |k: &Option<String>| {
        if k.as_ref().is_some_and(|s| !s.is_empty()) {
            "***".to_string()
        } else {
            "not set".to_string()
        }
    };
    println!("    backend:   {}", config.storage.backend);
    match config.storage.backend {
        StorageBackend::Libsql => {
            println!("    db path:   {}", config.storage.libsql_path);
        }
        StorageBackend::Postgres => {
            println!(
                "    db url:    {}",
                config.storage.database_url.as_deref().unwrap_or("not set")
            );
        }
    }
    println!(
        "    llm:       {} ({})",
        config.llm.base_url, config.llm.model
    );
    println!("    llm key:   {}", redact(&config.llm.api_key));
    println!(
        "    embedder:  {} ({})",
        config.embedder.base_url, config.embedder.model
    );
    println!("    emb key:   {}", redact(&config.embedder.api_key));
    if let Some(d) = config.dimension {
        println!("    dim:       {d} (configured)");
    }
}

/// Result of a non-destructive storage inspection.
struct StorageInspection {
    passed: bool,
    warnings: usize,
    lines: Vec<(bool, String)>, // (is_warning, text)
}

impl StorageInspection {
    fn print(&self) {
        for (warn, line) in &self.lines {
            if *warn {
                println!("  {} {line}", style("⚠").yellow().bold());
            } else {
                println!("  {} {line}", style("✔").green().bold());
            }
        }
    }
}

/// Inspect the configured storage backend without opening it for writes.
async fn inspect_storage(config: &Config) -> Result<StorageInspection, String> {
    match config.storage.backend {
        StorageBackend::Libsql => {
            let info = memayu_storage_libsql::LibsqlProvider::inspect(&config.storage.libsql_path)
                .await
                .map_err(|e| format!("failed to inspect libsql store: {e}"))?;
            let mut passed = true;
            let mut warnings = 0usize;
            let mut lines = Vec::new();
            if info.database_exists {
                lines.push((
                    false,
                    format!("database exists at {}", config.storage.libsql_path),
                ));
                if let Some(dim) = info.dimension {
                    lines.push((false, format!("schema present (dimension {dim})")));
                    if let Some(configured) = config.dimension {
                        if configured != dim {
                            warnings += 1;
                            passed = false;
                            lines.push((
                                true,
                                format!(
                                    "configured dimension {configured} differs from stored {dim}"
                                ),
                            ));
                        }
                    }
                } else {
                    warnings += 1;
                    lines.push((
                        true,
                        "schema not created yet — created automatically on first run".to_string(),
                    ));
                }
            } else {
                warnings += 1;
                lines.push((
                    true,
                    format!(
                        "no database file at {} — created automatically on first run",
                        config.storage.libsql_path
                    ),
                ));
            }
            Ok(StorageInspection {
                passed,
                warnings,
                lines,
            })
        }
        StorageBackend::Postgres => {
            let url = config.storage.database_url.as_deref().unwrap_or_default();
            if url.is_empty() {
                return Ok(StorageInspection {
                    passed: false,
                    warnings: 0,
                    lines: vec![(
                        false,
                        "database_url is missing — required for postgres backend".to_string(),
                    )],
                });
            }
            let info = memayu_storage_postgres::PostgresProvider::inspect(url)
                .await
                .map_err(|e| format!("failed to connect to postgres: {e}"))?;
            let mut passed = true;
            let mut warnings = 0usize;
            let mut lines = Vec::new();
            lines.push((false, "connected to postgres".to_string()));
            if info.schema_exists {
                lines.push((false, "schema present".to_string()));
                if let Some(dim) = info.dimension {
                    if let Some(configured) = config.dimension {
                        if configured != dim {
                            warnings += 1;
                            passed = false;
                            lines.push((
                                true,
                                format!(
                                    "configured dimension {configured} differs from stored {dim}"
                                ),
                            ));
                        }
                    }
                }
            } else {
                warnings += 1;
                lines.push((
                    true,
                    "memories table missing — created automatically on first run".to_string(),
                ));
            }
            Ok(StorageInspection {
                passed,
                warnings,
                lines,
            })
        }
    }
}

/// Probe a provider's `GET /models` endpoint, print the outcome, and return
/// `(passed, warned)` so the caller can fold it into the overall result.
async fn check_provider(
    name: &str,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
) -> (bool, bool) {
    match memayu_llm_client::check_models(base_url, api_key, model).await {
        memayu_llm_client::ModelsCheck::Ok { model_available } => {
            if model_available {
                println!(
                    "  {} reachable, key accepted, model \"{model}\" advertised",
                    style("✔").green().bold()
                );
                (true, false)
            } else {
                println!(
                    "  {} reachable, key accepted, but \"{model}\" not in /models list",
                    style("⚠").yellow().bold()
                );
                (true, true)
            }
        }
        memayu_llm_client::ModelsCheck::Unauthorized => {
            println!(
                "  {} API key rejected (401) for {name}",
                style("✘").red().bold()
            );
            (false, false)
        }
        memayu_llm_client::ModelsCheck::Http { status, detail } => {
            println!(
                "  {} {name} returned HTTP {status}: {}",
                style("✘").red().bold(),
                truncate(detail)
            );
            (false, false)
        }
        memayu_llm_client::ModelsCheck::Unreachable(msg) => {
            println!(
                "  {} cannot reach {name} at {base_url}: {}",
                style("✘").red().bold(),
                truncate(msg)
            );
            (false, false)
        }
    }
}

fn truncate(s: String) -> String {
    const MAX: usize = 160;
    if s.len() <= MAX {
        s
    } else {
        format!("{}…", &s[..MAX])
    }
}

/// Render a section header like `[ config ]`.
fn section(name: &str) -> String {
    format!("[ {} ]", style(name).cyan().bold())
}
