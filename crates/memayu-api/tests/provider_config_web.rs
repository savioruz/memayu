// Regression test: the web form upsert path (WebServices::provider_upsert) must
// clear base_url/api_key for a local embedder before persisting, so a stale
// base_url carried over from a prior remote config never lands in the DB row.
use memayu_api::{open_db, DbClient, WebServices};
use memayu_config::{StorageBackend, StorageConfig};

async fn mem_db() -> DbClient {
    let storage = StorageConfig {
        backend: StorageBackend::Libsql,
        libsql_path: ":memory:".to_string(),
        database_url: None,
    };
    let db = open_db(&storage).await.unwrap();
    db
}

#[tokio::test]
async fn web_form_local_embedder_clears_base_url_and_api_key() {
    let db = mem_db().await;
    let svc = WebServices::new(db.clone());
    svc.provider_upsert(
        "embedder",
        "local",
        "https://api.openai.com/v1",
        "sk-stale",
        "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
    )
    .await
    .unwrap();

    let rows = db.provider_configs().await.unwrap();
    let (backend, base_url, api_key, model) = &rows["embedder"];
    assert_eq!(backend, "local");
    assert_eq!(base_url, "", "local embedder must not persist a base_url");
    assert_eq!(api_key, "", "local embedder must not persist an api_key");
    assert_eq!(
        model,
        "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
    );
}

#[tokio::test]
async fn web_form_remote_embedder_keeps_base_url_and_api_key() {
    let db = mem_db().await;
    let svc = WebServices::new(db.clone());
    svc.provider_upsert("embedder", "remote", "http://localhost:11434", "sk", "m")
        .await
        .unwrap();

    let rows = db.provider_configs().await.unwrap();
    let (backend, base_url, api_key, _) = &rows["embedder"];
    assert_eq!(backend, "remote");
    assert_eq!(base_url, "http://localhost:11434");
    assert_eq!(api_key, "sk");
}

#[tokio::test]
async fn web_setup_persist_raw_writes_no_llm_row() {
    let db = mem_db().await;
    let svc = WebServices::new(db.clone());
    // A raw-mode setup must not persist a placeholder LLM row, even though a
    // full LLM config is passed (e.g. a re-config prefilled with old values).
    let llm = memayu_config::ProviderConfig {
        backend: memayu_config::EmbedderBackend::Remote,
        base_url: "https://api.openai.com/v1".into(),
        api_key: Some("sk-stale".into()),
        model: "gpt-4".into(),
    };
    let embedder = memayu_config::ProviderConfig {
        backend: memayu_config::EmbedderBackend::Local,
        base_url: String::new(),
        api_key: None,
        model: "sentence-transformers/all-MiniLM-L6-v2".into(),
    };
    svc.setup_persist(&llm, &embedder, memayu_core::ExtractionMode::Raw)
        .await
        .unwrap();

    let rows = db.provider_configs().await.unwrap();
    assert!(
        !rows.contains_key("llm"),
        "raw web /setup must not write an llm row, got {rows:?}"
    );
    assert!(rows.contains_key("embedder"));
    assert_eq!(
        db.get_extraction_mode().await.unwrap().as_deref(),
        Some("raw")
    );
}
