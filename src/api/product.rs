//! Analyse produit : entonnoirs enregistrés ou essayés à la volée, et parcours des visiteurs.
//!
//! Les événements des sessions de la période sont lus en flux, session par session,
//! et passés aux moteurs de `crate::analysis` : la mémoire ne dépend pas du volume.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use futures_util::{TryStreamExt, pin_mut};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_postgres::Row;

use super::stats::{Period, Sql, parse_period, session_where};
use crate::AppState;
use crate::analysis::funnel::{self, FunnelCalculator, Hit, StepDef};
use crate::analysis::paths::PathCalculator;
use crate::auth::{self, Need, Principal};
use crate::error::AppError;

fn funnel_json(row: &Row) -> Value {
    json!({
        "id": row.get::<_, i64>(0),
        "name": row.get::<_, String>(1),
        "steps": row.get::<_, Value>(2),
        "created_at": row.get::<_, DateTime<Utc>>(3),
        "updated_at": row.get::<_, DateTime<Utc>>(4),
    })
}

const FUNNEL_COLUMNS: &str = "id, name, steps, created_at, updated_at";

#[derive(Deserialize)]
pub struct FunnelBody {
    name: String,
    steps: Vec<StepDef>,
}

fn clean(body: FunnelBody) -> Result<(String, Vec<StepDef>), AppError> {
    let name: String = body.name.trim().chars().filter(|c| !c.is_control()).take(100).collect();
    if name.is_empty() {
        return Err(AppError::bad("Donnez un nom à l'entonnoir."));
    }
    Ok((name, funnel::sanitize(body.steps).map_err(AppError::BadRequest)?))
}

fn steps_value(steps: &[StepDef]) -> Value {
    serde_json::to_value(steps).expect("étapes sérialisables")
}

pub async fn list_funnels(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let rows = client
        .query(
            &format!(
                "SELECT {FUNNEL_COLUMNS} FROM funnels WHERE project_id = $1 AND deleted_at IS NULL ORDER BY name, id"
            ),
            &[&project_id],
        )
        .await?;
    Ok(Json(json!({ "funnels": rows.iter().map(funnel_json).collect::<Vec<_>>() })))
}

pub async fn create_funnel(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Json(body): Json<FunnelBody>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Write).await?;
    let (name, steps) = clean(body)?;
    let row = client
        .query_one(
            &format!("INSERT INTO funnels (project_id, name, steps, created_by) VALUES ($1, $2, $3, $4) RETURNING {FUNNEL_COLUMNS}"),
            &[&project_id, &name, &steps_value(&steps), &principal.user_id().ok()],
        )
        .await?;
    Ok(Json(funnel_json(&row)))
}

pub async fn update_funnel(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, funnel_id)): Path<(i64, i64)>,
    Json(body): Json<FunnelBody>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Write).await?;
    let (name, steps) = clean(body)?;
    let row = client
        .query_opt(
            &format!(
                "UPDATE funnels SET name = $3, steps = $4, updated_at = now()
                  WHERE id = $1 AND project_id = $2 AND deleted_at IS NULL RETURNING {FUNNEL_COLUMNS}"
            ),
            &[&funnel_id, &project_id, &name, &steps_value(&steps)],
        )
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(funnel_json(&row)))
}

pub async fn delete_funnel(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, funnel_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Write).await?;
    let n = client
        .execute(
            "UPDATE funnels SET deleted_at = now() WHERE id = $1 AND project_id = $2 AND deleted_at IS NULL",
            &[&funnel_id, &project_id],
        )
        .await?;
    if n == 0 { Err(AppError::NotFound) } else { Ok(Json(json!({ "deleted": funnel_id }))) }
}

/// Lit les événements des sessions de la période (filtres compris), groupés par session
/// et triés par date, et les passe un à un à `each`.
async fn each_hit(
    client: &tokio_postgres::Client,
    project_id: i64,
    period: &Period,
    query: &HashMap<String, String>,
    pageviews_only: bool,
    mut each: impl FnMut(&Hit),
) -> Result<(), AppError> {
    let mut sql = Sql::new();
    let filter = session_where(&mut sql, project_id, period, query)?;
    let lower = sql.bind(period.lower);
    let upper = sql.bind(period.upper + chrono::Duration::days(1));
    let kind = if pageviews_only { "AND e.kind = 0" } else { "" };
    sql.text = format!(
        "SELECT e.session_id, e.kind, e.path, e.name, (EXTRACT(EPOCH FROM e.ts) * 1000)::bigint
           FROM events e
           JOIN sessions s ON s.project_id = e.project_id AND s.id = e.session_id
          WHERE {filter} AND e.ts >= {lower} AND e.ts < {upper} {kind}
          ORDER BY e.session_id, e.ts, e.id"
    );
    let rows = client.query_raw(&sql.text, sql.refs()).await?;
    pin_mut!(rows);
    while let Some(row) = rows.try_next().await? {
        let path: &str = row.get(2);
        let name: Option<&str> = row.get(3);
        each(&Hit { session: row.get(0), ts_ms: row.get(4), pageview: row.get::<_, i16>(1) == 0, path, name });
    }
    Ok(())
}

async fn run_funnel(
    client: &tokio_postgres::Client,
    project_id: i64,
    query: &HashMap<String, String>,
    steps: &[StepDef],
) -> Result<Value, AppError> {
    let period = parse_period(query)?;
    let mut calculator = FunnelCalculator::new(steps).map_err(AppError::BadRequest)?;
    each_hit(client, project_id, &period, query, false, |hit| calculator.push(hit)).await?;
    let mut report = serde_json::to_value(calculator.finish()).expect("rapport sérialisable");
    report["from"] = json!(period.from);
    report["to"] = json!(period.to);
    report["definition"] = steps_value(steps);
    Ok(report)
}

/// `GET funnels/{id}/report` : calcule un entonnoir enregistré sur une période.
pub async fn funnel_report(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, funnel_id)): Path<(i64, i64)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let row = client
        .query_opt(
            "SELECT name, steps FROM funnels WHERE id = $1 AND project_id = $2 AND deleted_at IS NULL",
            &[&funnel_id, &project_id],
        )
        .await?
        .ok_or(AppError::NotFound)?;
    let steps: Vec<StepDef> = serde_json::from_value(row.get(1))
        .map_err(|e| AppError::Internal(format!("étapes illisibles de l'entonnoir {funnel_id} : {e}")))?;
    let mut report = run_funnel(&client, project_id, &query, &steps).await?;
    report["id"] = json!(funnel_id);
    report["name"] = json!(row.get::<_, String>(0));
    Ok(Json(report))
}

#[derive(Deserialize)]
pub struct Preview {
    steps: Vec<StepDef>,
}

/// `POST funnels/preview` : calcule des étapes sans les enregistrer, pour essayer avant de garder.
pub async fn preview_funnel(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Preview>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let steps = funnel::sanitize(body.steps).map_err(AppError::BadRequest)?;
    Ok(Json(run_funnel(&client, project_id, &query, &steps).await?))
}

/// `GET paths` : parcours de pages les plus fréquents. Paramètres : `depth` (2 à 8),
/// `max_nodes` (2 à 15), `start` (chemin de la page de départ).
pub async fn paths(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let period = parse_period(&query)?;
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let number = |key: &str, default: usize| query.get(key).and_then(|v| v.parse().ok()).unwrap_or(default);
    let mut calculator = PathCalculator::new(number("depth", 5), number("max_nodes", 6), query.get("start").cloned());
    each_hit(&client, project_id, &period, &query, true, |hit| calculator.push(hit.session, hit.path)).await?;
    let mut result = serde_json::to_value(calculator.finish()).expect("parcours sérialisables");
    result["from"] = json!(period.from);
    result["to"] = json!(period.to);
    Ok(Json(result))
}
