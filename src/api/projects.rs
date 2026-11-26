//! Projets, jetons d'API et import d'événements datés.

use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_postgres::Row;

use crate::AppState;
use crate::auth::{self, Need, Principal, Scope};
use crate::error::AppError;
use crate::ids;
use crate::ingest::{self, RawEvent, VisitorContext, writer::Msg};
use crate::recording::RecordingSettings;
use crate::ua;

const PROJECT_COLUMNS: &str = "id, org_id, name, public_key, timezone, domains, created_at, settings";

fn project_json(row: &Row) -> Value {
    json!({
        "id": row.get::<_, i64>(0),
        "org_id": row.get::<_, i64>(1),
        "name": row.get::<_, String>(2),
        "public_key": row.get::<_, String>(3),
        "timezone": row.get::<_, String>(4),
        "domains": row.get::<_, Vec<String>>(5),
        "created_at": row.get::<_, DateTime<Utc>>(6),
        "recording": RecordingSettings::from_settings(&row.get::<_, Value>(7)),
    })
}

fn clean_name(name: &str) -> Result<String, AppError> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(100).collect();
    if name.is_empty() { Err(AppError::bad("Le nom est obligatoire.")) } else { Ok(name) }
}

fn clean_timezone(timezone: &str) -> Result<String, AppError> {
    timezone.parse::<Tz>().map(|tz| tz.name().to_string()).map_err(|_| AppError::bad("Fuseau horaire inconnu."))
}

/// Normalise une liste de domaines : « https://www.Exemple.fr/ » devient « exemple.fr ».
fn clean_domains(domains: &[String]) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    for raw in domains.iter().take(50) {
        let host = raw.trim().to_ascii_lowercase();
        let host = host.strip_prefix("https://").or_else(|| host.strip_prefix("http://")).unwrap_or(&host);
        let host = host.split(['/', ':', '?']).next().unwrap_or("").trim_start_matches("www.").trim_end_matches('.');
        if host.is_empty() {
            continue;
        }
        let valid = host.len() <= 253 && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if !valid {
            return Err(AppError::bad(format!("Domaine invalide : {raw}")));
        }
        if !out.iter().any(|d| d == host) {
            out.push(host.to_string());
        }
    }
    Ok(out)
}

pub async fn list(
    State(state): State<AppState>,
    principal: Principal,
    Path(org_id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_org(&client, &principal, org_id, false).await?;
    let rows = client
        .query(
            &format!(
                "SELECT {PROJECT_COLUMNS} FROM projects WHERE org_id = $1 AND archived_at IS NULL ORDER BY name, id"
            ),
            &[&org_id],
        )
        .await?;
    Ok(Json(json!({ "projects": rows.iter().map(project_json).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
pub struct NewProject {
    name: String,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    domains: Vec<String>,
}

pub async fn create(
    State(state): State<AppState>,
    principal: Principal,
    Path(org_id): Path<i64>,
    Json(body): Json<NewProject>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_org(&client, &principal, org_id, true).await?;
    let name = clean_name(&body.name)?;
    let timezone = clean_timezone(body.timezone.as_deref().unwrap_or("UTC"))?;
    let domains = clean_domains(&body.domains)?;
    let row = client
        .query_one(
            &format!(
                "INSERT INTO projects (org_id, name, public_key, timezone, domains) VALUES ($1, $2, $3, $4, $5)
                 RETURNING {PROJECT_COLUMNS}"
            ),
            &[&org_id, &name, &ids::random_public_key(), &timezone, &domains],
        )
        .await?;
    Ok(Json(project_json(&row)))
}

pub async fn get(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Read).await?;
    let row =
        client.query_one(&format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = $1"), &[&project_id]).await?;
    Ok(Json(project_json(&row)))
}

#[derive(Deserialize)]
pub struct ProjectPatch {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    domains: Option<Vec<String>>,
    #[serde(default)]
    recording: Option<RecordingSettings>,
}

pub async fn update(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Json(body): Json<ProjectPatch>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Admin).await?;
    let name = body.name.as_deref().map(clean_name).transpose()?;
    let timezone = body.timezone.as_deref().map(clean_timezone).transpose()?;
    let domains = body.domains.as_deref().map(clean_domains).transpose()?;
    let recording = body.recording.map(|r| serde_json::to_value(r.clamped()).expect("réglages sérialisables"));
    let row = client
        .query_one(
            &format!(
                "UPDATE projects SET name = COALESCE($2, name), timezone = COALESCE($3, timezone),
                        domains = COALESCE($4, domains),
                        settings = CASE WHEN $5::jsonb IS NULL THEN settings
                                        ELSE jsonb_set(settings, '{{recording}}', $5::jsonb) END
                  WHERE id = $1 RETURNING {PROJECT_COLUMNS}"
            ),
            &[&project_id, &name, &timezone, &domains, &recording],
        )
        .await?;
    state.projects.invalidate();
    Ok(Json(project_json(&row)))
}

pub async fn list_tokens(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Admin).await?;
    let rows = client
        .query(
            "SELECT id, name, scope, created_at, last_used_at FROM api_tokens
              WHERE project_id = $1 AND revoked_at IS NULL ORDER BY id",
            &[&project_id],
        )
        .await?;
    Ok(Json(json!({ "tokens": rows.iter().map(|r| json!({
        "id": r.get::<_, i64>(0), "name": r.get::<_, String>(1), "scope": r.get::<_, String>(2),
        "created_at": r.get::<_, DateTime<Utc>>(3), "last_used_at": r.get::<_, Option<DateTime<Utc>>>(4),
    })).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
pub struct NewToken {
    name: String,
    scope: String,
}

/// Crée un jeton. Sa valeur n'est renvoyée qu'ici : seule son empreinte est conservée.
pub async fn create_token(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Json(body): Json<NewToken>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Admin).await?;
    let name = clean_name(&body.name)?;
    Scope::parse(&body.scope).ok_or_else(|| AppError::bad("Portée inconnue : read, ingest ou admin."))?;
    let token = format!("syn_{}", ids::random_token());
    let created_by = principal.user_id().ok();
    let row = client
        .query_one(
            "INSERT INTO api_tokens (project_id, name, scope, token_hash, created_by) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            &[&project_id, &name, &body.scope, &auth::token_hash(&token).as_slice(), &created_by],
        )
        .await?;
    Ok(Json(json!({ "id": row.get::<_, i64>(0), "name": name, "scope": body.scope, "token": token })))
}

pub async fn revoke_token(
    State(state): State<AppState>,
    principal: Principal,
    Path((project_id, token_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Admin).await?;
    let n = client
        .execute(
            "UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND project_id = $2 AND revoked_at IS NULL",
            &[&token_id, &project_id],
        )
        .await?;
    if n == 0 { Err(AppError::NotFound) } else { Ok(Json(json!({ "revoked": token_id }))) }
}

#[derive(Deserialize)]
pub struct ImportEvent {
    /// Horodatage Unix en millisecondes.
    ts: i64,
    /// Identifiant du visiteur dans le système d'origine ; il est haché, jamais stocké.
    visitor: String,
    #[serde(default)]
    ua: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    screen_w: Option<i32>,
    #[serde(flatten)]
    event: RawEvent,
}

#[derive(Deserialize)]
pub struct Import {
    events: Vec<ImportEvent>,
}

const IMPORT_MAX: usize = 5000;

/// Import d'événements datés (reprise d'historique, jeux d'essai). La réponse n'arrive
/// qu'une fois les événements écrits. Les événements d'un même visiteur doivent être
/// envoyés dans l'ordre chronologique.
pub async fn import(
    State(state): State<AppState>,
    principal: Principal,
    Path(project_id): Path<i64>,
    Json(body): Json<Import>,
) -> Result<Json<Value>, AppError> {
    let client = state.pool.get().await?;
    auth::authorize_project(&client, &principal, project_id, Need::Ingest).await?;
    if body.events.len() > IMPORT_MAX {
        return Err(AppError::bad(format!("Au plus {IMPORT_MAX} événements par appel.")));
    }
    let row = client.query_one("SELECT timezone, domains FROM projects WHERE id = $1", &[&project_id]).await?;
    drop(client);
    let timezone: String = row.get(0);
    let project = std::sync::Arc::new(ingest::Project {
        id: project_id,
        tz: timezone.parse().unwrap_or(chrono_tz::UTC),
        domains: row.get(1),
        recording: RecordingSettings::default(),
    });

    let (mut accepted, mut rejected) = (0usize, 0usize);
    for item in &body.events {
        let agent = ua::parse(item.ua.as_deref().unwrap_or(""));
        let built = DateTime::<Utc>::from_timestamp_millis(item.ts).and_then(|ts| {
            let ctx = VisitorContext {
                project: project.clone(),
                visitor: ingest::stable_visitor(project_id, &format!("import:{}", item.visitor)),
                visitor_prev: None,
                identified: false,
                agent,
                lang: item.lang.clone().filter(|l| l.len() <= 16),
                screen_w: item.screen_w.filter(|w| (1..=20_000).contains(w)),
            };
            ingest::build_event(&ctx, ts, &item.event)
        });
        match built {
            Some(event) => {
                state
                    .writer
                    .send(Msg::Event(Box::new(event)))
                    .await
                    .map_err(|_| AppError::Internal("rédacteur arrêté".into()))?;
                accepted += 1;
            }
            None => rejected += 1,
        }
    }
    let (reply, done) = oneshot::channel();
    state.writer.send(Msg::Flush(reply)).await.map_err(|_| AppError::Internal("rédacteur arrêté".into()))?;
    done.await.map_err(|_| AppError::Internal("rédacteur arrêté".into()))?;
    Ok(Json(json!({ "accepted": accepted, "rejected": rejected })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_domains() {
        let input =
            vec!["https://www.Exemple.fr/".to_string(), "exemple.fr".into(), " app.exemple.fr:8443 ".into(), "".into()];
        assert_eq!(clean_domains(&input).unwrap(), vec!["exemple.fr", "app.exemple.fr"]);
        assert!(clean_domains(&["exemple .fr".to_string()]).is_err());
        assert!(clean_domains(&["<script>".to_string()]).is_err());
    }

    #[test]
    fn validates_names_and_timezones() {
        assert_eq!(clean_name("  Boutique\n ").unwrap(), "Boutique");
        assert!(clean_name("   ").is_err());
        assert_eq!(clean_timezone("Europe/Paris").unwrap(), "Europe/Paris");
        assert!(clean_timezone("Mars/Olympus").is_err());
    }
}
