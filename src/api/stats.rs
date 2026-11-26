//! Statistiques de base d'un projet. La mesure d'audience complète arrive au lot 1 ;
//! ce résumé sert déjà à vérifier l'ingestion de bout en bout.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{NaiveDate, NaiveTime};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::{self, Need, Principal};
use crate::error::AppError;

#[derive(Deserialize)]
pub struct Range {
    from: NaiveDate,
    to: NaiveDate,
}

const MAX_RANGE_DAYS: i64 = 800;

pub async fn summary(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Query(range): Query<Range>,
) -> Result<Json<Value>, AppError> {
    if range.to < range.from || (range.to - range.from).num_days() > MAX_RANGE_DAYS {
        return Err(AppError::bad("Période invalide."));
    }
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;

    // `day` est le jour dans le fuseau du projet. Les bornes sur started_at, plus larges d'un
    // jour de chaque côté, ne servent qu'à écarter les partitions hors période.
    let lower = (range.from - chrono::Duration::days(1)).and_time(NaiveTime::MIN).and_utc();
    let upper = (range.to + chrono::Duration::days(2)).and_time(NaiveTime::MIN).and_utc();
    let rows = client
        .query(
            "SELECT day, COUNT(*), COUNT(DISTINCT visitor_id), COALESCE(SUM(pageviews), 0), COALESCE(SUM(events), 0),
                    COUNT(*) FILTER (WHERE pageviews <= 1),
                    COALESCE(SUM(EXTRACT(EPOCH FROM (last_at - started_at))), 0)::float8
               FROM sessions
              WHERE project_id = $1 AND day BETWEEN $2 AND $3 AND started_at >= $4 AND started_at < $5
              GROUP BY day ORDER BY day",
            &[&project_id, &range.from, &range.to, &lower, &upper],
        )
        .await?;
    let visitors: i64 = client
        .query_one(
            "SELECT COUNT(DISTINCT visitor_id) FROM sessions
              WHERE project_id = $1 AND day BETWEEN $2 AND $3 AND started_at >= $4 AND started_at < $5",
            &[&project_id, &range.from, &range.to, &lower, &upper],
        )
        .await?
        .get(0);

    let (mut sessions, mut pageviews, mut events, mut bounces, mut seconds) = (0i64, 0i64, 0i64, 0i64, 0f64);
    let mut by_day = Vec::with_capacity(rows.len());
    for row in &rows {
        let (s, v, p, e, b, d): (i64, i64, i64, i64, i64, f64) =
            (row.get(1), row.get(2), row.get(3), row.get(4), row.get(5), row.get(6));
        sessions += s;
        pageviews += p;
        events += e;
        bounces += b;
        seconds += d;
        by_day.push(
            json!({ "day": row.get::<_, NaiveDate>(0), "sessions": s, "visitors": v, "pageviews": p, "events": e }),
        );
    }
    Ok(Json(json!({
        "from": range.from,
        "to": range.to,
        "visitors": visitors,
        "sessions": sessions,
        "pageviews": pageviews,
        "events": events,
        "bounce_rate": if sessions > 0 { bounces as f64 / sessions as f64 } else { 0.0 },
        "avg_session_seconds": if sessions > 0 { seconds / sessions as f64 } else { 0.0 },
        "by_day": by_day,
    })))
}
