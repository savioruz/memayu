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

/// A temp libsql db path unique per test process.
fn temp_db_path() -> std::path::PathBuf {
    static COUNTER: AtomicU16 = AtomicU16::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("memayu-cli-test-{}-{n}.db", std::process::id()))
}

/// Run the real `memayu` binary with an isolated raw-mode config pointing at
/// the given mock embedder port and libsql path. Returns (exit_code, stdout).
fn run_memayu(args: &[&str], port: u16, db: &std::path::Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_memayu"))
        .args(args)
        .env("MEMAYU_EXTRACTION_MODE", "raw")
        .env(
            "MEMAYU_EMBEDDER_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .env("MEMAYU_EMBEDDER_MODEL", "test-embed")
        .env("MEMAYU_EMBEDDING_DIM", "3")
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
    (out.status.code().unwrap_or(-1), stdout)
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
    let arr: serde_json::Value = serde_json::from_str(stdout.trim()).expect("list --json is JSON");
    let arr = arr.as_array().expect("list --json is an array");
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
    let arr: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(arr.as_array().unwrap().iter().all(|m| m["id"] != id));
}
