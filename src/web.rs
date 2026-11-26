//! Interface web embarquée dans le binaire.
//!
//! En développement, `SYNALYSE_WEB_DIR` fait lire les fichiers sur le disque à chaque
//! requête : on modifie l'interface sans recompiler.

use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::ids;

struct Asset {
    path: &'static str,
    mime: &'static str,
    bytes: &'static [u8],
}

const ASSETS: &[Asset] = &[
    Asset { path: "index.html", mime: "text/html; charset=utf-8", bytes: include_bytes!("../web/index.html") },
    Asset { path: "app.css", mime: "text/css; charset=utf-8", bytes: include_bytes!("../web/app.css") },
    Asset { path: "app.js", mime: "text/javascript; charset=utf-8", bytes: include_bytes!("../web/app.js") },
    Asset { path: "cairn.svg", mime: "image/svg+xml", bytes: include_bytes!("../web/cairn.svg") },
    Asset { path: "manrope.woff2", mime: "font/woff2", bytes: include_bytes!("../web/fonts/manrope.woff2") },
];

/// Politique de sécurité de l'interface : aucun contenu tiers, aucun script ni style en ligne.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; \
                   connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

async fn serve(name: &str, request: &HeaderMap) -> Response {
    let Some(asset) = ASSETS.iter().find(|a| a.path == name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // La liste ci-dessus sert de liste blanche : `name` ne peut pas sortir du dossier.
    let disk = match std::env::var("SYNALYSE_WEB_DIR") {
        Ok(dir) => {
            let file = if name == "manrope.woff2" { "fonts/manrope.woff2" } else { name };
            tokio::fs::read(std::path::Path::new(&dir).join(file)).await.ok()
        }
        Err(_) => None,
    };
    let bytes: Vec<u8> = disk.unwrap_or_else(|| asset.bytes.to_vec());
    let etag = format!("\"{}\"", ids::hex(&ids::sha256(&[&bytes])[..8]));
    let fresh = request.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v == etag);

    let mut response = if fresh { StatusCode::NOT_MODIFIED.into_response() } else { bytes.into_response() };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(asset.mime));
    // Toujours revalider : une mise à jour du serveur est visible au rechargement suivant.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::ETAG, HeaderValue::from_str(&etag).expect("empreinte hexadécimale"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if name == "index.html" {
        headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
        headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("same-origin"));
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    response
}

pub async fn index(headers: HeaderMap) -> Response {
    serve("index.html", &headers).await
}

pub async fn asset(Path(name): Path<String>, headers: HeaderMap) -> Response {
    if name == "index.html" {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve(&name, &headers).await
}
