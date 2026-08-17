//! End-to-end tests for the non-interactive CLI (`add`, `search`, `list`,
//! `get`, `delete`, `--version`) against a mock embedder and a throwaway
//! libsql store. Each test spawns the real `memayu` binary as a subprocess.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};

/// Fixed embedding vector returned by the mock embedder. Using a non-trivial
/// vector keeps cosine similarity meaningful for `search`.
const EMB: [f32; 3] = [0.1, 0.2, 0.3];

/// Bind an HTTP mock embedder that answers every request to `/embeddings`
/// with a fixed embedding. Returns the bound port.
fn start_mock_embedder() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock embedder");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // Drain the request headers so the client sees a complete exchange.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let body = format!(
                r#"{{"data":[{{"embedding":[{:.6},{:.6},{:.6}]}}]}}"#,
                EMB[0], EMB[1], EMB[2]
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    port
}

/// Like [`start_mock_embedder`] but derives a distinct embedding from each
/// request's full body. Used by batch tests where every item must get its own
/// vector so the storage layer doesn't treat them as duplicates and merge them.
fn start_distinct_embedder() -> u16 {
    use std::hash::{Hash, Hasher};
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind distinct embedder");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // Read the full request: headers plus Content-Length bytes of body.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            loop {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let header = String::from_utf8_lossy(&buf[..header_end]).into_owned();
                            let content_length = header
                                .lines()
                                .find_map(|line| {
                                    let mut parts = line.splitn(2, ':');
                                    if parts
                                        .next()
                                        .map(|k| k.trim().eq_ignore_ascii_case("content-length"))
                                        == Some(true)
                                    {
                                        parts.next()?.trim().parse::<usize>().ok()
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(0);
                            if buf.len() >= header_end + 4 + content_length {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            let mut h = std::collections::hash_map::DefaultHasher::new();
            buf.hash(&mut h);
            let v = h.finish();
            let emb = [
                ((v & 0xffff) as f32) / 65535.0,
                (((v >> 16) & 0xffff) as f32) / 65535.0,
                (((v >> 32) & 0xffff) as f32) / 65535.0,
            ];
            let body = format!(
                r#"{{"data":[{{"embedding":[{:.6},{:.6},{:.6}]}}]}}"#,
                emb[0], emb[1], emb[2]
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    port
}

/// A temp libsql db path unique per test process.
fn temp_db_path() -> std::path::PathBuf {
    static COUNTER: AtomicU16 = AtomicU16::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("memayu-cli-test-{}-{n}.db", std::process::id()))
}

/// Run the real `memayu` binary with an isolated raw-mode config pointing at
/// the given mock embedder port and libsql path. Returns
/// (exit_code, stdout, stderr).
fn run_memayu_full(args: &[&str], port: u16, db: &std::path::Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_memayu"))
        .args(args)
        .env("MEMAYU_EXTRACTION_MODE", "raw")
        .env(
            "MEMAYU_EMBEDDER_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .env("MEMAYU_EMBEDDER_MODEL", "test-embed")
        .env("MEMAYU_EMBEDDER_DIM", "3")
        .env("MEMAYU_LIBSQL_PATH", db)
        .env("MEMAYU_CONFIG", db.with_extension("toml")) // non-existent: keep env-only config
        .env_remove("MEMAYU_API_URL")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn memayu binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    eprintln!("exit={} stderr={stderr}", out.status.code().unwrap_or(-1));
    (out.status.code().unwrap_or(-1), stdout, stderr)
}

/// Convenience wrapper returning only (exit_code, stdout).
fn run_memayu(args: &[&str], port: u16, db: &std::path::Path) -> (i32, String) {
    let (code, stdout, _stderr) = run_memayu_full(args, port, db);
    (code, stdout)
}

#[test]
fn version_flag_prints_version() {
    let (code, stdout) = run_memayu(&["--version"], 1, &temp_db_path());
    assert_eq!(code, 0);
    assert!(stdout.trim_start().starts_with("memayu "));
}

#[test]
fn add_then_list_get_search_delete_roundtrip() {
    let port = start_mock_embedder();
    let db = temp_db_path();
    let content = "the quick brown fox";

    // add
    let (code, stdout) = run_memayu(&["add", content], port, &db);
    assert_eq!(code, 0, "add should succeed");
    let stored = stdout.trim();
    let id = stored.strip_prefix("stored: ").expect("add prints id");
    assert!(!id.is_empty());

    // list (plain)
    let (code, stdout) = run_memayu(&["list"], port, &db);
    assert_eq!(code, 0);
    assert!(
        stdout.contains(content),
        "list should contain the added content"
    );

    // list --json
    let (code, stdout) = run_memayu(&["list", "--json"], port, &db);
    assert_eq!(code, 0);
    let obj: serde_json::Value = serde_json::from_str(stdout.trim()).expect("list --json is JSON");
    let arr = obj["memories"].as_array().expect("memories is an array");
    assert!(
        obj["total"].as_u64().is_some(),
        "list exposes the total row count"
    );
    assert!(
        obj.get("next_cursor").is_some(),
        "list exposes next_cursor (null on the last page)"
    );
    assert!(arr.iter().any(|m| m["content"] == content));
    assert!(arr.iter().any(|m| m["id"] == id));

    // search --json (fixed embeddings -> similarity 1.0, above threshold)
    let (code, stdout) = run_memayu(&["search", "fox", "--json"], port, &db);
    assert_eq!(code, 0);
    let arr: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("search --json is JSON");
    let arr = arr.as_array().expect("search --json is an array");
    assert!(
        !arr.is_empty(),
        "search should return at least the stored memory"
    );
    assert!(arr.iter().any(|m| m["id"] == id));
    assert!(arr.iter().all(|m| m.get("score").is_some()));

    // get
    let (code, stdout) = run_memayu(&["get", id], port, &db);
    assert_eq!(code, 0);
    assert!(stdout.contains(content));

    // delete
    let (code, stdout) = run_memayu(&["delete", id], port, &db);
    assert_eq!(code, 0);
    assert!(stdout.contains("deleted:"));

    // list after delete -> gone
    let (code, stdout) = run_memayu(&["list", "--json"], port, &db);
    assert_eq!(code, 0);
    let obj: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(obj["memories"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["id"] != id));
}

#[test]
fn unknown_flag_and_invalid_values_are_rejected() {
    let port = start_mock_embedder();
    let db = temp_db_path();

    // Unknown flag is rejected instead of being silently swallowed.
    let (code, _stdout, stderr) = run_memayu_full(&["list", "--bogus"], port, &db);
    assert_ne!(code, 0);
    assert!(
        stderr.contains("unknown option: --bogus"),
        "stderr: {stderr}"
    );

    // Unknown option flag (one that consumes a value) is also rejected.
    let (code, _stdout, stderr) = run_memayu_full(&["list", "--bogus", "x"], port, &db);
    assert_ne!(code, 0);
    assert!(
        stderr.contains("unknown option: --bogus"),
        "stderr: {stderr}"
    );

    // Zero limit is rejected.
    let (code, _stdout, stderr) = run_memayu_full(&["list", "--limit", "0"], port, &db);
    assert_ne!(code, 0);
    assert!(stderr.contains("must be at least 1"), "stderr: {stderr}");

    // Over-cap limit is rejected with a clear message.
    let (code, _stdout, stderr) = run_memayu_full(&["list", "--limit", "999"], port, &db);
    assert_ne!(code, 0);
    assert!(stderr.contains("maximum is 100"), "stderr: {stderr}");

    // Non-numeric limit gets a clear message.
    let (code, _stdout, stderr) = run_memayu_full(&["list", "--limit", "abc"], port, &db);
    assert_ne!(code, 0);
    assert!(stderr.contains("invalid --limit value"), "stderr: {stderr}");

    // Malformed --filter is rejected.
    let (code, _stdout, stderr) = run_memayu_full(&["list", "--filter", "nope"], port, &db);
    assert_ne!(code, 0);
    assert!(stderr.contains("expected key=value"), "stderr: {stderr}");
}

#[test]
fn list_exposes_paging_contract() {
    let port = start_mock_embedder();
    let db = temp_db_path();

    // Empty store: total 0, explicit null next_cursor.
    let (code, stdout) = run_memayu(&["list", "--json"], port, &db);
    assert_eq!(code, 0);
    let obj: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(obj["memories"].as_array().unwrap().len(), 0);
    assert_eq!(obj["total"].as_u64(), Some(0));
    assert!(obj["next_cursor"].is_null());

    // A metadata filter runs and filters to the empty match set without error.
    let (code, stdout) = run_memayu(&["list", "--json", "--filter", "source=cli"], port, &db);
    assert_eq!(code, 0);
    let obj: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(obj["memories"].as_array().unwrap().len(), 0);
    assert_eq!(obj["total"].as_u64(), Some(0));
}

#[test]
fn add_batch_stores_all_items() {
    let port = start_distinct_embedder();
    let db = temp_db_path();
    let batch = temp_db_path().with_extension("batch.jsonl");
    std::fs::write(
        &batch,
        concat!(
            "{\"content\":\"alpha\",\"metadata\":{\"source\":\"batch\"}}\n",
            "{\"content\":\"beta\"}\n",
            "{\"content\":\"gamma\"}\n",
        ),
    )
    .unwrap();

    let (code, stdout) = run_memayu(&["add", "--batch", batch.to_str().unwrap()], port, &db);
    assert_eq!(code, 0, "batch add should succeed");
    assert_eq!(stdout.matches("stored:").count(), 3);
    assert!(stdout.contains("added: 3 memory(ies)"), "stdout: {stdout}");

    // All three are persisted.
    let (code, stdout) = run_memayu(&["list", "--json"], port, &db);
    assert_eq!(code, 0);
    let obj: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    let contents: Vec<String> = obj["memories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap().to_string())
        .collect();
    for c in ["alpha", "beta", "gamma"] {
        assert!(
            contents.iter().any(|x| x == c),
            "missing {c} in {contents:?}"
        );
    }
}

#[test]
fn add_batch_reports_partial_failures() {
    let port = start_distinct_embedder();
    let db = temp_db_path();
    let batch = temp_db_path().with_extension("batch.jsonl");
    std::fs::write(&batch, "{\"content\":\"ok\"}\n{\"content\":\"   \"}\n").unwrap();

    let (code, stdout, stderr) =
        run_memayu_full(&["add", "--batch", batch.to_str().unwrap()], port, &db);
    // The valid item is stored, but the blank one fails and we exit non-zero.
    assert_ne!(code, 0, "partial failure should exit non-zero");
    assert_eq!(stdout.matches("stored:").count(), 1);
    assert!(
        stderr.contains("failed: memory content is required"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("1 item(s) failed"), "stderr: {stderr}");
}
