//! Visiteurs reconnus (consentement donné ou utilisateur identifié) : liste, fiche, rétention.
//!
//! L'identifiant interne d'un visiteur est un entier de 63 bits : il voyage en texte, la
//! précision des nombres JavaScript ne suffit pas.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_postgres::Row;

use super::stats::parse_period;
use crate::AppState;
use crate::auth::{self, Need, Principal};
use crate::error::AppError;

const VISITOR_COLUMNS: &str = "id, distinct_id, first_seen, last_seen, sessions, traits";

fn visitor_json(row: &Row) -> Value {
    let distinct: String = row.get(1);
    // « u:42 » : identifiant donné par le site. Sinon, identifiant tiré au hasard après consentement.
    let user = distinct.strip_prefix("u:").map(str::to_string);
    json!({
        "id": row.get::<_, i64>(0).to_string(),
        "user": user,
        "first_seen": row.get::<_, DateTime<Utc>>(2),
        "last_seen": row.get::<_, DateTime<Utc>>(3),
        "sessions": row.get::<_, i32>(4),
        "traits": row.get::<_, Value>(5),
    })
}

fn parse_visitor(text: &str) -> Result<i64, AppError> {
    text.parse::<i64>().ok().filter(|id| *id >= 0).ok_or(AppError::NotFound)
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    limit: Option<i64>,
    /// Curseur : visiteurs vus pour la dernière fois avant cet instant.
    #[serde(default)]
    before: Option<DateTime<Utc>>,
    /// Recherche dans l'identifiant d'utilisateur.
    #[serde(default)]
    q: Option<String>,
    /// « users » : seulement les utilisateurs identifiés par le site.
    #[serde(default)]
    kind: Option<String>,
}

/// Échappe les caractères spéciaux de LIKE : la recherche est littérale.
fn like_pattern(text: &str) -> String {
    let mut out = String::from("%");
    for c in text.chars().take(100) {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
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
    let search = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty()).map(like_pattern);
    let users_only = query.kind.as_deref() == Some("users") || search.is_some();
    let rows = client
        .query(
            &format!(
                "SELECT {VISITOR_COLUMNS} FROM visitors
                  WHERE project_id = $1 AND ($2::timestamptz IS NULL OR last_seen < $2)
                    AND (NOT $3 OR distinct_id LIKE 'u:%')
                    AND ($4::text IS NULL OR substr(distinct_id, 3) ILIKE $4)
                  ORDER BY last_seen DESC, id LIMIT $5"
            ),
            &[&project_id, &query.before, &users_only, &search, &(limit + 1)],
        )
        .await?;
    let totals = client
        .query_one(
            "SELECT count(*)::bigint, count(*) FILTER (WHERE distinct_id LIKE 'u:%')::bigint
               FROM visitors WHERE project_id = $1",
            &[&project_id],
        )
        .await?;
    let page: Vec<Value> = rows.iter().take(limit as usize).map(visitor_json).collect();
    let next = if rows.len() as i64 > limit { page.last().map(|v| v["last_seen"].clone()) } else { None };
    Ok(Json(json!({
        "visitors": page,
        "next_before": next,
        "total": totals.get::<_, i64>(0),
        "users": totals.get::<_, i64>(1),
    })))
}

/// Fiche d'un visiteur : ses dernières visites et ses enregistrements.
pub async fn get(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, visitor)): Path<(i64, String)>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let visitor = parse_visitor(&visitor)?;
    let row = client
        .query_opt(
            &format!("SELECT {VISITOR_COLUMNS} FROM visitors WHERE project_id = $1 AND id = $2"),
            &[&project_id, &visitor],
        )
        .await?
        .ok_or(AppError::NotFound)?;
    let sessions = client
        .query(
            "SELECT id, started_at, last_at, entry_path, exit_path, pageviews, events, channel, referrer_host,
                    device, browser, os
               FROM sessions WHERE project_id = $1 AND visitor_id = $2
              ORDER BY last_at DESC LIMIT 50",
            &[&project_id, &visitor],
        )
        .await?;
    let recordings: i64 = client
        .query_one(
            "SELECT count(*)::bigint FROM recordings WHERE project_id = $1 AND visitor_id = $2",
            &[&project_id, &visitor],
        )
        .await?
        .get(0);
    let mut body = visitor_json(&row);
    body["recordings"] = json!(recordings);
    body["recent_sessions"] = sessions
        .iter()
        .map(|s| {
            let started: DateTime<Utc> = s.get(1);
            let last: DateTime<Utc> = s.get(2);
            json!({
                "id": s.get::<_, i64>(0).to_string(),
                "started_at": started,
                "duration_seconds": (last - started).num_seconds().max(0),
                "entry_path": s.get::<_, String>(3),
                "exit_path": s.get::<_, String>(4),
                "pageviews": s.get::<_, i32>(5),
                "events": s.get::<_, i32>(6),
                "channel": s.get::<_, i16>(7),
                "referrer": s.get::<_, Option<String>>(8),
                "device": s.get::<_, i16>(9),
                "browser": s.get::<_, String>(10),
                "os": s.get::<_, String>(11),
            })
        })
        .collect();
    Ok(Json(body))
}

#[derive(Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    before: Option<DateTime<Utc>>,
}

/// Fil des pages vues et des événements d'un visiteur, du plus récent au plus ancien.
pub async fn events(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, visitor)): Path<(i64, String)>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let visitor = parse_visitor(&visitor)?;
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let rows = client
        .query(
            "SELECT e.ts, e.kind, e.name, e.path, e.title, e.props, e.session_id
               FROM events e
              WHERE e.project_id = $1 AND e.visitor_id = $2 AND ($3::timestamptz IS NULL OR e.ts < $3)
                AND EXISTS (SELECT 1 FROM visitors v WHERE v.project_id = $1 AND v.id = $2)
              ORDER BY e.ts DESC, e.id DESC LIMIT $4",
            &[&project_id, &visitor, &query.before, &(limit + 1)],
        )
        .await?;
    let page: Vec<Value> = rows
        .iter()
        .take(limit as usize)
        .map(|r| {
            json!({
                "ts": r.get::<_, DateTime<Utc>>(0),
                "kind": if r.get::<_, i16>(1) == 0 { "page" } else { "event" },
                "name": r.get::<_, Option<String>>(2),
                "path": r.get::<_, String>(3),
                "title": r.get::<_, Option<String>>(4),
                "props": r.get::<_, Option<Value>>(5),
                "session": r.get::<_, i64>(6).to_string(),
            })
        })
        .collect();
    let next = if rows.len() as i64 > limit { page.last().map(|e| e["ts"].clone()) } else { None };
    Ok(Json(json!({ "events": page, "next_before": next })))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Day,
    Week,
    Month,
}

impl Unit {
    fn parse(text: Option<&str>) -> Result<Self, AppError> {
        match text.unwrap_or("week") {
            "day" => Ok(Self::Day),
            "week" => Ok(Self::Week),
            "month" => Ok(Self::Month),
            _ => Err(AppError::bad("Unité de rétention inconnue : day, week ou month.")),
        }
    }

    /// Début de la période qui contient ce jour (les semaines commencent le lundi).
    pub fn start(self, day: NaiveDate) -> NaiveDate {
        match self {
            Self::Day => day,
            Self::Week => day - chrono::Duration::days(i64::from(day.weekday().num_days_from_monday())),
            Self::Month => day.with_day(1).expect("premier du mois"),
        }
    }

    /// Nombre de périodes entières entre deux débuts de période.
    pub fn index(self, cohort: NaiveDate, period: NaiveDate) -> i64 {
        match self {
            Self::Day => (period - cohort).num_days(),
            Self::Week => (period - cohort).num_days() / 7,
            Self::Month => {
                i64::from(period.year() - cohort.year()) * 12 + i64::from(period.month()) - i64::from(cohort.month())
            }
        }
    }
}

const MAX_RETENTION_PERIODS: i64 = 26;

/// Tableau de rétention : une ligne par cohorte (les visiteurs vus pour la première fois
/// pendant une période), une colonne par période écoulée depuis.
pub fn retention_table(
    unit: Unit,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
    first_days: &[(i64, NaiveDate)],
    activity: &[(i64, NaiveDate)],
) -> Value {
    let mut cohort_of: HashMap<i64, NaiveDate> = HashMap::new();
    let mut sizes: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let mut cursor = unit.start(from);
    while cursor <= to {
        sizes.insert(cursor, 0);
        // Le début de la période suivante : un saut plus long que la période, ramené à son début.
        let step = match unit {
            Unit::Day => 1,
            Unit::Week => 7,
            Unit::Month => 32,
        };
        cursor = unit.start(cursor + chrono::Duration::days(step));
    }
    for (visitor, first_day) in first_days {
        if *first_day < from || *first_day > to {
            continue;
        }
        let cohort = unit.start(*first_day);
        cohort_of.insert(*visitor, cohort);
        *sizes.entry(cohort).or_default() += 1;
    }
    // (cohorte, rang de la période) → visiteurs distincts revenus.
    let mut returned: HashMap<(NaiveDate, i64), std::collections::HashSet<i64>> = HashMap::new();
    for (visitor, day) in activity {
        let Some(cohort) = cohort_of.get(visitor) else { continue };
        let index = unit.index(*cohort, unit.start(*day));
        if (0..=MAX_RETENTION_PERIODS).contains(&index) {
            returned.entry((*cohort, index)).or_default().insert(*visitor);
        }
    }
    let current = unit.start(today);
    let cohorts: Vec<Value> = sizes
        .iter()
        .map(|(cohort, size)| {
            // Une période qui n'est pas encore commencée n'a pas de valeur : elle n'est pas « zéro ».
            let elapsed = unit.index(*cohort, current).clamp(0, MAX_RETENTION_PERIODS);
            let values: Vec<i64> = (0..=elapsed)
                .map(|i| if i == 0 { *size } else { returned.get(&(*cohort, i)).map_or(0, |set| set.len() as i64) })
                .collect();
            json!({ "start": cohort, "size": size, "values": values })
        })
        .collect();
    let total: i64 = sizes.values().sum();
    json!({ "unit": match unit { Unit::Day => "day", Unit::Week => "week", Unit::Month => "month" }, "visitors": total, "cohorts": cohorts })
}

/// `GET retention?from&to&unit` : part des visiteurs reconnus qui reviennent, période après période.
pub async fn retention(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let period = parse_period(&query)?;
    let unit = Unit::parse(query.get("unit").map(String::as_str))?;
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let timezone: String =
        client.query_one("SELECT timezone FROM projects WHERE id = $1", &[&project_id]).await?.get(0);
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let today = Utc::now().with_timezone(&tz).date_naive();

    let first_days: Vec<(i64, NaiveDate)> = client
        .query(
            "SELECT id, first_day FROM visitors WHERE project_id = $1 AND first_day BETWEEN $2 AND $3",
            &[&project_id, &period.from, &period.to],
        )
        .await?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    let activity: Vec<(i64, NaiveDate)> = client
        .query(
            "SELECT DISTINCT s.visitor_id, s.day FROM sessions s
              WHERE s.project_id = $1 AND s.identified AND s.started_at >= $2 AND s.day >= $3
                AND EXISTS (SELECT 1 FROM visitors v WHERE v.project_id = s.project_id AND v.id = s.visitor_id
                             AND v.first_day BETWEEN $3 AND $4)",
            &[&project_id, &period.lower, &period.from, &period.to],
        )
        .await?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    Ok(Json(retention_table(unit, period.from, period.to, today, &first_days, &activity)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn period_starts_and_ranks() {
        assert_eq!(Unit::Week.start(d("2026-11-26")), d("2026-11-23"), "jeudi → lundi");
        assert_eq!(Unit::Week.start(d("2026-11-23")), d("2026-11-23"));
        assert_eq!(Unit::Month.start(d("2026-11-26")), d("2026-11-01"));
        assert_eq!(Unit::Week.index(d("2026-11-02"), d("2026-11-23")), 3);
        assert_eq!(Unit::Month.index(d("2025-11-01"), d("2026-02-01")), 3);
        assert_eq!(Unit::Day.index(d("2026-11-02"), d("2026-11-05")), 3);
    }

    #[test]
    fn weekly_retention() {
        // Semaine du 2 : visiteurs 1, 2, 3. Semaine du 9 : visiteur 4. Le 5 est hors période.
        let firsts = [
            (1, d("2026-11-02")),
            (2, d("2026-11-04")),
            (3, d("2026-11-08")),
            (4, d("2026-11-10")),
            (5, d("2026-10-20")),
        ];
        let activity = [
            (1, d("2026-11-02")),
            (1, d("2026-11-09")),
            (1, d("2026-11-11")), // deux visites la même semaine : compté une fois
            (1, d("2026-11-17")),
            (2, d("2026-11-04")),
            (2, d("2026-11-16")),
            (3, d("2026-11-08")),
            (4, d("2026-11-10")),
            (5, d("2026-11-10")),
        ];
        let table = retention_table(Unit::Week, d("2026-11-02"), d("2026-11-15"), d("2026-11-18"), &firsts, &activity);
        assert_eq!(table["visitors"], 4);
        let cohorts = table["cohorts"].as_array().unwrap();
        assert_eq!(cohorts.len(), 2);
        assert_eq!(cohorts[0], json!({ "start": "2026-11-02", "size": 3, "values": [3, 1, 2] }));
        assert_eq!(
            cohorts[1],
            json!({ "start": "2026-11-09", "size": 1, "values": [1, 0] }),
            "la semaine 2 n'a pas commencé"
        );
    }

    #[test]
    fn empty_cohorts_are_listed() {
        let table =
            retention_table(Unit::Day, d("2026-11-01"), d("2026-11-03"), d("2026-11-03"), &[(1, d("2026-11-03"))], &[]);
        let cohorts = table["cohorts"].as_array().unwrap();
        assert_eq!(cohorts.len(), 3);
        assert_eq!(cohorts[0]["size"], 0);
        assert_eq!(cohorts[2]["values"], json!([1]));
        let months = retention_table(Unit::Month, d("2026-01-15"), d("2026-03-02"), d("2026-03-02"), &[], &[]);
        let starts: Vec<&str> =
            months["cohorts"].as_array().unwrap().iter().map(|c| c["start"].as_str().unwrap()).collect();
        assert_eq!(starts, ["2026-01-01", "2026-02-01", "2026-03-01"]);
    }

    #[test]
    fn search_is_literal() {
        assert_eq!(like_pattern("50%_a\\"), "%50\\%\\_a\\\\%");
    }
}
