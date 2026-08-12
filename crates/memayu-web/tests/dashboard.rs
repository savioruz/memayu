#![cfg(test)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use memayu_api::{open_db, ConfigRegistry, EmbedderConfigProvider, LlmConfigProvider};
use memayu_config::StorageConfig;
use memayu_core::MemoryService;
use memayu_web::build_web_router;
use tower::util::ServiceExt;

fn test_storage_config() -> StorageConfig {
    StorageConfig {
        backend: memayu_config::StorageBackend::Libsql,
        libsql_path: ":memory:".to_string(),
        database_url: None,
    }
}

fn test_provider(name: &str) -> memayu_config::ProviderConfig {
    memayu_config::ProviderConfig {
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
