//! Authentification : session de l'interface (cookie) ou jeton d'API (en-tête Authorization).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;

use crate::AppState;
use crate::error::AppError;
use crate::ids;

pub const SESSION_COOKIE: &str = "synalyse_session";
pub const SESSION_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Ingest,
    Admin,
}

impl Scope {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "read" => Some(Self::Read),
            "ingest" => Some(Self::Ingest),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

/// Qui appelle l'API.
#[derive(Debug, Clone)]
pub enum Principal {
    User { id: i64 },
    Token { project_id: i64, scope: Scope },
}

/// Ce qu'une route demande sur un projet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Read,
    Ingest,
    Admin,
}

impl Principal {
    pub fn user_id(&self) -> Result<i64, AppError> {
        match self {
            Self::User { id } => Ok(*id),
            Self::Token { .. } => Err(AppError::Forbidden),
        }
    }
}

fn role_allows(role: &str, need: Need) -> bool {
    match need {
        Need::Read => matches!(role, "owner" | "admin" | "member" | "viewer"),
        Need::Ingest | Need::Admin => matches!(role, "owner" | "admin"),
    }
}

fn scope_allows(scope: Scope, need: Need) -> bool {
    matches!((scope, need), (Scope::Admin, _) | (Scope::Read, Need::Read) | (Scope::Ingest, Need::Ingest))
}

/// Vérifie le droit sur un projet. Un projet inconnu et un projet d'une autre
/// organisation donnent la même réponse : son existence ne fuit pas.
pub async fn authorize_project(
    client: &tokio_postgres::Client,
    principal: &Principal,
    project_id: i64,
    need: Need,
) -> Result<(), AppError> {
    match principal {
        Principal::Token { project_id: own, scope } => {
            if *own != project_id {
                return Err(AppError::NotFound);
            }
            if scope_allows(*scope, need) { Ok(()) } else { Err(AppError::Forbidden) }
        }
        Principal::User { id } => {
            let row = client
                .query_opt(
                    "SELECT m.role FROM projects p JOIN memberships m ON m.org_id = p.org_id
                      WHERE p.id = $1 AND m.user_id = $2 AND p.archived_at IS NULL",
                    &[&project_id, id],
                )
                .await?;
            let role: String = row.ok_or(AppError::NotFound)?.get(0);
            if role_allows(&role, need) { Ok(()) } else { Err(AppError::Forbidden) }
        }
    }
}

/// Vérifie l'appartenance à une organisation ; `admin` exige le rôle owner ou admin.
pub async fn authorize_org(
    client: &tokio_postgres::Client,
    principal: &Principal,
    org_id: i64,
    admin: bool,
) -> Result<(), AppError> {
    let user_id = principal.user_id()?;
    let row = client
        .query_opt("SELECT role FROM memberships WHERE org_id = $1 AND user_id = $2", &[&org_id, &user_id])
        .await?;
    let role: String = row.ok_or(AppError::NotFound)?.get(0);
    if !admin || matches!(role.as_str(), "owner" | "admin") { Ok(()) } else { Err(AppError::Forbidden) }
}

pub fn cookie_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

pub fn token_hash(token: &str) -> [u8; 32] {
    ids::sha256(&[token.as_bytes()])
}

impl FromRequestParts<AppState> for Principal {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim);
        let client = state.pool.get().await?;

        if let Some(token) = bearer {
            let hash = token_hash(token);
            let row = client
                .query_opt(
                    "UPDATE api_tokens SET last_used_at = now()
                      WHERE token_hash = $1 AND revoked_at IS NULL
                  RETURNING project_id, scope",
                    &[&hash.as_slice()],
                )
                .await?
                .ok_or(AppError::Unauthorized)?;
            let scope: String = row.get(1);
            return Ok(Principal::Token {
                project_id: row.get(0),
                scope: Scope::parse(&scope).ok_or(AppError::Unauthorized)?,
            });
        }

        let token = cookie_value(&parts.headers, SESSION_COOKIE).ok_or(AppError::Unauthorized)?;
        let hash = token_hash(token);
        let row = client
            .query_opt(
                "UPDATE user_sessions s SET last_seen_at = now()
                   FROM users u
                  WHERE s.token_hash = $1 AND s.expires_at > now() AND u.id = s.user_id AND u.disabled_at IS NULL
              RETURNING s.user_id",
                &[&hash.as_slice()],
            )
            .await?
            .ok_or(AppError::Unauthorized)?;
        Ok(Principal::User { id: row.get(0) })
    }
}

/// Limite les essais de connexion par adresse e-mail : huit échecs par quart d'heure.
#[derive(Default)]
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

const LOGIN_WINDOW: Duration = Duration::from_secs(900);
const LOGIN_MAX_FAILURES: u32 = 8;

impl LoginLimiter {
    pub fn blocked(&self, key: &str) -> bool {
        let failures = self.failures.lock().expect("verrou du limiteur");
        failures.get(key).is_some_and(|(count, since)| *count >= LOGIN_MAX_FAILURES && since.elapsed() < LOGIN_WINDOW)
    }

    pub fn record_failure(&self, key: &str) {
        let mut failures = self.failures.lock().expect("verrou du limiteur");
        if failures.len() > 100_000 {
            failures.retain(|_, (_, since)| since.elapsed() < LOGIN_WINDOW);
        }
        let entry = failures.entry(key.to_string()).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= LOGIN_WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }

    pub fn reset(&self, key: &str) {
        self.failures.lock().expect("verrou du limiteur").remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn rights_matrix() {
        assert!(role_allows("viewer", Need::Read));
        assert!(!role_allows("viewer", Need::Admin));
        assert!(!role_allows("member", Need::Ingest));
        assert!(role_allows("admin", Need::Admin));
        assert!(!role_allows("inconnu", Need::Read));
        assert!(scope_allows(Scope::Read, Need::Read));
        assert!(!scope_allows(Scope::Read, Need::Ingest));
        assert!(!scope_allows(Scope::Ingest, Need::Read), "un jeton d'import ne lit pas les statistiques");
        assert!(scope_allows(Scope::Admin, Need::Ingest));
    }

    #[test]
    fn reads_cookies() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; synalyse_session=abc; b=2"));
        assert_eq!(cookie_value(&h, SESSION_COOKIE), Some("abc"));
        assert_eq!(cookie_value(&h, "session"), None, "pas de correspondance sur un suffixe");
    }

    #[test]
    fn limiter_blocks_after_repeated_failures() {
        let limiter = LoginLimiter::default();
        for _ in 0..LOGIN_MAX_FAILURES - 1 {
            limiter.record_failure("a@b.c");
        }
        assert!(!limiter.blocked("a@b.c"));
        limiter.record_failure("a@b.c");
        assert!(limiter.blocked("a@b.c"));
        assert!(!limiter.blocked("autre@b.c"));
        limiter.reset("a@b.c");
        assert!(!limiter.blocked("a@b.c"));
    }
}
