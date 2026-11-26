//! Enregistrements de session : réception des morceaux envoyés par le navigateur,
//! stockage sur disque et purge.
//!
//! Un enregistrement est un fichier `<dossier>/<projet>/<identifiant>.rec` : une suite de
//! blocs gzip mis bout à bout, un par morceau reçu. Chaque bloc décompressé est un tableau
//! JSON d'événements `rrweb` suivi d'un saut de ligne. La base ne garde que le résumé.
//!
//! Rien n'est enregistré pour un visiteur sans identifiant stable : celui-ci n'existe
//! qu'après son consentement ou pour un utilisateur connecté d'une application.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use deadpool_postgres::Pool;
use flate2::Compression;
use flate2::read::{GzDecoder, MultiGzDecoder};
use flate2::write::GzEncoder;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::AppState;
use crate::error::AppError;
use crate::{ingest, referrer, ua};

/// Taille maximale d'un morceau sur le réseau, puis une fois décompressé.
pub const MAX_WIRE_BYTES: usize = 2 << 20;
const MAX_CHUNK_BYTES: u64 = 16 << 20;
const MAX_EVENTS_PER_CHUNK: usize = 50_000;
/// Au-delà, l'enregistrement est clos : le navigateur reçoit 410 et s'arrête.
const MAX_RECORDING_BYTES: i64 = 64 << 20;
const MAX_RECORDING_MINUTES: i64 = 120;
/// Deux actions séparées de plus de cinq secondes ne comptent pas comme du temps actif.
const ACTIVE_GAP_MS: f64 = 5_000.0;

/// Réglages des enregistrements d'un projet (`projects.settings -> 'recording'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    pub enabled: bool,
    /// Part des sessions enregistrées, en pourcentage.
    pub sample: u8,
    pub retention_days: u16,
    /// Masque tout le texte des pages, pas seulement les champs de saisie.
    pub mask_text: bool,
    /// Plafond de stockage du projet, en mégaoctets.
    pub max_mb: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self { enabled: false, sample: 100, retention_days: 30, mask_text: false, max_mb: 500 }
    }
}

impl RecordingSettings {
    pub fn clamped(self) -> Self {
        Self {
            sample: self.sample.min(100),
            retention_days: self.retention_days.clamp(1, 365),
            max_mb: self.max_mb.clamp(10, 100_000),
            ..self
        }
    }

    pub fn from_settings(settings: &Value) -> Self {
        settings
            .get("recording")
            .and_then(|r| serde_json::from_value::<Self>(r.clone()).ok())
            .unwrap_or_default()
            .clamped()
    }
}

/// Identifiant d'enregistrement acceptable : il sert de nom de fichier.
pub fn valid_id(id: &str) -> bool {
    (16..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

pub fn file_path(data_dir: &Path, project_id: i64, id: &str) -> PathBuf {
    data_dir.join("recordings").join(project_id.to_string()).join(format!("{id}.rec"))
}

fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(Value::deserialize(d)?.as_u64())
}

fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        _ => None,
    })
}

/// Ce que le serveur lit d'un événement `rrweb` : le reste (l'arbre de la page) est ignoré sans être chargé.
#[derive(Deserialize)]
struct Lite {
    #[serde(rename = "type")]
    kind: u8,
    timestamp: f64,
    #[serde(default)]
    data: Option<LiteData>,
}

#[derive(Deserialize, Default)]
struct LiteData {
    #[serde(default, deserialize_with = "lenient_u64")]
    source: Option<u64>,
    #[serde(default, rename = "type", deserialize_with = "lenient_u64")]
    kind: Option<u64>,
    #[serde(default, deserialize_with = "lenient_string")]
    href: Option<String>,
    #[serde(default, deserialize_with = "lenient_u64")]
    width: Option<u64>,
    #[serde(default, deserialize_with = "lenient_u64")]
    height: Option<u64>,
}

const EVENT_INCREMENTAL: u8 = 3;
const EVENT_META: u8 = 4;
const SOURCE_MOUSE_INTERACTION: u64 = 2;
const MOUSE_CLICK: u64 = 2;
/// Sources qui prouvent une action du visiteur : souris, clic, défilement, saisie, toucher.
const ACTIVE_SOURCES: [u64; 5] = [1, 2, 3, 5, 6];

/// Résumé d'un morceau.
#[derive(Debug, Default, PartialEq)]
pub struct ChunkStats {
    pub events: i32,
    pub clicks: i32,
    pub pages: i32,
    pub active_ms: i64,
    pub span_ms: i64,
    pub first_href: Option<String>,
    pub hrefs: Vec<String>,
    pub view: Option<(i32, i32)>,
}

/// Lit le tableau d'événements d'un morceau. Erreur si ce n'en est pas un.
pub fn analyse(events_json: &str) -> Result<ChunkStats, &'static str> {
    let events: Vec<Lite> = serde_json::from_str(events_json).map_err(|_| "événements illisibles")?;
    if events.is_empty() || events.len() > MAX_EVENTS_PER_CHUNK {
        return Err("nombre d'événements invalide");
    }
    let mut stats = ChunkStats { events: events.len() as i32, ..Default::default() };
    let (mut min, mut max) = (f64::MAX, f64::MIN);
    let mut last_action: Option<f64> = None;
    for event in &events {
        if event.kind > 6 || !event.timestamp.is_finite() || event.timestamp < 0.0 {
            return Err("événement invalide");
        }
        min = min.min(event.timestamp);
        max = max.max(event.timestamp);
        let Some(data) = &event.data else { continue };
        if event.kind == EVENT_META {
            stats.pages += 1;
            if let Some(href) = &data.href {
                stats.first_href.get_or_insert_with(|| href.clone());
                stats.hrefs.push(href.clone());
            }
            if let (Some(w), Some(h)) = (data.width, data.height)
                && stats.view.is_none()
                && w <= 20_000
                && h <= 20_000
            {
                stats.view = Some((w as i32, h as i32));
            }
        } else if event.kind == EVENT_INCREMENTAL
            && let Some(source) = data.source
            && ACTIVE_SOURCES.contains(&source)
        {
            if source == SOURCE_MOUSE_INTERACTION && data.kind == Some(MOUSE_CLICK) {
                stats.clicks += 1;
            }
            if let Some(previous) = last_action {
                let gap = event.timestamp - previous;
                if gap > 0.0 && gap <= ACTIVE_GAP_MS {
                    stats.active_ms += gap as i64;
                }
            }
            last_action = Some(event.timestamp);
        }
    }
    stats.span_ms = ((max - min).max(0.0) as i64).min(3_600_000);
    Ok(stats)
}

fn gunzip_limited(body: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut out = Vec::new();
    GzDecoder::new(body)
        .take(MAX_CHUNK_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|_| AppError::bad("Corps compressé illisible."))?;
    if out.len() as u64 > MAX_CHUNK_BYTES {
        return Err(AppError::bad("Morceau trop volumineux."));
    }
    Ok(out)
}

/// Compresse un tableau d'événements et l'ajoute au fichier en une seule écriture.
fn append_chunk(path: &Path, events_json: &str) -> std::io::Result<u64> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(events_json.len() / 6 + 64), Compression::new(6));
    encoder.write_all(events_json.as_bytes())?;
    encoder.write_all(b"\n")?;
    let frame = encoder.finish()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&frame)?;
    Ok(frame.len() as u64)
}

/// Lit un enregistrement décompressé, par blocs, et les remet à `sink` (faux pour arrêter).
pub fn read_recording(path: &Path, mut sink: impl FnMut(Vec<u8>) -> bool) -> std::io::Result<()> {
    let mut decoder = MultiGzDecoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    loop {
        let mut block = vec![0u8; 64 * 1024];
        let read = decoder.read(&mut block)?;
        if read == 0 {
            return Ok(());
        }
        block.truncate(read);
        if !sink(block) {
            return Ok(());
        }
    }
}

pub fn delete_file(data_dir: &Path, project_id: i64, id: &str) {
    if valid_id(id) {
        let _ = std::fs::remove_file(file_path(data_dir, project_id, id));
    }
}

#[derive(Deserialize)]
struct Chunk<'a> {
    k: String,
    #[serde(default)]
    d: Option<String>,
    rid: String,
    #[serde(borrow)]
    e: &'a RawValue,
}

#[derive(Deserialize)]
pub struct WireOptions {
    /// « 1 » : le corps est compressé en gzip.
    #[serde(default)]
    z: Option<String>,
}

fn public(mut response: Response, cache: &'static str) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

pub async fn preflight() -> Response {
    public(StatusCode::NO_CONTENT.into_response(), "no-store")
}

#[derive(Deserialize)]
pub struct ConfigQuery {
    #[serde(default)]
    k: String,
}

/// `GET /r/config?k=<clé>` : ce que le traceur doit savoir avant de charger l'enregistreur.
pub async fn config(State(state): State<AppState>, Query(query): Query<ConfigQuery>) -> Response {
    let settings = match state.projects.by_key(&state.pool, &query.k).await {
        Ok(Some(project)) => project.recording,
        Ok(None) => RecordingSettings::default(),
        Err(e) => return public(e.into_response(), "no-store"),
    };
    let body = serde_json::json!({
        "record": settings.enabled && settings.sample > 0,
        "sample": settings.sample,
        "mask_text": settings.mask_text,
    });
    public(axum::Json(body).into_response(), "public, max-age=300")
}

enum Outcome {
    Stored,
    Ignored,
    /// L'enregistrement est clos ou refusé : le navigateur doit s'arrêter.
    Stop,
}

/// `POST /r` : un morceau d'enregistrement. 204 s'il est pris ou ignoré, 410 pour dire au
/// navigateur d'arrêter (enregistrements coupés, plafond atteint).
pub async fn collect(
    State(state): State<AppState>,
    ConnectInfo(_peer): ConnectInfo<SocketAddr>,
    Query(options): Query<WireOptions>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let compressed = options.z.as_deref() == Some("1");
    let response = match collect_inner(&state, &headers, compressed, body).await {
        Ok(Outcome::Stored | Outcome::Ignored) => StatusCode::NO_CONTENT.into_response(),
        Ok(Outcome::Stop) => StatusCode::GONE.into_response(),
        Err(e) => e.into_response(),
    };
    public(response, "no-store")
}

async fn collect_inner(
    state: &AppState,
    headers: &HeaderMap,
    compressed: bool,
    body: Bytes,
) -> Result<Outcome, AppError> {
    let user_agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let agent = ua::parse(user_agent);
    if agent.bot {
        return Ok(Outcome::Stop);
    }
    // Décompression et lecture hors du fil asynchrone : un morceau peut peser plusieurs mégaoctets.
    let parsed = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let text = if compressed { gunzip_limited(&body)? } else { body.to_vec() };
        let text = String::from_utf8(text).map_err(|_| AppError::bad("Corps de requête illisible."))?;
        let chunk: Chunk = serde_json::from_str(&text).map_err(|_| AppError::bad("Corps de requête illisible."))?;
        let stats = analyse(chunk.e.get()).map_err(AppError::bad)?;
        Ok((chunk.k, chunk.d, chunk.rid, chunk.e.get().to_owned(), stats))
    })
    .await
    .map_err(|e| AppError::Internal(format!("lecture d'un morceau : {e}")))??;
    let (key, distinct, rid, events_json, stats) = parsed;

    if !valid_id(&rid) {
        return Err(AppError::bad("Identifiant d'enregistrement invalide."));
    }
    let Some(project) = state.projects.by_key(&state.pool, &key).await? else {
        return Ok(Outcome::Stop);
    };
    if !project.recording.enabled {
        return Ok(Outcome::Stop);
    }
    let Some(distinct) = distinct.map(|d| ingest::clip(&d, 200)).filter(|d| d.len() >= 8) else {
        return Ok(Outcome::Stop);
    };
    let urls: Vec<referrer::ParsedUrl> = stats.hrefs.iter().filter_map(|h| referrer::parse_url(h)).collect();
    if urls.len() != stats.hrefs.len() || urls.iter().any(|u| !project.accepts_host(&u.host)) {
        return Ok(Outcome::Ignored);
    }
    let visitor = ingest::stable_visitor(project.id, &distinct);
    let now = Utc::now();

    let client = state.pool.get().await?;
    let existing = client
        .query_opt(
            "SELECT visitor_id, raw_bytes, started_at FROM recordings WHERE project_id = $1 AND id = $2",
            &[&project.id, &rid],
        )
        .await?;
    match &existing {
        Some(row) => {
            let owner: i64 = row.get(0);
            let raw: i64 = row.get(1);
            let started: chrono::DateTime<Utc> = row.get(2);
            if owner != visitor {
                return Ok(Outcome::Stop);
            }
            if raw >= MAX_RECORDING_BYTES || (now - started).num_minutes() >= MAX_RECORDING_MINUTES {
                return Ok(Outcome::Stop);
            }
        }
        None => {
            // Un enregistrement commence par l'état complet d'une page : sans lui, rien n'est rejouable.
            if stats.pages == 0 {
                return Ok(Outcome::Stop);
            }
        }
    }
    let used: i64 = client
        .query_one("SELECT COALESCE(SUM(bytes), 0)::bigint FROM recordings WHERE project_id = $1", &[&project.id])
        .await?
        .get(0);
    if used >= i64::from(project.recording.max_mb) << 20 {
        return Ok(Outcome::Stop);
    }

    let path = file_path(&state.config.data_dir, project.id, &rid);
    let raw_len = events_json.len() as i64 + 1;
    let stored = tokio::task::spawn_blocking(move || append_chunk(&path, &events_json))
        .await
        .map_err(|e| AppError::Internal(format!("écriture d'un morceau : {e}")))?
        .map_err(|e| AppError::Internal(format!("écriture d'un morceau : {e}")))? as i64;

    let entry = urls.first();
    let started = now - chrono::Duration::milliseconds(stats.span_ms);
    client
        .execute(
            "INSERT INTO recordings (project_id, id, visitor_id, started_at, last_at, active_ms, events, chunks, bytes,
                                     raw_bytes, clicks, pages, entry_host, entry_path, device, browser, os, view_w, view_h)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
             ON CONFLICT (project_id, id) DO UPDATE SET
                 last_at = GREATEST(recordings.last_at, EXCLUDED.last_at),
                 active_ms = recordings.active_ms + EXCLUDED.active_ms,
                 events = recordings.events + EXCLUDED.events,
                 chunks = recordings.chunks + 1,
                 bytes = recordings.bytes + EXCLUDED.bytes,
                 raw_bytes = recordings.raw_bytes + EXCLUDED.raw_bytes,
                 clicks = recordings.clicks + EXCLUDED.clicks,
                 pages = recordings.pages + EXCLUDED.pages,
                 view_w = COALESCE(recordings.view_w, EXCLUDED.view_w),
                 view_h = COALESCE(recordings.view_h, EXCLUDED.view_h)",
            &[
                &project.id,
                &rid,
                &visitor,
                &started,
                &now,
                &stats.active_ms,
                &stats.events,
                &stored,
                &raw_len,
                &stats.clicks,
                &stats.pages,
                &entry.map(|u| ingest::clip(&u.host, 255)).unwrap_or_default(),
                &entry.map(|u| ingest::clip(&u.path, 1024)).unwrap_or_default(),
                &agent.device,
                &agent.browser,
                &agent.os,
                &stats.view.map(|v| v.0),
                &stats.view.map(|v| v.1),
            ],
        )
        .await?;
    Ok(Outcome::Stored)
}

/// Supprime les enregistrements plus vieux que la durée de conservation de leur projet.
pub async fn purge(pool: &Pool, data_dir: &Path) -> Result<usize, AppError> {
    let client = pool.get().await?;
    let rows = client
        .query(
            "DELETE FROM recordings r USING projects p
              WHERE p.id = r.project_id
                AND r.last_at < now() - make_interval(days =>
                    LEAST(GREATEST(COALESCE((p.settings #>> '{recording,retention_days}')::int, 30), 1), 365))
          RETURNING r.project_id, r.id",
            &[],
        )
        .await?;
    for row in &rows {
        delete_file(data_dir, row.get(0), row.get::<_, &str>(1));
    }
    Ok(rows.len())
}

/// Purge au démarrage puis toutes les heures.
pub async fn purge_loop(pool: Pool, data_dir: PathBuf) {
    let mut tick = tokio::time::interval(Duration::from_secs(3600));
    loop {
        tick.tick().await;
        match purge(&pool, &data_dir).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(supprimes = n, "enregistrements expirés supprimés"),
            Err(e) => tracing::warn!(erreur = %e, "purge des enregistrements"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_and_limits() {
        let s = RecordingSettings::from_settings(&serde_json::json!({}));
        assert_eq!(s, RecordingSettings::default());
        assert!(!s.enabled, "coupé tant que le projet ne l'active pas");
        let s = RecordingSettings::from_settings(&serde_json::json!({
            "recording": { "enabled": true, "sample": 250, "retention_days": 0, "max_mb": 1 }
        }));
        assert_eq!((s.enabled, s.sample, s.retention_days, s.max_mb), (true, 100, 1, 10));
        let broken = RecordingSettings::from_settings(&serde_json::json!({ "recording": "oui" }));
        assert_eq!(broken, RecordingSettings::default());
    }

    #[test]
    fn identifiers_are_safe_file_names() {
        assert!(valid_id("0123456789abcdef0123456789abcdef"));
        assert!(valid_id("123e4567-e89b-12d3-a456-426614174000"));
        for bad in ["", "court", "../../etc/passwd-0000000", "0123456789abcdef/0123", "0123456789abcdeg0123"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn analyses_a_chunk() {
        let json = r#"[
            {"type":4,"timestamp":1000,"data":{"href":"https://demo.test/a?x=1","width":1280,"height":720}},
            {"type":2,"timestamp":1010,"data":{"node":{"type":0,"childNodes":[{"type":1,"name":"html"}]},"initialOffset":{"left":0,"top":0}}},
            {"type":3,"timestamp":2000,"data":{"source":1,"positions":[{"x":1,"y":2,"id":3,"timeOffset":0}]}},
            {"type":3,"timestamp":3000,"data":{"source":2,"type":2,"id":7,"x":10,"y":20}},
            {"type":3,"timestamp":3500,"data":{"source":2,"type":1,"id":7}},
            {"type":3,"timestamp":20000,"data":{"source":3,"id":1,"x":0,"y":400}},
            {"type":3,"timestamp":20500,"data":{"source":0,"adds":[],"removes":[],"texts":[],"attributes":[]}},
            {"type":5,"timestamp":21000,"data":{"tag":"t","payload":{"type":"texte","source":"libre"}}}
        ]"#;
        let stats = analyse(json).unwrap();
        assert_eq!((stats.events, stats.clicks, stats.pages), (8, 1, 1));
        assert_eq!(stats.active_ms, 1500, "le trou de 16,5 s n'est pas du temps actif");
        assert_eq!(stats.span_ms, 20000);
        assert_eq!(stats.first_href.as_deref(), Some("https://demo.test/a?x=1"));
        assert_eq!(stats.view, Some((1280, 720)));
    }

    #[test]
    fn rejects_what_is_not_a_recording() {
        for bad in
            ["[]", "{}", "[1,2]", r#"[{"type":9,"timestamp":1}]"#, r#"[{"type":3}]"#, r#"[{"type":3,"timestamp":-4}]"#]
        {
            assert!(analyse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn chunks_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("synalyse-test-{}", crate::ids::random_id()));
        let path = file_path(&dir, 7, "0123456789abcdef0123");
        let first = r#"[{"type":4,"timestamp":1,"data":{"href":"https://demo.test/"}}]"#;
        let second =
            format!(r#"[{{"type":3,"timestamp":2,"data":{{"source":0,"texts":["{}"]}}}}]"#, "é".repeat(100_000));
        let stored = append_chunk(&path, first).unwrap() + append_chunk(&path, &second).unwrap();
        assert_eq!(stored, std::fs::metadata(&path).unwrap().len());
        assert!(stored < second.len() as u64 / 20, "compressé");
        let mut out = Vec::new();
        read_recording(&path, |block| {
            out.extend(block);
            true
        })
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), format!("{first}\n{second}\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn decompression_is_bounded() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        let zeros = vec![b' '; 1 << 20];
        for _ in 0..17 {
            encoder.write_all(&zeros).unwrap();
        }
        let bomb = encoder.finish().unwrap();
        assert!(bomb.len() < 100_000);
        assert!(gunzip_limited(&bomb).is_err());
    }
}
