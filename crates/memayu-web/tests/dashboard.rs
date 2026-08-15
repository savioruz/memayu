#![cfg(test)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use memayu_api::{open_db, ConfigRegistry, EmbedderConfigProvider, LlmConfigProvider};
use memayu_config::StorageConfig;
use memayu_core::{EmbedError, EmbedderProvider, ExtractionMode, MemoryService};
use memayu_web::build_web_router;
use tower::util::ServiceExt;

/// Deterministic fake embedder used to seed memories without network calls.
/// Each content string yields a distinct, content-hashed 3-dim unit vector, so
/// raw-mode near-duplicate detection never collapses distinct memories.
struct StubEmbedder;

#[async_trait::async_trait]
impl EmbedderProvider for StubEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        // FNV-1a hash of the content seeds a tiny xorshift PRNG so each distinct
        // string maps to a distinct (normalized) 3-dim vector.
        let mut h: u64 = 0xcbf29ce484222325;
        for b in text.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        let mut s = h;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s & 0xFFFF) as f32) / 65535.0 - 0.5
        };
        let mut v = vec![next(), next(), next()];
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in v.iter_mut() {
            *x /= norm;
        }
        Ok(v)
    }
}

fn test_storage_config() -> StorageConfig {
    StorageConfig {
        backend: memayu_config::StorageBackend::Libsql,
        libsql_path: ":memory:".to_string(),
        database_url: None,
    }
}

fn test_provider(name: &str) -> memayu_config::ProviderConfig {
    memayu_config::ProviderConfig {
        backend: memayu_config::EmbedderBackend::Http,
        base_url: format!("http://127.0.0.1/{name}"),
        api_key: Some(format!("key-{name}")),
        model: format!("model-{name}"),
    }
}

async fn build_test_app() -> axum::Router {
    let storage = test_storage_config();
    let registry = ConfigRegistry::new(test_provider("llm"), test_provider("embedder"));

    let storage_provider = std::sync::Arc::new(
        memayu_storage_libsql::LibsqlProvider::open(":memory:", 3)
            .await
            .unwrap(),
    );
    let embedder = EmbedderConfigProvider::new(registry.clone());
    let llm = LlmConfigProvider::new(registry.clone());
    let service = std::sync::Arc::new(MemoryService::new(
        storage_provider,
        std::sync::Arc::new(embedder),
        std::sync::Arc::new(llm),
    ));

    let db = open_db(&storage).await.unwrap();
    build_web_router(db, service, registry)
}

/// Like [`build_test_app`] but uses the fake embedder in raw mode so memories
/// can be seeded directly through the service without a real embedding call.
async fn build_raw_test_app() -> (
    axum::Router,
    memayu_api::DbClient,
    std::sync::Arc<dyn memayu_core::StorageProvider>,
) {
    let storage = test_storage_config();
    let registry = ConfigRegistry::new(test_provider("llm"), test_provider("embedder"));

    let storage_provider: std::sync::Arc<dyn memayu_core::StorageProvider> = std::sync::Arc::new(
        memayu_storage_libsql::LibsqlProvider::open(":memory:", 3)
            .await
            .unwrap(),
    );
    let service = std::sync::Arc::new(
        MemoryService::new(
            storage_provider.clone(),
            std::sync::Arc::new(StubEmbedder),
            std::sync::Arc::new(LlmConfigProvider::new(registry.clone())),
        )
        .with_extraction_mode(ExtractionMode::Raw),
    );

    let db = open_db(&storage).await.unwrap();
    let app = build_web_router(db.clone(), service, registry);
    (app, db, storage_provider)
}

/// Read the value of an attribute (`name="value"`) from rendered HTML.
fn extract_attr<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = html.find(&key)? + key.len();
    let end = html[start..].find('"')? + start;
    Some(&html[start..end])
}

/// URL-encode a base64 cursor so `+`, `/`, and `=` survive in a query string.
fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'+' => out.push_str("%2B"),
            b'/' => out.push_str("%2F"),
            b'=' => out.push_str("%3D"),
            _ => out.push(b as char),
        }
    }
    out
}

#[tokio::test]
async fn setup_login_home_flow() {
    let app = build_test_app().await;

    // 1. First visit redirects to /setup (no users).
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/home").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(loc, "/setup");

    // 2. GET /setup shows the form.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/setup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. POST /setup creates admin + session cookie.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "email=admin@memayu.test&password=Secret12&confirm=Secret12",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let cookie = resp
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.contains("memayu_session="));
    assert_eq!(
        resp.headers().get("location").unwrap().to_str().unwrap(),
        "/home"
    );

    // 4. Access /home with the session cookie.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/home")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_rejects_bad_password() {
    let app = build_test_app().await;

    // Create the user first.
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "email=admin@memayu.test&password=Secret12&confirm=Secret12",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Bad password returns HTML page (not 401, just re-renders the form).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("email=admin@memayu.test&password=wrong"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_via_htmx_returns_hx_redirect() {
    let app = build_test_app().await;

    // Create the user first.
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "email=admin@memayu.test&password=Secret12&confirm=Secret12",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Successful htmx login should not return a redirect body that htmx swaps into the card.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("hx-request", "true")
                .body(Body::from("email=admin@memayu.test&password=Secret12"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        resp.headers().get("hx-redirect").unwrap().to_str().unwrap(),
        "/home"
    );
    assert!(resp.headers().get("set-cookie").is_some());
}

#[tokio::test]
async fn generate_api_key_shown_once() {
    let app = build_test_app().await;

    // Set up admin + capture session.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "email=admin@memayu.test&password=Secret12&confirm=Secret12",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = resp
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Generate a key (query param, as the modal does).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api-keys/generate?label=test-key")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("mmyu_"), "raw key shown once in the response");
}

#[tokio::test]
async fn requests_page_shows_stats() {
    let app = build_test_app().await;

    // Set up admin + capture session.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "email=admin@memayu.test&password=Secret12&confirm=Secret12",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = resp
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // /requests should render without panic (empty stats handled).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/requests")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Total Requests"));
    assert!(text.contains("0"), "empty->0 total");
}

#[tokio::test]
async fn home_paginates_memories_with_prev_next() {
    let (app, db, storage) = build_raw_test_app().await;

    // Set up admin + capture session cookie.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "email=admin@memayu.test&password=Secret12&confirm=Secret12",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let cookie = resp
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // Resolve the admin's user id from the session so we can seed memories.
    let token = cookie
        .split(';')
        .find_map(|p| p.trim().strip_prefix("memayu_session="))
        .unwrap()
        .to_string();
    let user_id = memayu_api::WebServices::new(db)
        .auth_resolve_session(&token)
        .await
        .unwrap();

    // Seed 12 memories directly through the storage provider with distinct,
    // increasing created_at so the newest-first pagination order is exact.
    use memayu_core::Memory;
    let base = chrono::Utc::now();
    for i in 0..12 {
        let created_at = base - chrono::Duration::seconds(12 - i as i64);
        storage
            .save_memory(&Memory {
                id: format!("mem-{i:02}"),
                user_id: user_id.clone(),
                content: format!("memory {i:02}"),
                vector: vec![1.0, 0.0, 0.0],
                metadata: Default::default(),
                created_at,
                updated_at: created_at,
            })
            .await
            .unwrap();
    }

    // First page: 10 of 12 memories, a next cursor, and a disabled Prev button.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/home")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 128)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();

    assert!(html.contains("12 memories"), "total shown on page 1");
    // Newest-first: the 10 newest (memory 11 down to 02) are on page 1.
    assert!(html.contains("memory 11"));
    assert!(html.contains("memory 02"));
    assert!(!html.contains("memory 01"), "2nd-oldest not on page 1");
    assert!(!html.contains("memory 00"), "oldest not on page 1");
    assert!(html.contains("data-next-cursor=\""));
    assert!(!html.contains("data-next-cursor=\"\""));
    assert!(html.contains("id=\"mem-prev\""), "Prev button present");
    assert!(html.contains("id=\"mem-next\""), "Next button present");
    assert!(html.contains("</script>"), "pager script included");

    let next_cursor = extract_attr(&html, "data-next-cursor").expect("next cursor on page 1");

    // Second page via the HTMX fragment route: the remaining 2, no next cursor.
    let url = format!("/home/list?cursor={}", encode_query(next_cursor));
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(url)
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 128)
        .await
        .unwrap();
    let page2 = String::from_utf8(body.to_vec()).unwrap();
    assert!(page2.contains("memory 01"));
    assert!(page2.contains("memory 00"));
    assert!(
        page2.contains("data-next-cursor=\"\""),
        "no further page beyond page 2"
    );

    // A bogus cursor should be rejected, not silently crash.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/home/list?cursor=not-a-valid-cursor")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
