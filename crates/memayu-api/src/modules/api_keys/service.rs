use crate::error::ApiError;
use crate::infrastructure::db::DbClient;
use crate::modules::api_keys::dto::{
    GenerateKeyRequest, GenerateKeyResponse, ListKeyResponse, ListKeysResponse,
};
use rand::Rng;
use sha2::{Digest, Sha256};

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// Resolve an API key to its owner's user_id. Also updates last_used_at.
pub async fn resolve(api_key: &str, db: &DbClient) -> Result<String, String> {
    let key_hash = sha256_hex(api_key);
    let user_id = db
        .find_api_key_by_hash(&key_hash)
        .await?
        .ok_or_else(|| String::from("invalid api key"))?;
    let _ = db.touch_api_key(&key_hash).await;
    Ok(user_id)
}

/// Generate a new API key for the given user.
pub async fn generate_key(
    db: &DbClient,
    user_id: &str,
    req: &GenerateKeyRequest,
) -> Result<GenerateKeyResponse, ApiError> {
    let label = req.label.trim();
    if label.is_empty() {
        return Err(ApiError::bad_request("label is required"));
    }

    let mut rng = rand::rngs::OsRng;
    let raw: [u8; 24] = rng.gen();
    let raw_key = format!("mmyu_{}", hex::encode(raw));
    let key_prefix = format!("mmyu_{}", hex::encode(&raw[..2]));
    let key_hash = sha256_hex(&raw_key);
    let id = uuid::Uuid::new_v4().to_string();
    let created = chrono::Utc::now().to_rfc3339();

    db.insert_api_key(&id, user_id, label, &key_prefix, &key_hash)
        .await
        .map_err(|e| ApiError {
            status: 500,
            error: "internal_error".into(),
            message: e,
        })?;

    Ok(GenerateKeyResponse {
        key: raw_key,
        id,
        label: label.to_string(),
        key_prefix,
        created_at: created,
    })
}

/// List all API keys.
pub async fn list_keys(db: &DbClient) -> Result<ListKeysResponse, ApiError> {
    let keys = db.list_api_keys().await.map_err(|e| ApiError {
        status: 500,
        error: "internal_error".into(),
        message: e,
    })?;
    Ok(ListKeysResponse {
        keys: keys
            .into_iter()
            .map(|k| ListKeyResponse {
                id: k.id,
                label: k.label,
                key_prefix: k.key_prefix,
                last_used_at: k.last_used_at,
                created_at: k.created_at,
            })
            .collect(),
    })
}

/// Delete an API key by ID.
pub async fn delete_key(db: &DbClient, id: &str) -> Result<(), ApiError> {
    db.delete_api_key(id).await.map_err(|e| ApiError {
        status: 500,
        error: "internal_error".into(),
        message: e,
    })?;
    Ok(())
}
