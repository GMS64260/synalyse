//! Connexion, déconnexion et profil de l'utilisateur courant.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::auth::{self, Principal, SESSION_COOKIE, SESSION_DAYS};
use crate::error::AppError;
use crate::ids;

#[derive(Deserialize)]
pub struct Login {
    email: String,
    password: String,
}

fn session_cookie(state: &AppState, value: &str, max_age: i64) -> HeaderValue {
    let secure = if state.config.secure_cookies() { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("cookie ASCII")
}

pub async fn login(State(state): State<AppState>, Json(body): Json<Login>) -> Result<Response, AppError> {
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || body.password.is_empty() {
        return Err(AppError::bad("Adresse e-mail et mot de passe requis."));
    }
    if state.logins.blocked(&email) {
        return Err(AppError::TooManyRequests);
    }
    let client = state.pool.get().await?;
    let row = client
        .query_opt("SELECT id, password_hash FROM users WHERE lower(email) = $1 AND disabled_at IS NULL", &[&email])
        .await?;
    let (user_id, stored): (Option<i64>, String) = match row {
        Some(row) => (Some(row.get(0)), row.get(1)),
        // Compte inconnu : on hache quand même, pour que le temps de réponse ne le révèle pas.
        None => (None, String::new()),
    };
    let password = body.password;
    let valid = tokio::task::spawn_blocking(move || {
        if stored.is_empty() {
            ids::hash_password(&password);
            false
        } else {
            ids::verify_password(&password, &stored)
        }
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    let Some(user_id) = user_id.filter(|_| valid) else {
        state.logins.record_failure(&email);
        return Err(AppError::bad("Adresse e-mail ou mot de passe incorrect."));
    };
    state.logins.reset(&email);

    let token = ids::random_token();
    let hash = auth::token_hash(&token);
    client
        .execute(
            "INSERT INTO user_sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + make_interval(days => $3))",
            &[&hash.as_slice(), &user_id, &(SESSION_DAYS as i32)],
        )
        .await?;
    client.execute("DELETE FROM user_sessions WHERE expires_at < now()", &[]).await?;

    let mut response = Json(json!({ "ok": true })).into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie(&state, &token, SESSION_DAYS * 86400));
    Ok(response)
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    if let Some(token) = auth::cookie_value(&headers, SESSION_COOKIE) {
        let hash = auth::token_hash(token);
        state.pool.get().await?.execute("DELETE FROM user_sessions WHERE token_hash = $1", &[&hash.as_slice()]).await?;
    }
    let mut response = Json(json!({ "ok": true })).into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie(&state, "", 0));
    Ok(response)
}

pub async fn me(State(state): State<AppState>, principal: Principal) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = principal.user_id()?;
    let client = state.pool.get().await?;
    let user = client.query_one("SELECT id, email, name FROM users WHERE id = $1", &[&user_id]).await?;
    let orgs = client
        .query(
            "SELECT o.id, o.name, m.role FROM organizations o JOIN memberships m ON m.org_id = o.id
              WHERE m.user_id = $1 ORDER BY o.name, o.id",
            &[&user_id],
        )
        .await?;
    let projects = client
        .query(
            "SELECT p.id, p.org_id, p.name, p.timezone FROM projects p JOIN memberships m ON m.org_id = p.org_id
              WHERE m.user_id = $1 AND p.archived_at IS NULL ORDER BY p.name, p.id",
            &[&user_id],
        )
        .await?;
    Ok(Json(json!({
        "user": { "id": user.get::<_, i64>(0), "email": user.get::<_, String>(1), "name": user.get::<_, String>(2) },
        "organizations": orgs.iter().map(|r| json!({
            "id": r.get::<_, i64>(0), "name": r.get::<_, String>(1), "role": r.get::<_, String>(2),
        })).collect::<Vec<_>>(),
        "projects": projects.iter().map(|r| json!({
            "id": r.get::<_, i64>(0), "org_id": r.get::<_, i64>(1), "name": r.get::<_, String>(2), "timezone": r.get::<_, String>(3),
        })).collect::<Vec<_>>(),
    })))
}
