//! Enregistrements de session : liste, résumé, lecture des événements, suppression.

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_postgres::Row;

use crate::AppState;
use crate::auth::{self, Need, Principal};
use crate::error::AppError;
use crate::recording;

const COLUMNS: &str = "id, started_at, last_at, active_ms, events, bytes, clicks, pages, entry_host, entry_path, \
                       device, browser, os, view_w, view_h, visitor_id";

fn recording_json(row: &Row) -> Value {
    let started: DateTime<Utc> = row.get(1);
    let last: DateTime<Utc> = row.get(2);
    json!({
        "id": row.get::<_, String>(0),
        "started_at": started,
        "last_at": last,
        "duration_ms": (last - started).num_milliseconds().max(0),
        "active_ms": row.get::<_, i64>(3),
        "events": row.get::<_, i32>(4),
        "bytes": row.get::<_, i64>(5),
        "clicks": row.get::<_, i32>(6),
        "pages": row.get::<_, i32>(7),
        "entry_host": row.get::<_, String>(8),
        "entry_path": row.get::<_, String>(9),
        "device": row.get::<_, i16>(10),
        "browser": row.get::<_, String>(11),
        "os": row.get::<_, String>(12),
        "view_w": row.get::<_, Option<i32>>(13),
        "view_h": row.get::<_, Option<i32>>(14),
        // Identifiant interne du visiteur, en texte : il dépasse la précision des nombres JavaScript.
        "visitor": row.get::<_, i64>(15).to_string(),
    })
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    limit: Option<i64>,
    /// Curseur : ne renvoie que les enregistrements commencés avant cet instant.
    #[serde(default)]
    before: Option<DateTime<Utc>>,
    /// Durée active minimale, en secondes.
    #[serde(default)]
    min_active: Option<i64>,
    #[serde(default)]
    visitor: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let min_active_ms = query.min_active.unwrap_or(0).clamp(0, 86_400) * 1000;
    let visitor = match query.visitor.as_deref() {
        Some(v) => Some(v.parse::<i64>().map_err(|_| AppError::bad("Visiteur invalide."))?),
        None => None,
    };
    let rows = client
        .query(
            &format!(
                "SELECT {COLUMNS} FROM recordings
                  WHERE project_id = $1 AND ($2::timestamptz IS NULL OR started_at < $2) AND active_ms >= $3
                    AND ($5::bigint IS NULL OR visitor_id = $5)
                  ORDER BY started_at DESC, id LIMIT $4"
            ),
            &[&project_id, &query.before, &min_active_ms, &(limit + 1), &visitor],
        )
        .await?;
    let total = client
        .query_one(
            "SELECT count(*)::bigint, COALESCE(SUM(bytes), 0)::bigint FROM recordings WHERE project_id = $1",
            &[&project_id],
        )
        .await?;
    let page: Vec<Value> = rows.iter().take(limit as usize).map(recording_json).collect();
    let next = if rows.len() as i64 > limit { page.last().map(|r| r["started_at"].clone()) } else { None };
    Ok(Json(json!({
        "recordings": page,
        "next_before": next,
        "total": total.get::<_, i64>(0),
        "stored_bytes": total.get::<_, i64>(1),
    })))
}

async fn find(
    state: &AppState,
    principal: &Principal,
    project_id: i64,
    id: &str,
    need: Need,
) -> Result<(deadpool_postgres::Client, Row), AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, principal, project_id, need).await?;
    if !recording::valid_id(id) {
        return Err(AppError::NotFound);
    }
    let row = client
        .query_opt(&format!("SELECT {COLUMNS} FROM recordings WHERE project_id = $1 AND id = $2"), &[&project_id, &id])
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((client, row))
}

pub async fn get(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, id)): Path<(i64, String)>,
) -> Result<Json<Value>, AppError> {
    let (_, row) = find(&state, &principal, project_id, &id, Need::Read).await?;
    Ok(Json(recording_json(&row)))
}

/// Événements de l'enregistrement : une ligne par morceau reçu, chaque ligne est un tableau JSON.
pub async fn events(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, id)): Path<(i64, String)>,
) -> Result<Response, AppError> {
    drop(find(&state, &principal, project_id, &id, Need::Read).await?);
    let path = recording::file_path(&state.config.data_dir, project_id, &id);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }
    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::task::spawn_blocking(move || {
        let result = recording::read_recording(&path, |block| tx.blocking_send(Ok(Bytes::from(block))).is_ok());
        if let Err(e) = result {
            let _ = tx.blocking_send(Err(e));
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });
    let mut response = Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-ndjson; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

pub async fn delete(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, id)): Path<(i64, String)>,
) -> Result<Json<Value>, AppError> {
    let (client, _) = find(&state, &principal, project_id, &id, Need::Write).await?;
    client.execute("DELETE FROM recordings WHERE project_id = $1 AND id = $2", &[&project_id, &id]).await?;
    let dir = state.config.data_dir.clone();
    tokio::task::spawn_blocking(move || recording::delete_file(&dir, project_id, &id)).await.ok();
    Ok(Json(json!({ "deleted": true })))
}
