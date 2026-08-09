use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "static/"]
#[include = "*.css"]
#[include = "*.js"]
#[exclude = "*.map"]
struct Assets;

/// Serves files embedded from the `static/` directory under `/static/*path`.
///
/// Files are embedded into the binary at compile time via `rust-embed`.
/// MIME types are derived from file extensions via `mime_guess`.
pub async fn serve_static(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let path = path.trim_start_matches('/');
    let file = Assets::get(path).ok_or(StatusCode::NOT_FOUND)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let headers = [(header::CONTENT_TYPE, mime.as_ref())];
    Ok((headers, file.data).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_embedded() {
        assert!(
            Assets::get("mu.slate.css").is_some(),
            "mu.slate.css must be embedded"
        );
        assert!(
            Assets::get("memayu.css").is_some(),
            "memayu.css must be embedded"
        );
    }

    #[test]
    fn htmx_embedded() {
        assert!(
            Assets::get("htmx.min.js").is_some(),
            "htmx.min.js must be embedded"
        );
    }

    #[test]
    fn alpine_embedded() {
        assert!(
            Assets::get("alpine.min.js").is_some(),
            "alpine.min.js must be embedded"
        );
    }
}
