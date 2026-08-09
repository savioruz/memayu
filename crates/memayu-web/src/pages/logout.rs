use axum::extract::State;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::response::{IntoResponse, Redirect, Response};
use memayu_api::WebServices;

pub async fn get_logout(
    State(services): State<WebServices>,
    headers: axum::http::HeaderMap,
) -> Result<Response, (axum::http::StatusCode, String)> {
    let token = headers
        .get(COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .find_map(|p| p.trim().strip_prefix("memayu_session="))
        });
    let _ = services.auth_logout(token).await;
    let mut resp = Redirect::to("/login").into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        "memayu_session=; Path=/; Max-Age=0".parse().unwrap(),
    );
    Ok(resp)
}

pub async fn post_logout(
    State(services): State<WebServices>,
    headers: axum::http::HeaderMap,
) -> Result<Response, (axum::http::StatusCode, String)> {
    let token = headers
        .get(COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .find_map(|p| p.trim().strip_prefix("memayu_session="))
        });
    let _ = services.auth_logout(token).await;
    let mut resp = Redirect::to("/login").into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        "memayu_session=; Path=/; Max-Age=0".parse().unwrap(),
    );
    Ok(resp)
}
