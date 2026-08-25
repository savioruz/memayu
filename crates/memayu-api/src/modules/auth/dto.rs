use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SetupRequest {
    pub email: String,
    pub password: String,
    pub confirm: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    pub confirm: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangeEmailRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub status: String,
    pub message: String,
}
