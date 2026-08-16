//! CLI-interactive setup wizard (dialoguer, plain stdin/stdout) — the default
//! presentation of the shared setup flow (#54).
//!
//! `memayu setup` runs this presenter. It reads an existing config file (if
//! present) as defaults for re-configuration, walks the exact same ordered
//! [`crate::setup_flow::SETUP_STEPS`] as the `--tui` presenter, and hands the
//! collected answers to [`finalize`], which writes the config, creates the
//! admin account, and generates the API key.

use dialoguer::{theme::ColorfulTheme, Input, Select};
use memayu_config::{config_path, read_config_file, StorageBackend};
use memayu_llm_client::local_embedder::DEFAULT_MODEL_ID;
use std::io::IsTerminal;

use crate::setup_flow::{
    check_device, finalize, fmt_bytes, fmt_cpu, step_active, step_title, DeviceReport,
    SetupAnswers, SetupStep, LOCAL_MODELS, LOCAL_MODEL_NAMES, SETUP_STEPS,
};

fn prompt(label: &str, default: &str) -> String {
    Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt(label)
        .default(default.to_string())
        .interact_text()
        .unwrap_or_else(|_| default.to_string())
}

fn select(label: &str, items: &[&str], default: usize) -> usize {
    Select::with_theme(&ColorfulTheme::default())
        .with_prompt(label)
        .items(items)
        .default(default)
        .interact()
        .unwrap_or(default)
}

/// Run the CLI-interactive setup wizard. `preseed` toggles whether an existing
/// config file is used to pre-fill defaults (re-configuration).
pub async fn run_cli_setup(preseed: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        console::style("  memayu setup  — first-run configuration wizard")
            .bold()
            .cyan()
    );
    println!();

    let existing = if preseed {
        read_config_file(&config_path()).ok().flatten()
    } else {
        None
    };
    let mut a = crate::setup_flow::preseed(existing.as_ref());
    // Probe the device once up front so the DeviceCheck step and the local
    // embedder gating share the same report.
    a.device = check_device();

    // Walk the shared step list, skipping branch-inactive steps.
    for &step in SETUP_STEPS {
        if !step_active(step, &a) {
            continue;
        }
        render_step(step, &mut a)?;
    }

    println!();
    let result = finalize(&a).await?;
    println!(
        "{} Written to {}",
        console::style("✔").green().bold(),
        result.config_path.display()
    );
    println!();
    println!("{}", console::style("API key (shown once)").bold().cyan());
    println!("  {}", console::style(&result.api_key).bold().yellow());
    println!(
        "{} store it now — it cannot be retrieved again",
        console::style("→").yellow()
    );
    Ok(())
}

fn render_step(step: SetupStep, a: &mut SetupAnswers) -> Result<(), Box<dyn std::error::Error>> {
    match step {
        SetupStep::DeviceCheck => {
            print_device_report(&a.device);
            // Pause only when interactive so the report is readable; skip the
            // keypress when headless/piped (no TTY) to avoid blocking.
            if std::io::stdin().is_terminal() {
                let _ = Input::<String>::with_theme(&ColorfulTheme::default())
                    .with_prompt("Press Enter to continue")
                    .allow_empty(true)
                    .interact_text();
            }
        }
        SetupStep::StorageBackend => {
            let items = ["libsql", "postgres"];
            let default = if a.storage_backend == StorageBackend::Postgres {
                1
            } else {
                0
            };
            let idx = select(step_title(step), &items, default);
            a.storage_backend = if items[idx] == "postgres" {
                StorageBackend::Postgres
            } else {
                StorageBackend::Libsql
            };
        }
        SetupStep::StoragePath => match a.storage_backend {
            StorageBackend::Libsql => {
                a.libsql_path = prompt("libsql database path", &a.libsql_path);
            }
            StorageBackend::Postgres => {
                let default = if a.database_url.is_empty() {
                    "postgres://localhost/memayu"
                } else {
                    &a.database_url
                };
                a.database_url = prompt("Postgres connection URL", default);
            }
        },
        SetupStep::EmbedderBackend => {
            if a.device.local_supported {
                let items = ["local", "http"];
                let default = if a.embedder_backend == "http" { 1 } else { 0 };
                let idx = select(
                    "Embedding backend (local = on-device Candle, http = bring-your-own-key)",
                    &items,
                    default,
                );
                a.embedder_backend = items[idx].to_string();
                if a.embedder_backend == "local" {
                    a.embedder_model = DEFAULT_MODEL_ID.to_string();
                }
            } else {
                println!(
                    "{} local embedding is not supported on this device, using HTTP embedder.",
                    console::style("→").yellow()
                );
                a.embedder_backend = "http".to_string();
            }
        }
        SetupStep::LocalModel => {
            print_local_model_table();
            let default = LOCAL_MODELS
                .iter()
                .position(|m| m.id == a.embedder_model)
                .unwrap_or(0);
            let idx = select("Local embedding model", LOCAL_MODEL_NAMES, default);
            a.embedder_model = LOCAL_MODELS[idx].id.to_string();
        }
        SetupStep::EmbedderConfig => {
            a.embedder_base_url = prompt("Base URL", &a.embedder_base_url);
            a.embedder_api_key = prompt(
                "API key (optional, press enter to skip)",
                &a.embedder_api_key,
            );
            a.embedder_model = prompt("Model", &a.embedder_model);
        }
        SetupStep::ExtractionMode => {
            let items = ["llm", "raw"];
            let default = if a.extraction_mode == "raw" { 1 } else { 0 };
            let idx = select(step_title(step), &items, default);
            a.extraction_mode = items[idx].to_string();
        }
        SetupStep::LlmConfig => {
            a.llm_base_url = prompt("Base URL", &a.llm_base_url);
            a.llm_api_key = prompt("API key (optional, press enter to skip)", &a.llm_api_key);
            a.llm_model = prompt("Model", &a.llm_model);
        }
        SetupStep::AdminEmail => {
            a.admin_email = prompt("Admin email", &a.admin_email);
        }
        SetupStep::AdminPassword => {
            a.admin_password = prompt("Password (min 8 chars, uppercase+lowercase+digit)", "");
        }
        SetupStep::AdminConfirm => loop {
            let confirm = prompt("Confirm password", "");
            if confirm == a.admin_password {
                break;
            }
            eprintln!(
                "{} passwords do not match, try again",
                console::style("✘").red()
            );
        },
        SetupStep::BindAddr => {
            a.bind_addr = prompt("Bind address", &a.bind_addr);
        }
        SetupStep::Port => {
            let s = prompt("Port", &a.port.to_string());
            a.port = s.parse().unwrap_or(18080);
        }
        SetupStep::ApiKeyLabel => {
            a.api_key_label = prompt("API key name", &a.api_key_label);
        }
    }
    Ok(())
}

/// Print the device capability report from the DeviceCheck step.
fn print_device_report(d: &DeviceReport) {
    println!();
    println!("{} Device check", console::style("•").bold().cyan());
    println!("  OS / arch   {} / {}", d.os, d.arch);
    println!("  CPU         {}", fmt_cpu(d));
    println!("  RAM          {}", fmt_bytes(d.ram_bytes));
    println!("  Free disk   {}", fmt_bytes(d.free_disk_bytes));
    if d.local_supported {
        println!("  Local embed {}", console::style("supported").green());
    } else {
        println!(
            "  Local embed {}",
            console::style("NOT supported").red().bold()
        );
        for issue in &d.issues {
            println!("     - {issue}");
        }
    }
    println!();
}

/// Print the local embedding model comparison table before the model picker.
fn print_local_model_table() {
    println!();
    println!(
        "{} Local embedding models",
        console::style("Model").bold().cyan()
    );
    println!(
        "  {:<38} {:>4} {:>8} {:>8} {:>9} | CPU / Lang",
        "Name", "Dim", "fp32", "int8", "Min RAM"
    );
    for m in LOCAL_MODELS {
        println!(
            "  {:<38} {:>4} {:>7}M {:>7}M {:>8}M | {} / {}",
            m.name, m.dim, m.fp32_size_mb, m.int8_size_mb, m.min_ram_mb, m.cpu_notes, m.langs
        );
    }
    println!();
}
