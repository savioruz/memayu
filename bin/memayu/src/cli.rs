//! Non-interactive CLI subcommands: `add`, `search`, `list`, `get`, `delete`
//! and `--version`.
//!
//! These work in-process via [`memayu-core`] (the same service layer as the TUI
//! and MCP frontends), so no web server or MCP client is needed. They are
//! single-user by design: the user id comes from the config (default `default`)
//! and no auth is required for a local instance.

use memayu_config::Config;
use std::collections::{HashMap, HashSet};

use crate::service::build_service;
use memayu_core::{Memory, MetadataFilter, MAX_PAGE_SIZE};

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

/// Parsed CLI arguments: positional values plus option flags.
#[derive(Default)]
struct Parsed {
    positionals: Vec<String>,
    opts: HashMap<String, String>,
    filters: Vec<String>,
    switches: HashSet<String>,
}

/// Split trailing args into positional values and `--flag[=value]` options.
///
/// `--limit`, `--cursor`, and `--filter` consume the following argument as
/// their value so both `--limit 5` and `--limit=5` work. `--filter` is
/// repeatable; every other `--flag` is treated as a boolean switch.
fn parse_args(args: impl Iterator<Item = String>) -> Parsed {
    let mut p = Parsed::default();
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        if let Some(rest) = a.strip_prefix("--") {
            if let Some((k, v)) = rest.split_once('=') {
                insert_flag(&mut p, k, v.to_string());
            } else if matches!(rest, "limit" | "cursor" | "filter") {
                let v = it.next().unwrap_or_default();
                insert_flag(&mut p, rest, v);
            } else {
                p.switches.insert(rest.to_string());
            }
        } else {
            p.positionals.push(a);
        }
    }
    p
}

fn insert_flag(p: &mut Parsed, name: &str, value: String) {
    if name == "filter" {
        p.filters.push(value);
    } else {
        p.opts.insert(name.to_string(), value);
    }
}

fn flag_bool(p: &Parsed, name: &str) -> bool {
    p.switches.contains(name)
}

/// The `--limit` value with a helpful error on non-numeric, zero, or too-large
/// input, falling back to `default` when the flag is absent.
fn opt_limit(p: &Parsed, default: usize) -> Result<usize, String> {
    match p.opts.get("limit") {
        None => Ok(default),
        Some(s) => {
            let n: usize = s.parse().map_err(|_| {
                format!(
                    "invalid --limit value: {s:?} (expected a positive integer up to {MAX_PAGE_SIZE})"
                )
            })?;
            if n == 0 {
                return Err("invalid --limit value: \"0\" (must be at least 1)".to_string());
            }
            if n > MAX_PAGE_SIZE {
                return Err(format!(
                    "invalid --limit value: {n} (maximum is {MAX_PAGE_SIZE})"
                ));
            }
            Ok(n)
        }
    }
}

/// The `--filter key=value` predicates as a [`MetadataFilter`], or `None` when
/// no filter was given. Duplicate keys and malformed values are rejected.
fn metadata_filter(p: &Parsed) -> Result<Option<MetadataFilter>, String> {
    if p.filters.is_empty() {
        return Ok(None);
    }
    let mut m = MetadataFilter::new();
    for raw in &p.filters {
        let (k, v) = raw
            .split_once('=')
            .ok_or_else(|| format!("invalid --filter value: {raw:?} (expected key=value)"))?;
        if k.is_empty() {
            return Err(format!(
                "invalid --filter value: {raw:?} (expected key=value)"
            ));
        }
        if m.insert(k.to_string(), v.to_string()).is_some() {
            return Err(format!("duplicate --filter key: {k}"));
        }
    }
    Ok(Some(m))
}

/// The `--cursor` value, or `None` when absent/empty.
fn opt_cursor(p: &Parsed) -> Option<String> {
    p.opts.get("cursor").cloned().filter(|s| !s.is_empty())
}

/// Reject any flag the command does not understand, so typos surface as a
/// clear error instead of being silently swallowed as a boolean switch.
fn reject_unknown(
    p: &Parsed,
    allowed_opts: &[&str],
    allowed_switches: &[&str],
) -> Result<(), String> {
    let mut unknown = Vec::new();
    for k in p.opts.keys() {
        if !allowed_opts.contains(&k.as_str()) {
            unknown.push(format!("--{k}"));
        }
    }
    for k in &p.switches {
        if !allowed_switches.contains(&k.as_str()) {
            unknown.push(format!("--{k}"));
        }
    }
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!("unknown option: {}", unknown.join(", ")))
    }
}

/// `memayu add "<content>"` — run ADD/UPDATE extraction and store the memory.
pub async fn cmd_add(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let p = parse_args(args);
    reject_unknown(&p, &[], &[])?;
    let content = p.positionals.join(" ").trim().to_string();
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

/// `memayu search "<query>" [--limit N] [--filter key=value] [--json]` —
/// ranked semantic results.
pub async fn cmd_search(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let p = parse_args(args);
    reject_unknown(&p, &["limit"], &["json"])?;
    let query = p.positionals.join(" ").trim().to_string();
    if query.is_empty() {
        return Err(
            "usage: memayu search \"<query>\" [--limit N] [--filter key=value] [--json]"
                .to_string(),
        );
    }
    guard_local(config)?;
    let limit = opt_limit(&p, 5)?;
    let json = flag_bool(&p, "json");
    let filter = metadata_filter(&p)?;
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    let results = service
        .search_memory_filtered(DEFAULT_USER_ID, &query, limit, filter.as_ref())
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

/// `memayu list [--limit N] [--cursor C] [--filter key=value] [--json]` —
/// recent memories, with the paging contract (total + next_cursor) exposed.
pub async fn cmd_list(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let p = parse_args(args);
    reject_unknown(&p, &["limit", "cursor"], &["json"])?;
    guard_local(config)?;
    let limit = opt_limit(&p, 50)?;
    let json = flag_bool(&p, "json");
    let cursor = opt_cursor(&p);
    let filter = metadata_filter(&p)?;
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    let page = service
        .list_memories_paged(DEFAULT_USER_ID, limit, cursor.as_deref(), filter.as_ref())
        .await
        .map_err(|e| e.to_string())?;
    if json {
        let arr: Vec<serde_json::Value> =
            page.memories.iter().map(|m| memory_json(m, None)).collect();
        let obj = serde_json::json!({
            "memories": arr,
            "total": page.total,
            "next_cursor": page.next_cursor,
        });
        println!(
            "{}",
            serde_json::to_string(&obj).map_err(|e| e.to_string())?
        );
    } else {
        for m in &page.memories {
            println!("{}\t{}", m.id, m.content);
        }
        println!("total: {}", page.total);
        if let Some(nc) = &page.next_cursor {
            println!("next: {nc}");
        }
    }
    Ok(())
}

/// `memayu get <memory_id> [--json]` — fetch one memory by id.
pub async fn cmd_get(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let p = parse_args(args);
    reject_unknown(&p, &[], &["json"])?;
    let id = p.positionals.first().cloned().unwrap_or_default();
    if id.is_empty() {
        return Err("usage: memayu get <memory_id>".to_string());
    }
    guard_local(config)?;
    let (service, _) = build_service(config).await.map_err(|e| e.to_string())?;
    let m = service.get_memory(&id).await.map_err(|e| e.to_string())?;
    print_memory(&m, flag_bool(&p, "json"))?;
    Ok(())
}

/// `memayu delete <memory_id>` — remove a memory by id.
pub async fn cmd_delete(config: &Config, args: impl Iterator<Item = String>) -> Result<(), String> {
    let p = parse_args(args);
    reject_unknown(&p, &[], &[])?;
    let id = p.positionals.first().cloned().unwrap_or_default();
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
