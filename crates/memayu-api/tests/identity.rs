//! Regression test for #32: TUI, Web, and in-process MCP frontends must all
//! resolve to the *identical* admin account_id for a given self-hosted
//! instance, instead of the TUI/local-MCP hardcoding `"default"`.
//!
//! The Web path resolves the account from a session token (the exact path the
//! auth middleware uses). The TUI/local-MCP path resolves it from the `users`
//! table via `memayu_identity`. This test proves they agree on a fresh
//! first-run setup.

use memayu_api::auth_dto::SetupRequest;
use memayu_api::{open_db, WebServices};
use memayu_config::{StorageBackend, StorageConfig};
use memayu_identity::resolve_self_hosted_account_id;
use std::sync::atomic::{AtomicU64, Ordering};

/// A unique on-disk libsql path. A file (not `:memory:`) is required so the
/// separate connections opened by `open_db` and `memayu_identity` see the same
/// database.
fn temp_config() -> StorageConfig {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("memayu-identity-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let n = N.fetch_add(1, Ordering::Relaxed);
    StorageConfig {
        backend: StorageBackend::Libsql,
        libsql_path: dir.join(format!("db-{n}.db")).display().to_string(),
        database_url: None,
    }
}

#[tokio::test]
async fn tui_web_mcp_resolve_to_same_admin_account() {
    let config = temp_config();
    let db = open_db(&config).await.unwrap();
    let ws = WebServices::new(db.clone());

    // First-run setup (Web path) creates the single admin account + session.
    let (_resp, session_token) = ws
        .auth_setup(&SetupRequest {
            email: "admin@example.com".into(),
            password: "Correct-Horse-Battery-Staple-42!".into(),
            confirm: "Correct-Horse-Battery-Staple-42!".into(),
        })
        .await
        .map_err(|e| format!("setup failed: {} {}", e.error, e.message))
        .unwrap();

    // Web frontend resolves the admin account from the session token.
    let web_account_id = ws.auth_resolve_session(&session_token).await.unwrap();

    // TUI / in-process MCP resolve the same admin account from the users table.
    let local_account_id = resolve_self_hosted_account_id(&config).await.unwrap();

    assert_eq!(
        web_account_id, local_account_id,
        "Web and in-process frontends must share one admin account_id"
    );

    // Backfill is a no-op when there are no legacy placeholder rows yet.
    let migrated = memayu_identity::backfill_placeholder_memories(&config, &web_account_id)
        .await
        .unwrap();
    assert_eq!(migrated, 0);
}
