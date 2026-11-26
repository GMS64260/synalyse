//! Réception des événements : validation, enrichissement, puis remise au rédacteur
//! (`writer`) qui les écrit par lots.

pub mod writer;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use deadpool_postgres::Pool;
use serde::Deserialize;

use crate::AppState;
use crate::error::AppError;
use crate::ids;
use crate::recording::RecordingSettings;
use crate::referrer::{self, Acquisition};
use crate::ua::{self, Agent};

pub const KIND_PAGEVIEW: i16 = 0;
pub const KIND_EVENT: i16 = 1;

const MAX_EVENTS_PER_REQUEST: usize = 50;
const MAX_PROPS_BYTES: usize = 4096;
const MAX_TRAITS_BYTES: usize = 2048;
/// Retard maximal accepté pour un événement envoyé par le navigateur (file d'attente hors ligne).
const MAX_CLIENT_DELAY_MS: u64 = 3_600_000;

/// Événement prêt à être écrit.
#[derive(Debug, Clone)]
pub struct IngestEvent {
    pub project_id: i64,
    pub tz: Tz,
    pub ts: DateTime<Utc>,
    pub visitor: i64,
    /// Autres identités sous lesquelles une session en cours peut exister : l'empreinte
    /// calculée avec le sel de la veille (la session ne se coupe pas à minuit UTC) et, pour
    /// un visiteur qui vient d'être identifié, son empreinte anonyme (sa visite continue).
    pub fallbacks: [Option<i64>; 2],
    pub identified: bool,
    /// Identifiant stable tel qu'envoyé, pour la fiche du visiteur. None pour un anonyme.
    pub distinct: Option<String>,
    /// Propriétés transmises avec identify.
    pub traits: Option<serde_json::Value>,
    pub kind: i16,
    pub name: Option<String>,
    pub host: String,
    pub path: String,
    pub title: Option<String>,
    pub props: Option<serde_json::Value>,
    pub acquisition: Acquisition,
    pub agent: Agent,
    pub lang: Option<String>,
    pub screen_w: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: i64,
    pub tz: Tz,
    pub domains: Vec<String>,
    pub recording: RecordingSettings,
}

impl Project {
    /// Vrai si l'hôte est autorisé à envoyer des événements pour ce projet.
    pub fn accepts_host(&self, host: &str) -> bool {
        self.domains.is_empty()
            || self.domains.iter().any(|d| {
                let d = d.trim_start_matches("www.");
                let h = host.trim_start_matches("www.");
                h == d || h.ends_with(&format!(".{d}"))
            })
    }
}

/// Instant de la lecture en base et projet trouvé (None : clé inconnue).
type CachedProject = (Instant, Option<Arc<Project>>);

/// Projets connus par clé publique, gardés une minute en mémoire (absences comprises).
#[derive(Default)]
pub struct ProjectCache {
    entries: RwLock<HashMap<String, CachedProject>>,
}

const PROJECT_TTL: Duration = Duration::from_secs(60);

impl ProjectCache {
    pub async fn by_key(&self, pool: &Pool, key: &str) -> Result<Option<Arc<Project>>, AppError> {
        if let Some((at, project)) = self.entries.read().expect("verrou du cache").get(key)
            && at.elapsed() < PROJECT_TTL
        {
            return Ok(project.clone());
        }
        let client = pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id, timezone, domains, settings FROM projects WHERE public_key = $1 AND archived_at IS NULL",
                &[&key],
            )
            .await?;
        let project = row.map(|row| {
            let tz: String = row.get(1);
            Arc::new(Project {
                id: row.get(0),
                tz: tz.parse().unwrap_or(chrono_tz::UTC),
                domains: row.get(2),
                recording: RecordingSettings::from_settings(&row.get::<_, serde_json::Value>(3)),
            })
        });
        let mut entries = self.entries.write().expect("verrou du cache");
        if entries.len() > 10_000 {
            entries.clear(); // des clés inventées ne doivent pas faire grossir le cache sans fin
        }
        entries.insert(key.to_string(), (Instant::now(), project.clone()));
        Ok(project)
    }

    /// À appeler quand un projet est modifié sur ce serveur.
    pub fn invalidate(&self) {
        self.entries.write().expect("verrou du cache").clear();
    }
}

/// Sels quotidiens partagés entre tous les serveurs par la table `visitor_salts`.
#[derive(Default)]
pub struct SaltStore {
    salts: RwLock<HashMap<NaiveDate, [u8; 32]>>,
}

impl SaltStore {
    pub async fn get(&self, pool: &Pool, day: NaiveDate) -> Result<[u8; 32], AppError> {
        if let Some(salt) = self.salts.read().expect("verrou des sels").get(&day) {
            return Ok(*salt);
        }
        let client = pool.get().await?;
        let candidate = ids::random_bytes::<32>();
        client
            .execute(
                "INSERT INTO visitor_salts (day, salt) VALUES ($1, $2) ON CONFLICT (day) DO NOTHING",
                &[&day, &candidate.as_slice()],
            )
            .await?;
        let stored: Vec<u8> = client.query_one("SELECT salt FROM visitor_salts WHERE day = $1", &[&day]).await?.get(0);
        let salt: [u8; 32] = stored.try_into().map_err(|_| AppError::Internal("sel de taille inattendue".into()))?;
        // Un sel de plus de deux jours ne sert plus : sa suppression rend les anciennes empreintes irréversibles.
        client.execute("DELETE FROM visitor_salts WHERE day < CURRENT_DATE - 2", &[]).await?;
        let mut salts = self.salts.write().expect("verrou des sels");
        salts.retain(|d, _| (day - *d).num_days().abs() <= 2);
        salts.insert(day, salt);
        Ok(salt)
    }
}

/// Empreinte d'un visiteur sans consentement : dépend du jour, du projet, de l'adresse
/// et du navigateur. L'adresse IP n'est ni stockée ni journalisée.
pub fn fingerprint(salt: &[u8; 32], project_id: i64, ip: IpAddr, user_agent: &str) -> i64 {
    ids::hash_to_id(&[salt, &project_id.to_be_bytes(), ip.to_string().as_bytes(), user_agent.as_bytes()])
}

/// Identifiant d'un visiteur qui porte un identifiant stable (consentement, import, utilisateur connecté).
pub fn stable_visitor(project_id: i64, distinct_id: &str) -> i64 {
    ids::hash_to_id(&[b"stable", &project_id.to_be_bytes(), distinct_id.as_bytes()])
}

/// Identifiant stable envoyé par le navigateur, nettoyé, ou None s'il n'est pas recevable.
/// Un identifiant d'utilisateur connecté (`u:…`) peut être court ; un identifiant tiré au
/// hasard par le traceur fait au moins huit caractères.
pub(crate) fn stable_id(raw: Option<&str>) -> Option<String> {
    let id = clip(raw?, 200);
    let user = id.strip_prefix("u:").is_some_and(|rest| !rest.is_empty());
    (user || id.len() >= 8).then_some(id)
}

/// Adresse du visiteur, compte tenu des mandataires de confiance.
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr, trusted_proxies: usize) -> IpAddr {
    if trusted_proxies == 0 {
        return peer.ip();
    }
    let forwarded: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .collect();
    // Chaque mandataire de confiance ajoute une adresse à droite : celle du visiteur
    // est la n-ième en partant de la fin. Tout ce qui est plus à gauche est falsifiable.
    forwarded
        .len()
        .checked_sub(trusted_proxies)
        .and_then(|i| forwarded.get(i))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| peer.ip())
}

#[derive(Debug, Deserialize)]
pub struct RawEvent {
    /// Type : « pageview » (défaut) ou « event ».
    #[serde(default)]
    pub t: Option<String>,
    /// Nom de l'événement personnalisé.
    #[serde(default)]
    pub n: Option<String>,
    /// URL complète de la page.
    pub u: String,
    /// Référent.
    #[serde(default)]
    pub r: Option<String>,
    /// Titre de la page.
    #[serde(default)]
    pub ti: Option<String>,
    /// Propriétés libres.
    #[serde(default)]
    pub p: Option<serde_json::Value>,
    /// Ancienneté de l'événement en millisecondes au moment de l'envoi.
    #[serde(default)]
    pub o: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Payload {
    k: String,
    #[serde(default)]
    d: Option<String>,
    #[serde(default)]
    sw: Option<i32>,
    #[serde(default)]
    l: Option<String>,
    /// Propriétés du visiteur identifié (offre, rôle…).
    #[serde(default)]
    tr: Option<serde_json::Value>,
    e: Vec<RawEvent>,
}

pub struct VisitorContext {
    pub project: Arc<Project>,
    pub visitor: i64,
    pub fallbacks: [Option<i64>; 2],
    pub identified: bool,
    pub distinct: Option<String>,
    pub traits: Option<serde_json::Value>,
    pub agent: Agent,
    pub lang: Option<String>,
    pub screen_w: Option<i32>,
}

pub(crate) fn clip(text: &str, max: usize) -> String {
    text.trim().chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Propriétés d'un visiteur : un objet de valeurs simples, de taille bornée.
pub(crate) fn clean_traits(traits: Option<serde_json::Value>) -> Option<serde_json::Value> {
    let object = traits?.as_object()?.clone();
    let kept: serde_json::Map<String, serde_json::Value> = object
        .into_iter()
        .filter(|(key, value)| {
            key.chars().count() <= 60 && (value.is_string() || value.is_number() || value.is_boolean())
        })
        .take(30)
        .collect();
    let value = serde_json::Value::Object(kept);
    let size = serde_json::to_string(&value).map(|s| s.len()).unwrap_or(usize::MAX);
    (size > 2 && size <= MAX_TRAITS_BYTES).then_some(value)
}

fn clip_opt(text: Option<&str>, max: usize) -> Option<String> {
    text.map(|t| clip(t, max)).filter(|t| !t.is_empty())
}

/// Transforme un événement brut en événement prêt à écrire, ou None s'il est à ignorer.
pub fn build_event(ctx: &VisitorContext, ts: DateTime<Utc>, raw: &RawEvent) -> Option<IngestEvent> {
    let url = referrer::parse_url(&raw.u)?;
    if !ctx.project.accepts_host(&url.host) {
        return None;
    }
    let (kind, name) = match raw.t.as_deref().unwrap_or("pageview") {
        "pageview" => (KIND_PAGEVIEW, None),
        "event" => (KIND_EVENT, Some(clip_opt(raw.n.as_deref(), 100)?)),
        _ => return None,
    };
    let props = raw
        .p
        .as_ref()
        .filter(|p| p.is_object() && serde_json::to_string(p).map(|s| s.len() <= MAX_PROPS_BYTES).unwrap_or(false))
        .cloned();
    Some(IngestEvent {
        project_id: ctx.project.id,
        tz: ctx.project.tz,
        ts,
        visitor: ctx.visitor,
        fallbacks: ctx.fallbacks,
        identified: ctx.identified,
        distinct: ctx.distinct.clone(),
        traits: ctx.traits.clone(),
        kind,
        name,
        acquisition: referrer::classify(&url, raw.r.as_deref()),
        host: clip(&url.host, 255),
        path: clip(&url.path, 1024),
        title: clip_opt(raw.ti.as_deref(), 300),
        props,
        agent: ctx.agent,
        lang: ctx.lang.clone(),
        screen_w: ctx.screen_w,
    })
}

fn cors(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("POST, OPTIONS"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn preflight() -> Response {
    cors(StatusCode::NO_CONTENT.into_response())
}

/// `POST /i` : point d'entrée public du traceur. Répond toujours 204 quand la requête est
/// lisible, y compris pour un robot ou une clé inconnue : le traceur n'a rien à apprendre du refus.
pub async fn collect(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    cors(match collect_inner(&state, peer, &headers, &body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    })
}

async fn collect_inner(state: &AppState, peer: SocketAddr, headers: &HeaderMap, body: &[u8]) -> Result<(), AppError> {
    let payload: Payload = serde_json::from_slice(body).map_err(|_| AppError::bad("Corps de requête illisible."))?;
    if payload.e.is_empty() || payload.e.len() > MAX_EVENTS_PER_REQUEST {
        return Err(AppError::bad("Nombre d'événements invalide."));
    }
    let user_agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let agent = ua::parse(user_agent);
    if agent.bot {
        return Ok(());
    }
    let Some(project) = state.projects.by_key(&state.pool, &payload.k).await? else {
        return Ok(());
    };

    let now = Utc::now();
    let distinct = stable_id(payload.d.as_deref());
    // L'empreinte anonyme est calculée dans tous les cas : pour un visiteur identifié, elle
    // sert à retrouver la visite qu'il avait commencée avant de l'être.
    let ip = client_ip(headers, peer, state.config.trusted_proxies);
    let today = now.date_naive();
    let salt = state.salts.get(&state.pool, today).await?;
    let previous = state.salts.get(&state.pool, today.pred_opt().expect("veille")).await?;
    let anonymous = fingerprint(&salt, project.id, ip, user_agent);
    let anonymous_prev = fingerprint(&previous, project.id, ip, user_agent);
    let (visitor, fallbacks) = match &distinct {
        Some(d) => (stable_visitor(project.id, d), [Some(anonymous), Some(anonymous_prev)]),
        None => (anonymous, [Some(anonymous_prev), None]),
    };
    let ctx = VisitorContext {
        project,
        visitor,
        fallbacks,
        identified: distinct.is_some(),
        traits: distinct.as_ref().and(clean_traits(payload.tr)),
        distinct,
        agent,
        lang: clip_opt(payload.l.as_deref(), 16),
        screen_w: payload.sw.filter(|w| (1..=20_000).contains(w)),
    };
    for raw in &payload.e {
        let delay = raw.o.unwrap_or(0).min(MAX_CLIENT_DELAY_MS);
        let ts = now - chrono::Duration::milliseconds(delay as i64);
        if let Some(event) = build_event(&ctx, ts, raw) {
            // File pleine : on perd l'événement plutôt que de faire attendre le navigateur.
            if state.writer.try_send(writer::Msg::Event(Box::new(event))).is_err() {
                tracing::warn!("file d'écriture pleine, événement abandonné");
                return Err(AppError::TooManyRequests);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(xff: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in xff {
            h.append("x-forwarded-for", HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn client_ip_respects_trusted_proxies() {
        let peer: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        let spoofed = headers(&["6.6.6.6, 203.0.113.7"]);
        assert_eq!(client_ip(&spoofed, peer, 0), peer.ip(), "sans mandataire, l'en-tête est ignoré");
        assert_eq!(client_ip(&spoofed, peer, 1).to_string(), "203.0.113.7", "l'adresse falsifiée à gauche est ignorée");
        assert_eq!(client_ip(&headers(&["6.6.6.6", "203.0.113.7, 10.0.0.9"]), peer, 2).to_string(), "203.0.113.7");
        assert_eq!(client_ip(&headers(&[]), peer, 1), peer.ip());
        assert_eq!(client_ip(&headers(&["n'importe quoi"]), peer, 1), peer.ip());
    }

    #[test]
    fn traits_are_small_flat_objects() {
        let ok = clean_traits(Some(
            serde_json::json!({ "offre": "pro", "sièges": 4, "essai": false, "liste": [1], "o": {} }),
        ));
        assert_eq!(ok, Some(serde_json::json!({ "offre": "pro", "sièges": 4, "essai": false })));
        assert_eq!(clean_traits(Some(serde_json::json!(["a"]))), None);
        assert_eq!(clean_traits(Some(serde_json::json!({}))), None);
        assert_eq!(clean_traits(Some(serde_json::json!({ "x": "a".repeat(3000) }))), None);
        assert_eq!(clean_traits(None), None);
    }

    #[test]
    fn stable_ids() {
        assert_eq!(stable_id(Some("u:42")).as_deref(), Some("u:42"), "utilisateur connecté à identifiant court");
        assert_eq!(stable_id(Some(" 123e4567-e89b ")).as_deref(), Some("123e4567-e89b"));
        for bad in [None, Some(""), Some("u:"), Some("court"), Some("1234567")] {
            assert_eq!(stable_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn fingerprints_depend_on_every_input() {
        let (s1, s2) = ([1u8; 32], [2u8; 32]);
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let base = fingerprint(&s1, 1, ip, "UA");
        assert_eq!(base, fingerprint(&s1, 1, ip, "UA"));
        assert_ne!(base, fingerprint(&s2, 1, ip, "UA"), "autre jour");
        assert_ne!(base, fingerprint(&s1, 2, ip, "UA"), "autre projet : pas de suivi entre sites");
        assert_ne!(base, fingerprint(&s1, 1, "203.0.113.8".parse().unwrap(), "UA"));
        assert_ne!(base, fingerprint(&s1, 1, ip, "UB"));
    }

    fn ctx(domains: &[&str]) -> VisitorContext {
        VisitorContext {
            project: Arc::new(Project {
                id: 1,
                tz: chrono_tz::Europe::Paris,
                domains: domains.iter().map(|d| d.to_string()).collect(),
                recording: RecordingSettings::default(),
            }),
            visitor: 1,
            fallbacks: [None, None],
            identified: false,
            distinct: None,
            traits: None,
            agent: ua::parse("Mozilla/5.0 (Windows NT 10.0) Chrome/130.0 Safari/537.36"),
            lang: None,
            screen_w: None,
        }
    }

    fn raw(json: &str) -> RawEvent {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn builds_and_filters_events() {
        let now = Utc::now();
        let pv = build_event(&ctx(&[]), now, &raw(r#"{"u":"https://demo.test/a?x=1","ti":"  Titre\n "}"#)).unwrap();
        assert_eq!((pv.kind, pv.path.as_str(), pv.title.as_deref()), (KIND_PAGEVIEW, "/a", Some("Titre")));

        let ev =
            build_event(&ctx(&[]), now, &raw(r#"{"t":"event","n":"Ajout","u":"https://demo.test/a","p":{"sku":"x"}}"#))
                .unwrap();
        assert_eq!((ev.kind, ev.name.as_deref()), (KIND_EVENT, Some("Ajout")));
        assert!(ev.props.is_some());

        assert!(
            build_event(&ctx(&[]), now, &raw(r#"{"t":"event","u":"https://demo.test/a"}"#)).is_none(),
            "événement sans nom"
        );
        assert!(build_event(&ctx(&[]), now, &raw(r#"{"t":"autre","u":"https://demo.test/a"}"#)).is_none());
        assert!(build_event(&ctx(&[]), now, &raw(r#"{"u":"javascript:alert(1)"}"#)).is_none());
        let big = format!(r#"{{"u":"https://demo.test/","p":{{"x":"{}"}}}}"#, "a".repeat(5000));
        assert!(build_event(&ctx(&[]), now, &raw(&big)).unwrap().props.is_none(), "propriétés trop lourdes écartées");
        let not_object = build_event(&ctx(&[]), now, &raw(r#"{"u":"https://demo.test/","p":[1,2]}"#)).unwrap();
        assert!(not_object.props.is_none());
    }

    #[test]
    fn domain_allow_list() {
        let now = Utc::now();
        let allowed = ctx(&["demo.test"]);
        assert!(build_event(&allowed, now, &raw(r#"{"u":"https://www.demo.test/"}"#)).is_some());
        assert!(build_event(&allowed, now, &raw(r#"{"u":"https://app.demo.test/"}"#)).is_some());
        assert!(build_event(&allowed, now, &raw(r#"{"u":"https://evildemo.test/"}"#)).is_none());
        assert!(build_event(&allowed, now, &raw(r#"{"u":"https://autre.test/"}"#)).is_none());
    }
}
