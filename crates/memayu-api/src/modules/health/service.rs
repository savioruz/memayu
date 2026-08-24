use crate::infrastructure::db::DbClient;
use crate::modules::health::dto::{HealthResponse, HealthStatus};

/// Compute the health status of the server.
///
/// The endpoint is unauthenticated and must work even on a completely fresh,
/// unconfigured instance, so it derives state from the database tables the
/// boot paths already initialize (`users`, `provider_config`).
///
/// - `setup_required`: no admin account yet, or no provider config yet.
/// - `ready`: both the admin account and provider config are present.
///
/// DB errors resolve conservatively to `setup_required`: the server is not
/// provably ready if its backing store cannot be queried.
pub async fn status(db: &DbClient) -> HealthResponse {
    let users_ready = db.users_empty().await.map(|empty| !empty).unwrap_or(false);
    let providers_ready = db
        .provider_configs()
        .await
        .map(|rows| !rows.is_empty())
        .unwrap_or(false);

    let status = if users_ready && providers_ready {
        HealthStatus::Ready
    } else {
        HealthStatus::SetupRequired
    };
    HealthResponse { status }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::DbClient;
    use memayu_config::{StorageBackend, StorageConfig};

    async fn mem_db() -> DbClient {
        let storage = StorageConfig {
            backend: StorageBackend::Libsql,
            libsql_path: ":memory:".to_string(),
            database_url: None,
        };
        let db = DbClient::open(&storage).await.unwrap();
        db.init().await.unwrap();
        db
    }

    /// A provider row fixture: `(provider, (backend, base_url, api_key, model))`.
    type ProviderRow = (
        &'static str,
        (&'static str, &'static str, &'static str, &'static str),
    );

    fn provider_rows() -> Vec<ProviderRow> {
        vec![(
            "embedder",
            ("local", "", "", "sentence-transformers/all-MiniLM-L6-v2"),
        )]
    }

    #[tokio::test]
    async fn fresh_instance_is_setup_required() {
        let db = mem_db().await;
        assert_eq!(status(&db).await.status, HealthStatus::SetupRequired);
    }

    #[tokio::test]
    async fn admin_without_providers_is_setup_required() {
        let db = mem_db().await;
        db.create_user("admin@example.com", "hash", "salt")
            .await
            .unwrap();
        assert_eq!(status(&db).await.status, HealthStatus::SetupRequired);
    }

    #[tokio::test]
    async fn providers_without_admin_is_setup_required() {
        let db = mem_db().await;
        for (provider, (backend, base_url, api_key, model)) in provider_rows() {
            db.upsert_provider_config(provider, backend, base_url, api_key, model)
                .await
                .unwrap();
        }
        assert_eq!(status(&db).await.status, HealthStatus::SetupRequired);
    }

    #[tokio::test]
    async fn admin_and_providers_is_ready() {
        let db = mem_db().await;
        db.create_user("admin@example.com", "hash", "salt")
            .await
            .unwrap();
        for (provider, (backend, base_url, api_key, model)) in provider_rows() {
            db.upsert_provider_config(provider, backend, base_url, api_key, model)
                .await
                .unwrap();
        }
        assert_eq!(status(&db).await.status, HealthStatus::Ready);
    }
}
