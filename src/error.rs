//! Erreur applicative unique, traduite en réponse JSON `{ "error": { "code", "message" } }`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    /// Requête invalide : le message est destiné à l'utilisateur.
    BadRequest(String),
    Unauthorized,
    Forbidden,
    NotFound,
    TooManyRequests,
    Conflict(String),
    /// Erreur interne : journalisée, jamais renvoyée telle quelle.
    Internal(String),
}

impl AppError {
    pub fn bad(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(m) | Self::Conflict(m) | Self::Internal(m) => write!(f, "{m}"),
            Self::Unauthorized => write!(f, "authentification requise"),
            Self::Forbidden => write!(f, "accès refusé"),
            Self::NotFound => write!(f, "introuvable"),
            Self::TooManyRequests => write!(f, "trop de tentatives"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", "Connectez-vous pour continuer.".into()),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Vous n'avez pas accès à cette ressource.".into()),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Ressource introuvable.".into()),
            Self::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                "Trop de tentatives. Réessayez dans quelques minutes.".into(),
            ),
            Self::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            Self::Internal(m) => {
                tracing::error!(error = %m, "erreur interne");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", "Erreur interne du serveur.".into())
            }
        };
        (status, Json(json!({ "error": { "code": code, "message": message } }))).into_response()
    }
}

impl From<tokio_postgres::Error> for AppError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Internal(format!("postgres : {e}"))
    }
}

impl From<deadpool_postgres::PoolError> for AppError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self::Internal(format!("pool postgres : {e}"))
    }
}
