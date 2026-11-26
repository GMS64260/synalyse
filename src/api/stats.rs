//! Mesure d'audience : résumé par période, répartitions et temps réel.
//!
//! Toutes les statistiques partent des sessions de visite. Un filtre (canal, appareil,
//! page vue…) restreint les sessions prises en compte, et donc tous les chiffres.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::{Value, json};
use tokio_postgres::types::ToSql;

use crate::AppState;
use crate::auth::{self, Need, Principal};
use crate::error::AppError;

const MAX_RANGE_DAYS: i64 = 800;

/// Requête SQL en construction, avec ses paramètres numérotés.
pub(super) struct Sql {
    pub text: String,
    pub params: Vec<Box<dyn ToSql + Sync + Send>>,
}

impl Sql {
    pub fn new() -> Self {
        Self { text: String::new(), params: Vec::new() }
    }

    /// Enregistre une valeur et renvoie son repère (`$1`, `$2`…). Aucune valeur
    /// venue de la requête HTTP n'est jamais écrite dans le texte SQL.
    pub fn bind<T: ToSql + Sync + Send + 'static>(&mut self, value: T) -> String {
        self.params.push(Box::new(value));
        format!("${}", self.params.len())
    }

    pub fn refs(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.params.iter().map(|p| p.as_ref() as &(dyn ToSql + Sync)).collect()
    }
}

/// Période demandée et bornes UTC élargies, qui ne servent qu'à écarter les partitions hors période.
pub(super) struct Period {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub lower: DateTime<Utc>,
    pub upper: DateTime<Utc>,
}

pub(super) fn parse_period(query: &HashMap<String, String>) -> Result<Period, AppError> {
    let day = |key: &str| -> Result<NaiveDate, AppError> {
        query
            .get(key)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| AppError::bad(format!("Paramètre « {key} » attendu au format AAAA-MM-JJ.")))
    };
    let (from, to) = (day("from")?, day("to")?);
    if to < from || (to - from).num_days() > MAX_RANGE_DAYS {
        return Err(AppError::bad("Période invalide."));
    }
    Ok(Period {
        from,
        to,
        lower: (from - chrono::Duration::days(1)).and_time(NaiveTime::MIN).and_utc(),
        upper: (to + chrono::Duration::days(2)).and_time(NaiveTime::MIN).and_utc(),
    })
}

/// Filtres portés par une colonne texte de `sessions` : nom du paramètre, colonne.
const TEXT_FILTERS: &[(&str, &str)] = &[
    ("referrer", "referrer_host"),
    ("utm_source", "utm_source"),
    ("utm_medium", "utm_medium"),
    ("utm_campaign", "utm_campaign"),
    ("browser", "browser"),
    ("os", "os"),
    ("lang", "lang"),
    ("entry", "entry_path"),
    ("exit", "exit_path"),
];
/// Filtres portés par une colonne entière de `sessions`.
const INT_FILTERS: &[(&str, &str)] = &[("channel", "channel"), ("device", "device")];

/// Écrit la clause WHERE sur `sessions s` : projet, période et filtres.
pub(super) fn session_where(
    sql: &mut Sql,
    project_id: i64,
    period: &Period,
    query: &HashMap<String, String>,
) -> Result<String, AppError> {
    let mut clauses = vec![
        format!("s.project_id = {}", sql.bind(project_id)),
        format!("s.day BETWEEN {} AND {}", sql.bind(period.from), sql.bind(period.to)),
        format!("s.started_at >= {} AND s.started_at < {}", sql.bind(period.lower), sql.bind(period.upper)),
    ];
    for (param, column) in TEXT_FILTERS {
        if let Some(value) = query.get(*param) {
            if value.is_empty() {
                clauses.push(format!("(s.{column} IS NULL OR s.{column} = '')"));
            } else {
                clauses.push(format!("s.{column} = {}", sql.bind(value.clone())));
            }
        }
    }
    for (param, column) in INT_FILTERS {
        if let Some(value) = query.get(*param) {
            let value: i16 = value.parse().map_err(|_| AppError::bad(format!("Filtre « {param} » invalide.")))?;
            clauses.push(format!("s.{column} = {}", sql.bind(value)));
        }
    }
    if let Some(path) = query.get("page") {
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM events f WHERE f.project_id = s.project_id AND f.session_id = s.id
                      AND f.ts >= {} AND f.ts < {} AND f.kind = 0 AND f.path = {})",
            sql.bind(period.lower),
            sql.bind(period.upper + chrono::Duration::days(1)),
            sql.bind(path.clone()),
        ));
    }
    Ok(clauses.join(" AND "))
}

async fn project_timezone(client: &tokio_postgres::Client, project_id: i64) -> Result<String, AppError> {
    Ok(client.query_one("SELECT timezone FROM projects WHERE id = $1", &[&project_id]).await?.get(0))
}

/// `GET stats/summary` : totaux de la période et série par jour (ou par heure avec `interval=hour`
/// sur une seule journée).
pub async fn summary(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let period = parse_period(&query)?;
    let hourly = query.get("interval").is_some_and(|i| i == "hour");
    if hourly && period.from != period.to {
        return Err(AppError::bad("Le détail par heure porte sur une seule journée."));
    }
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;

    const MEASURES: &str = "COUNT(*), COUNT(DISTINCT s.visitor_id), COALESCE(SUM(s.pageviews), 0), COALESCE(SUM(s.events), 0),
        COUNT(*) FILTER (WHERE s.pageviews <= 1), COALESCE(SUM(EXTRACT(EPOCH FROM (s.last_at - s.started_at))), 0)::float8";

    let mut totals = Sql::new();
    let filter = session_where(&mut totals, project_id, &period, &query)?;
    totals.text = format!("SELECT {MEASURES} FROM sessions s WHERE {filter}");
    let row = client.query_one(&totals.text, &totals.refs()).await?;
    let (sessions, visitors, pageviews, events, bounces, seconds): (i64, i64, i64, i64, i64, f64) =
        (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4), row.get(5));

    let mut series = Sql::new();
    let filter = session_where(&mut series, project_id, &period, &query)?;
    let points: Vec<Value> = if hourly {
        let timezone = project_timezone(&client, project_id).await?;
        let tz = series.bind(timezone);
        series.text = format!(
            "SELECT EXTRACT(HOUR FROM s.started_at AT TIME ZONE {tz})::int AS bucket, {MEASURES}
               FROM sessions s WHERE {filter} GROUP BY bucket ORDER BY bucket"
        );
        client
            .query(&series.text, &series.refs())
            .await?
            .iter()
            .map(|r| json!({ "hour": r.get::<_, i32>(0), "sessions": r.get::<_, i64>(1), "visitors": r.get::<_, i64>(2), "pageviews": r.get::<_, i64>(3), "events": r.get::<_, i64>(4) }))
            .collect()
    } else {
        series.text = format!("SELECT s.day, {MEASURES} FROM sessions s WHERE {filter} GROUP BY s.day ORDER BY s.day");
        client
            .query(&series.text, &series.refs())
            .await?
            .iter()
            .map(|r| json!({ "day": r.get::<_, NaiveDate>(0), "sessions": r.get::<_, i64>(1), "visitors": r.get::<_, i64>(2), "pageviews": r.get::<_, i64>(3), "events": r.get::<_, i64>(4) }))
            .collect()
    };

    let mut body = json!({
        "from": period.from,
        "to": period.to,
        "visitors": visitors,
        "sessions": sessions,
        "pageviews": pageviews,
        "events": events,
        "bounce_rate": if sessions > 0 { bounces as f64 / sessions as f64 } else { 0.0 },
        "avg_session_seconds": if sessions > 0 { seconds / sessions as f64 } else { 0.0 },
    });
    body[if hourly { "by_hour" } else { "by_day" }] = Value::Array(points);
    Ok(Json(body))
}

/// Dimensions lues sur `sessions` : nom public, expression SQL.
const SESSION_DIMS: &[(&str, &str)] = &[
    ("channel", "s.channel::text"),
    ("referrer", "COALESCE(s.referrer_host, '')"),
    ("utm_source", "COALESCE(s.utm_source, '')"),
    ("utm_medium", "COALESCE(s.utm_medium, '')"),
    ("utm_campaign", "COALESCE(s.utm_campaign, '')"),
    ("device", "s.device::text"),
    ("browser", "s.browser"),
    ("os", "s.os"),
    ("lang", "COALESCE(s.lang, '')"),
    ("entry", "s.entry_path"),
    ("exit", "s.exit_path"),
];

/// `GET stats/breakdown?dim=…` : répartition des visites selon une dimension.
/// `page` et `event` comptent des événements ; les autres dimensions comptent des sessions.
pub async fn breakdown(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let period = parse_period(&query)?;
    let dim = query.get("dim").map(String::as_str).unwrap_or("");
    let limit: i64 = query.get("limit").and_then(|l| l.parse().ok()).unwrap_or(10).clamp(1, 500);
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;

    let mut sql = Sql::new();
    let filter = session_where(&mut sql, project_id, &period, &query)?;
    let limit = sql.bind(limit);

    if let Some((_, expr)) = SESSION_DIMS.iter().find(|(name, _)| *name == dim) {
        sql.text = format!(
            "SELECT {expr} AS label, COUNT(DISTINCT s.visitor_id) AS visitors, COUNT(*) AS sessions,
                    COALESCE(SUM(s.pageviews), 0) AS pageviews, COUNT(*) FILTER (WHERE s.pageviews <= 1) AS bounces
               FROM sessions s WHERE {filter}
              GROUP BY label ORDER BY visitors DESC, sessions DESC, label LIMIT {limit}"
        );
        let rows = client.query(&sql.text, &sql.refs()).await?;
        let rows: Vec<Value> = rows
            .iter()
            .map(|r| {
                let sessions: i64 = r.get(2);
                json!({
                    "label": r.get::<_, String>(0), "visitors": r.get::<_, i64>(1), "sessions": sessions,
                    "pageviews": r.get::<_, i64>(3),
                    "bounce_rate": if sessions > 0 { r.get::<_, i64>(4) as f64 / sessions as f64 } else { 0.0 },
                })
            })
            .collect();
        return Ok(Json(json!({ "dim": dim, "rows": rows })));
    }

    let (label, kind) = match dim {
        "page" => ("e.path", 0i16),
        "event" => ("COALESCE(e.name, '')", 1i16),
        _ => return Err(AppError::bad("Dimension inconnue.")),
    };
    let kind = sql.bind(kind);
    let lower = sql.bind(period.lower);
    let upper = sql.bind(period.upper + chrono::Duration::days(1));
    sql.text = format!(
        "SELECT {label} AS label, COUNT(DISTINCT e.visitor_id) AS visitors, COUNT(DISTINCT e.session_id) AS sessions, COUNT(*) AS hits
           FROM events e
           JOIN sessions s ON s.project_id = e.project_id AND s.id = e.session_id
          WHERE {filter} AND e.ts >= {lower} AND e.ts < {upper} AND e.kind = {kind}
          GROUP BY label ORDER BY visitors DESC, hits DESC, label LIMIT {limit}"
    );
    let rows = client.query(&sql.text, &sql.refs()).await?;
    let count_key = if dim == "page" { "pageviews" } else { "count" };
    let rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut row = json!({ "label": r.get::<_, String>(0), "visitors": r.get::<_, i64>(1), "sessions": r.get::<_, i64>(2) });
            row[count_key] = json!(r.get::<_, i64>(3));
            row
        })
        .collect();
    Ok(Json(json!({ "dim": dim, "rows": rows })))
}

/// `GET stats/realtime` : visiteurs actifs sur les cinq et trente dernières minutes.
pub async fn realtime(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let row = client
        .query_one(
            "SELECT COUNT(DISTINCT visitor_id) FILTER (WHERE last_at >= now() - interval '5 minutes'),
                    COUNT(DISTINCT visitor_id)
               FROM sessions
              WHERE project_id = $1 AND started_at >= now() - interval '25 hours' AND last_at >= now() - interval '30 minutes'",
            &[&project_id],
        )
        .await?;
    Ok(Json(json!({ "last_5_minutes": row.get::<_, i64>(0), "last_30_minutes": row.get::<_, i64>(1) })))
}
