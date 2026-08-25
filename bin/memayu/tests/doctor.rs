//! End-to-end tests for `memayu doctor` against a mock provider and a
//! throwaway libsql store. Each test spawns the real `memayu` binary as a
//! subprocess and asserts on its exit code.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};

const EMB: [f32; 3] = [0.1, 0.2, 0.3];

/// How the mock provider should answer real test calls.
#[derive(Clone, Copy)]
enum ProviderBehavior {
    /// `/embeddings` and `/chat/completions` both succeed. There is no
    /// `/models` endpoint at all (proxy scenario).
    Good,
    /// 401 on real calls (bad API key).
    Unauthorized,
    /// 500 on real calls (provider misconfigured).
    ServerError,
}

/// Bind an HTTP mock that has NO `/models` endpoint and answers the real test
/// calls (`POST /embeddings`, `POST /chat/completions`) according to
/// `behavior`. Returns the bound port.
fn start_mock(behavior: ProviderBehavior) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
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

            let head = String::from_utf8_lossy(&buf);
            let (status, body) = if head.starts_with("GET /models") {
                // No /models endpoint: the proxy exposes only the real call
                // surface. This is exactly what issue #48 exercises.
                ("404 Not Found", r#"{"error":"not found"}"#.to_string())
            } else {
                match behavior {
                    ProviderBehavior::Good => {
                        if head.starts_with("POST /embeddings") {
                            (
                                "200 OK",
                                format!(
                                    r#"{{"data":[{{"embedding":[{:.6},{:.6},{:.6}]}}]}}"#,
                                    EMB[0], EMB[1], EMB[2]
                                ),
                            )
                        } else {
                            // /chat/completions
                            (
                                "200 OK",
                                r#"{"choices":[{"message":{"content":"{\"decision\":\"add\",\"memory_id\":null,\"content\":\"ok\"}"}}]}"#.to_string(),
                            )
                        }
                    }
                    ProviderBehavior::Unauthorized => ("401 Unauthorized", String::new()),
                    ProviderBehavior::ServerError => ("500 Internal Server Error", String::new()),
                }
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    port
}

fn temp_db_path() -> std::path::PathBuf {
    static COUNTER: AtomicU16 = AtomicU16::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("memayu-doctor-test-{}-{n}.db", std::process::id()))
}

/// Run `memayu doctor` with an isolated raw-mode config pointing at the mock.
/// Returns (exit_code, stdout, stderr).
fn run_doctor(
    port: u16,
    db: &std::path::Path,
    extra_env: &[(&str, &str)],
) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_memayu"));
    cmd.arg("doctor")
        .env("MEMAYU_EXTRACTION_MODE", "raw")
        .env(
            "MEMAYU_EMBEDDER_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .env("MEMAYU_EMBEDDER_MODEL", "test-embed")
        .env("MEMAYU_EMBEDDER_DIM", "3")
        .env("MEMAYU_LIBSQL_PATH", db)
        .env("MEMAYU_CONFIG", db.with_extension("toml"))
        .env_remove("MEMAYU_API_URL");
    for (k, v) in extra_env {
        cmd.env(*k, *v);
    }
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn memayu binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    eprintln!("exit={} stderr={stderr}", out.status.code().unwrap_or(-1));
    eprintln!("stdout:\n{stdout}");
    (out.status.code().unwrap_or(-1), stdout, stderr)
}

/// `memayu list` once to create the libsql schema, simulating a used store.
fn prime_schema(port: u16, db: &std::path::Path) {
    let out = Command::new(env!("CARGO_BIN_EXE_memayu"))
        .arg("list")
        .env("MEMAYU_EXTRACTION_MODE", "raw")
        .env(
            "MEMAYU_EMBEDDER_BASE_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .env("MEMAYU_EMBEDDER_MODEL", "test-embed")
        .env("MEMAYU_EMBEDDER_DIM", "3")
        .env("MEMAYU_LIBSQL_PATH", db)
        .env("MEMAYU_CONFIG", db.with_extension("toml"))
        .env_remove("MEMAYU_API_URL")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("spawn memayu binary");
    assert_eq!(out.status.code().unwrap_or(-1), 0, "priming should succeed");
}

#[test]
fn doctor_fresh_machine_exits_zero() {
    let port = start_mock(ProviderBehavior::Good);
    let db = temp_db_path(); // does not exist yet
    let (code, stdout, _) = run_doctor(port, &db, &[]);
    assert_eq!(code, 0, "fresh machine should be healthy");
    assert!(stdout.contains("all checks passed") || stdout.contains("healthy"));
}

#[test]
fn doctor_used_store_exits_zero() {
    let port = start_mock(ProviderBehavior::Good);
    let db = temp_db_path();
    prime_schema(port, &db);
    let (code, stdout, _) = run_doctor(port, &db, &[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("schema present"), "stdout: {stdout}");
}

#[test]
fn doctor_bad_key_exits_one() {
    let port = start_mock(ProviderBehavior::Unauthorized);
    let db = temp_db_path();
    let (code, stdout, _) = run_doctor(port, &db, &[]);
    assert_eq!(code, 1, "rejected key must fail doctor");
    assert!(stdout.contains("401"), "stdout: {stdout}");
}

#[test]
fn doctor_proxy_without_models_endpoint_is_healthy() {
    // The mock has no /models endpoint at all (a proxy like 9Router), yet the
    // real embed test call succeeds. Doctor must report success (issue #48).
    let port = start_mock(ProviderBehavior::Good);
    let db = temp_db_path();
    let (code, stdout, _) = run_doctor(port, &db, &[]);
    assert_eq!(code, 0, "real test call must succeed through a proxy");
    assert!(
        stdout.contains("returned a 3-dim embedding"),
        "stdout: {stdout}"
    );
}

#[test]
fn doctor_server_error_exits_one() {
    let port = start_mock(ProviderBehavior::ServerError);
    let db = temp_db_path();
    let (code, stdout, _) = run_doctor(port, &db, &[]);
    assert_eq!(code, 1, "misconfigured provider must still fail doctor");
    assert!(stdout.contains("500"), "stdout: {stdout}");
}

#[test]
fn doctor_llm_proxy_without_models_endpoint_is_healthy() {
    // In llm mode doctor also exercises the LLM with a real completion. The
    // mock has no /models endpoint, but both real calls succeed.
    let port = start_mock(ProviderBehavior::Good);
    let db = temp_db_path();
    let (code, stdout, _) = run_doctor(
        port,
        &db,
        &[
            ("MEMAYU_EXTRACTION_MODE", "llm"),
            ("MEMAYU_LLM_BASE_URL", &format!("http://127.0.0.1:{port}")),
            ("MEMAYU_LLM_MODEL", "proxy-llm-model"),
        ],
    );
    assert_eq!(code, 0, "LLM test completion must succeed through a proxy");
    assert!(
        stdout.contains("answered a test completion"),
        "stdout: {stdout}"
    );
}

#[test]
fn doctor_missing_config_exits_one() {
    // No base URL, model, or dim: raw mode still requires an embedder.
    let db = temp_db_path();
    let (code, _stdout, stderr) = run_doctor(1, &db, &[("MEMAYU_EMBEDDER_MODEL", "")]);
    assert_eq!(code, 1, "broken config must fail doctor");
    assert!(stderr.contains("Could not load config"), "stderr: {stderr}");
}

#[test]
fn doctor_dimension_mismatch_fails() {
    let port = start_mock(ProviderBehavior::Good);
    let db = temp_db_path();
    prime_schema(port, &db); // stores dimension 3
                             // Run doctor with a conflicting configured dimension.
    let (code, stdout, _) = run_doctor(port, &db, &[("MEMAYU_EMBEDDER_DIM", "8")]);
    assert_eq!(code, 1, "dimension mismatch must fail doctor");
    assert!(
        stdout.contains("configured dimension 8 differs from stored 3"),
        "stdout: {stdout}"
    );
}
