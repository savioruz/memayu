//! Non-interactive CLI subcommands: `add`, `search`, `list`, `get`, `delete`
//! and `--version`.
//!
//! These work in-process via [`memayu-core`] (the same service layer as the TUI
//! and MCP frontends), so no web server or MCP client is needed. They are
//! single-user by design: the user id comes from the config (default `default`)
//! and no auth is required for a local instance.

use memayu_config::Config;
use std::collections::HashMap;

use crate::service::build_service;
use memayu_core::Memory;

/// Default user id for the local single-user CLI.
pub const DEFAULT_USER_ID: &str = "default";

/// Print the binary version (from `CARGO_PKG_VERSION`) and exit successfully.
pub fn cmd_version() {
    println!("memayu {}", env!("CARGO_PKG_VERSION"));
}

/// The CLI commands must run against a local store; they have no cloud client.
fn guard_local(config: &Config) -> Result<(), String> {
    if config.api_url.is_some() {
        Err(
            "error: this command requires local config — MEMAYU_API_URL is set (cloud mode)"
                .to_string(),
        )
    } else {
        Ok(())
    }
}

/// Split trailing args into positional values and `--flag[=value]` options.
///
/// `--limit` consumes the following argument as its value so both `--limit 5`
/// and `--limit=5` work. All other `--flag`s are treated as boolean switches.
fn parse_args(
    args: impl Iterator<Item = String>,
) -> (Vec<String>, HashMap<String, Option<String>>) {
    let mut positionals = Vec::new();
    let mut flags = HashMap::new();
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        if let Some(rest) = a.strip_prefix("--") {
            if let Some((k, v)) = rest.split_once('=') {
                flags.insert(k.to_string(), Some(v.to_string()));
            } else if rest == "limit" {
                let v = it.next().unwrap_or_default();
                flags.insert("limit".to_string(), Some(v));
            } else {
                flags.insert(rest.to_string(), None);
            }
        } else {
            positionals.push(a);
        }
    }
    (positionals, flags)
}

fn flag_bool(flags: &HashMap<String, Option<String>>, name: &str) -> bool {
    flags.contains_key(name)
}

fn flag_limit(flags: &HashMap<String, Option<String>>, default: usize) -> Result<usize, String> {
    match flags.get("limit").and_then(|v| v.clone()) {
        None => Ok(default),
        Some(s) => s
            .parse()
            .map_err(|_| format!("invalid --limit value: {s:?}")),
    }
}

/// `memayu add "<content>"` — run ADD/UPDATE extraction and store the memory.
pub async fn cmd_add(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let (positionals, _) = parse_args(args);
    let content = positionals.join(" ").trim().to_string();
    if content.is_empty() {
        return Err("usage: memayu add \"<content>\"".to_string());
    }
    guard_local(config)?;
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    let mem = service
        .add_memory(DEFAULT_USER_ID, &content, &Default::default())
        .await
        .map_err(|e| e.to_string())?;
    println!("stored: {}", mem.id);
    Ok(())
}

/// `memayu search "<query>" [--limit N] [--json]` — ranked semantic results.
pub async fn cmd_search(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let (positionals, flags) = parse_args(args);
    let query = positionals.join(" ").trim().to_string();
    if query.is_empty() {
        return Err("usage: memayu search \"<query>\" [--limit N] [--json]".to_string());
    }
    guard_local(config)?;
    let limit = flag_limit(&flags, 5)?;
    let json = flag_bool(&flags, "json");
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    let results = service
        .search_memory(DEFAULT_USER_ID, &query, limit)
        .await
        .map_err(|e| e.to_string())?;
    if json {
        let arr: Vec<serde_json::Value> = results
            .iter()
            .map(|(m, s)| memory_json(m, Some(*s)))
            .collect();
        println!(
            "{}",
            serde_json::to_string(&arr).map_err(|e| e.to_string())?
        );
    } else {
        for (m, score) in &results {
            println!("{:.4}  {}", score, m.content);
        }
    }
    Ok(())
}

/// `memayu list [--limit N] [--json]` — recent memories.
pub async fn cmd_list(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let (_positionals, flags) = parse_args(args);
    guard_local(config)?;
    let limit = flag_limit(&flags, 50)?;
    let json = flag_bool(&flags, "json");
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    let mems = service
        .list_memories(DEFAULT_USER_ID, limit)
        .await
        .map_err(|e| e.to_string())?;
    if json {
        let arr: Vec<serde_json::Value> = mems.iter().map(|m| memory_json(m, None)).collect();
        println!(
            "{}",
            serde_json::to_string(&arr).map_err(|e| e.to_string())?
        );
    } else {
        for m in &mems {
            println!("{}\t{}", m.id, m.content);
        }
    }
    Ok(())
}

/// `memayu get <memory_id> [--json]` — fetch one memory by id.
pub async fn cmd_get(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let (positionals, flags) = parse_args(args);
    let id = positionals.first().cloned().unwrap_or_default();
    if id.is_empty() {
        return Err("usage: memayu get <memory_id>".to_string());
    }
    guard_local(config)?;
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    let m = service.get_memory(&id).await.map_err(|e| e.to_string())?;
    print_memory(&m, flag_bool(&flags, "json"))?;
    Ok(())
}

/// `memayu delete <memory_id>` — remove a memory by id.
pub async fn cmd_delete(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let (positionals, _) = parse_args(args);
    let id = positionals.first().cloned().unwrap_or_default();
    if id.is_empty() {
        return Err("usage: memayu delete <memory_id>".to_string());
    }
    guard_local(config)?;
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    service
        .delete_memory(&id)
        .await
        .map_err(|e| e.to_string())?;
    println!("deleted: {id}");
    Ok(())
}

/// Serialize a memory (with an optional search score) for `--json` output.
fn memory_json(m: &Memory, score: Option<f32>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let Some(s) = score {
        obj.insert("score".to_string(), serde_json::json!(s));
    }
    obj.insert("id".to_string(), serde_json::json!(m.id));
    obj.insert("content".to_string(), serde_json::json!(m.content));
    obj.insert(
        "created_at".to_string(),
        serde_json::json!(m.created_at.to_rfc3339()),
    );
    obj.insert(
        "updated_at".to_string(),
        serde_json::json!(m.updated_at.to_rfc3339()),
    );
    if !m.metadata.is_empty() {
        obj.insert(
            "metadata".to_string(),
            serde_json::to_value(&m.metadata).unwrap_or(serde_json::Value::Null),
        );
    }
    serde_json::Value::Object(obj)
}

/// Print a single memory in human-readable or JSON form.
fn print_memory(m: &Memory, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string(&memory_json(m, None)).map_err(|e| e.to_string())?
        );
    } else {
        println!("id:        {}", m.id);
        println!("content:   {}", m.content);
        println!("created:   {}", m.created_at);
        if !m.metadata.is_empty() {
            println!(
                "metadata:  {}",
                serde_json::to_string(&m.metadata).map_err(|e| e.to_string())?
            );
        }
    }
    Ok(())
}
