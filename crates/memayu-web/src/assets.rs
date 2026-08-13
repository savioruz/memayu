use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use rust_embed::Embed;

/// Pinned versions of the vendored static bundles under `static/`.
#[allow(dead_code)]
pub mod vendored {
    /// `htmx.min.js` - the htmx.org distribution bundle.
    pub const HTMX_FILE: &str = "htmx.min.js";
    pub const HTMX_VERSION: &str = "1.9.12";

    /// `alpine.min.js` - the Alpine.js CDN bundle.
    pub const ALPINE_FILE: &str = "alpine.min.js";
    pub const ALPINE_VERSION: &str = "3.15.12";

    /// `scalar.min.js` - the `@scalar/api-reference` standalone bundle,
    /// self-hosted so `/docs` does not depend on a third-party CDN (issue #29).
    pub const SCALAR_FILE: &str = "scalar.min.js";
    pub const SCALAR_VERSION: &str = "1.65.0";
}

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

    #[test]
    fn scalar_embedded() {
        assert!(
            Assets::get("scalar.min.js").is_some(),
            "scalar.min.js must be embedded for the self-hosted /docs page"
        );
    }

    /// Returns whether the embedded asset contains `needle` as a byte slice.
    fn asset_contains(name: &str, needle: &str) -> bool {
        Assets::get(name)
            .map(|file| {
                file.data
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
            })
            .unwrap_or(false)
    }

    #[test]
    fn vendored_versions_pinned() {
        let cases = [
            (vendored::HTMX_FILE, vendored::HTMX_VERSION.to_string()),
            (vendored::ALPINE_FILE, vendored::ALPINE_VERSION.to_string()),
            (
                vendored::SCALAR_FILE,
                format!("@scalar/api-reference@{}", vendored::SCALAR_VERSION),
            ),
        ];
        for (file, marker) in cases {
            assert!(
                asset_contains(file, &marker),
                "{file} must embed version marker {marker:?}; \
                 bump vendored::*_VERSION if upgrading"
            );
        }
    }
}
