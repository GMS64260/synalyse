//! Enregistrements de session : réception des morceaux envoyés par le navigateur,
//! stockage sur disque et purge.
//!
//! Un enregistrement est un fichier `<dossier>/<projet>/<identifiant>.rec` : une suite de
//! blocs gzip mis bout à bout, un par morceau reçu. Chaque bloc décompressé est un tableau
//! JSON d'événements `rrweb` suivi d'un saut de ligne. La base ne garde que le résumé.
//!
//! Rien n'est enregistré pour un visiteur sans identifiant stable : celui-ci n'existe
//! qu'après son consentement ou pour un utilisateur connecté d'une application.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use deadpool_postgres::Pool;
use flate2::Compression;
use flate2::read::{GzDecoder, MultiGzDecoder};
use flate2::write::GzEncoder;
use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
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
/// Morceaux décompressés et lus en même temps : chacun peut occuper quelques dizaines de mégaoctets.
const PARALLEL_CHUNKS: usize = 4;
static CHUNK_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(PARALLEL_CHUNKS);
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

/// Modification partielle des réglages : un champ absent garde sa valeur.
#[derive(Debug, Default, Deserialize)]
pub struct RecordingPatch {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    sample: Option<u8>,
    #[serde(default)]
    retention_days: Option<u16>,
    #[serde(default)]
    mask_text: Option<bool>,
    #[serde(default)]
    max_mb: Option<u32>,
}

impl RecordingSettings {
    pub fn patched(self, patch: &RecordingPatch) -> Self {
        Self {
            enabled: patch.enabled.unwrap_or(self.enabled),
            sample: patch.sample.unwrap_or(self.sample),
            retention_days: patch.retention_days.unwrap_or(self.retention_days),
            mask_text: patch.mask_text.unwrap_or(self.mask_text),
            max_mb: patch.max_mb.unwrap_or(self.max_mb),
        }
        .clamped()
    }

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

/// Nombre entier lu s'il y en a un, sinon rien. Un tableau ou un objet à cette place est
/// parcouru sans être chargé en mémoire : le contenu d'un envoi n'est pas digne de confiance.
#[derive(Default)]
struct LooseU64(Option<u64>);

/// Chaîne lue s'il y en a une (tronquée), sinon rien.
#[derive(Default)]
struct LooseString(Option<String>);

struct LooseVisitor<T>(std::marker::PhantomData<T>);

macro_rules! skip_everything_else {
    () => {
        fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(Default::default())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(Default::default())
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(Default::default())
        }
    };
}

impl<'de> Visitor<'de> for LooseVisitor<LooseU64> {
    type Value = LooseU64;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("une valeur JSON")
    }
    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(LooseU64(Some(v)))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(LooseU64(u64::try_from(v).ok()))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
        Ok(LooseU64(((0.0..1e15).contains(&v) && v.fract() == 0.0).then_some(v as u64)))
    }
    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(LooseU64(None))
    }
    skip_everything_else!();
}

impl<'de> Visitor<'de> for LooseVisitor<LooseString> {
    type Value = LooseString;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("une valeur JSON")
    }
    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(LooseString(None))
    }
    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(LooseString(None))
    }
    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(LooseString(None))
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(LooseString(Some(v.chars().take(2048).collect())))
    }
    skip_everything_else!();
}

impl<'de> Deserialize<'de> for LooseU64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(LooseVisitor::<Self>(std::marker::PhantomData))
    }
}

impl<'de> Deserialize<'de> for LooseString {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(LooseVisitor::<Self>(std::marker::PhantomData))
    }
}

/// Ce que le serveur lit d'un événement `rrweb` : le reste (l'arbre de la page) est ignoré sans être chargé.
#[derive(Deserialize)]
pub struct Lite {
    #[serde(rename = "type")]
    kind: u8,
    timestamp: f64,
    #[serde(default)]
    data: Option<LiteData>,
}

#[derive(Deserialize, Default)]
struct LiteData {
    #[serde(default)]
    source: LooseU64,
    #[serde(default, rename = "type")]
    kind: LooseU64,
    #[serde(default)]
    href: LooseString,
    #[serde(default)]
    width: LooseU64,
    #[serde(default)]
    height: LooseU64,
}

/// Liste d'événements bornée pendant la lecture : un envoi démesuré est refusé avant d'occuper la mémoire.
struct Events(Vec<Lite>);

impl<'de> Deserialize<'de> for Events {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ListVisitor;
        impl<'de> Visitor<'de> for ListVisitor {
            type Value = Events;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("un tableau d'événements")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Events, A::Error> {
                let mut events = Vec::new();
                while let Some(event) = seq.next_element::<Lite>()? {
                    if events.len() >= MAX_EVENTS_PER_CHUNK {
                        return Err(serde::de::Error::custom("trop d'événements"));
                    }
                    events.push(event);
                }
                Ok(Events(events))
            }
        }
        d.deserialize_seq(ListVisitor)
    }
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
    /// Instant (horloge du navigateur) de la dernière action du visiteur, pour le morceau suivant.
    pub last_action_ms: Option<i64>,
    pub hrefs: Vec<String>,
    pub view: Option<(i32, i32)>,
}

/// Lit le tableau d'événements d'un morceau. Erreur si ce n'en est pas un.
pub fn parse_events(events_json: &str) -> Result<Vec<Lite>, &'static str> {
    let Events(events) = serde_json::from_str(events_json).map_err(|_| "événements illisibles")?;
    if events.is_empty() {
        return Err("aucun événement");
    }
    Ok(events)
}

/// Résume un morceau. `previous_action` est la dernière action connue de l'enregistrement :
/// le temps actif se compte aussi à cheval sur deux envois.
pub fn summarise(events: &[Lite], previous_action: Option<i64>) -> Result<ChunkStats, &'static str> {
    let mut stats = ChunkStats { events: events.len() as i32, ..Default::default() };
    let (mut min, mut max) = (f64::MAX, f64::MIN);
    let mut last_action: Option<f64> = previous_action.map(|ms| ms as f64);
    for event in events {
        if event.kind > 6 || !event.timestamp.is_finite() || event.timestamp < 0.0 || event.timestamp > 1e14 {
            return Err("événement invalide");
        }
        min = min.min(event.timestamp);
        max = max.max(event.timestamp);
        if event.kind == EVENT_META {
            // Sans adresse, impossible de vérifier que la page appartient au site : refusé.
            let data = event.data.as_ref().ok_or("page sans adresse")?;
            stats.hrefs.push(data.href.0.clone().ok_or("page sans adresse")?);
            stats.pages += 1;
            if let (Some(w), Some(h)) = (data.width.0, data.height.0)
                && stats.view.is_none()
                && w <= 20_000
                && h <= 20_000
            {
                stats.view = Some((w as i32, h as i32));
            }
        } else if event.kind == EVENT_INCREMENTAL
            && let Some(data) = &event.data
            && let Some(source) = data.source.0
            && ACTIVE_SOURCES.contains(&source)
        {
            if source == SOURCE_MOUSE_INTERACTION && data.kind.0 == Some(MOUSE_CLICK) {
                stats.clicks += 1;
            }
            if let Some(previous) = last_action {
                let gap = event.timestamp - previous;
                if gap > 0.0 && gap <= ACTIVE_GAP_MS {
                    stats.active_ms += gap as i64;
                }
            }
            if last_action.is_none_or(|previous| event.timestamp > previous) {
                last_action = Some(event.timestamp);
            }
        }
    }
    stats.last_action_ms = last_action.map(|ms| ms as i64);
    stats.span_ms = ((max - min).max(0.0) as i64).min(3_600_000);
    Ok(stats)
}

#[cfg(test)]
fn analyse(events_json: &str) -> Result<ChunkStats, &'static str> {
    summarise(&parse_events(events_json)?, None)
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
    // Lisibles du seul compte du serveur : ces fichiers montrent ce que des visiteurs ont vu.
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
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

/// Corps d'un envoi : l'identifiant stable du visiteur et les événements.
#[derive(Deserialize)]
struct Chunk<'a> {
    #[serde(default)]
    d: Option<String>,
    #[serde(borrow)]
    e: &'a RawValue,
}

/// Paramètres d'un envoi. La clé du site et l'identifiant de l'enregistrement sont dans
/// l'adresse : le serveur les vérifie avant de décompresser et de lire quoi que ce soit.
#[derive(Deserialize)]
pub struct WireOptions {
    #[serde(default)]
    k: String,
    #[serde(default)]
    rid: String,
    /// « 1 » : le corps est compressé en gzip.
    #[serde(default)]
    z: Option<String>,
}

/// Place occupée par les enregistrements de chaque projet, relue en base une fois par minute
/// et tenue à jour entre-temps : le plafond se vérifie à chaque envoi sans requête lourde.
#[derive(Default)]
pub struct Usage {
    by_project: Mutex<HashMap<i64, (Instant, i64)>>,
}

impl Usage {
    async fn used(&self, client: &tokio_postgres::Client, project_id: i64) -> Result<i64, AppError> {
        if let Some((at, bytes)) = self.by_project.lock().expect("verrou").get(&project_id)
            && at.elapsed() < Duration::from_secs(60)
        {
            return Ok(*bytes);
        }
        let bytes: i64 = client
            .query_one("SELECT COALESCE(SUM(bytes), 0)::bigint FROM recordings WHERE project_id = $1", &[&project_id])
            .await?
            .get(0);
        self.by_project.lock().expect("verrou").insert(project_id, (Instant::now(), bytes));
        Ok(bytes)
    }

    fn add(&self, project_id: i64, bytes: i64) {
        if let Some(entry) = self.by_project.lock().expect("verrou").get_mut(&project_id) {
            entry.1 += bytes;
        }
    }

    /// À appeler après une suppression : la prochaine vérification relit la base.
    pub fn forget(&self, project_id: i64) {
        self.by_project.lock().expect("verrou").remove(&project_id);
    }
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

/// `POST /r?k=<clé>&rid=<enregistrement>` : un morceau d'enregistrement. 204 s'il est pris ou
/// ignoré, 410 pour dire au navigateur d'arrêter (enregistrements coupés, plafond atteint).
pub async fn collect(
    State(state): State<AppState>,
    Query(options): Query<WireOptions>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let response = match collect_inner(&state, &headers, &options, body).await {
        Ok(Outcome::Stored | Outcome::Ignored) => StatusCode::NO_CONTENT.into_response(),
        Ok(Outcome::Stop) => StatusCode::GONE.into_response(),
        Err(e) => e.into_response(),
    };
    public(response, "no-store")
}

fn blocking_failed(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("traitement d'un morceau : {e}"))
}

async fn collect_inner(
    state: &AppState,
    headers: &HeaderMap,
    options: &WireOptions,
    body: Bytes,
) -> Result<Outcome, AppError> {
    let user_agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let agent = ua::parse(user_agent);
    if agent.bot {
        return Ok(Outcome::Stop);
    }
    // Tout ce qui ne coûte rien d'abord : un envoi pour un site inconnu ou qui n'enregistre
    // pas est écarté sans que son corps soit décompressé ni lu.
    let rid = options.rid.as_str();
    if !valid_id(rid) {
        return Err(AppError::bad("Identifiant d'enregistrement invalide."));
    }
    let Some(project) = state.projects.by_key(&state.pool, &options.k).await? else {
        return Ok(Outcome::Stop);
    };
    if !project.recording.enabled {
        return Ok(Outcome::Stop);
    }

    let _slot = tokio::time::timeout(Duration::from_secs(5), CHUNK_SLOTS.acquire())
        .await
        .map_err(|_| AppError::TooManyRequests)?
        .map_err(blocking_failed)?;
    let compressed = options.z.as_deref() == Some("1");
    let (distinct, events_json, events) = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
        let text = if compressed { gunzip_limited(&body)? } else { body.to_vec() };
        let chunk: Chunk = serde_json::from_slice(&text).map_err(|_| AppError::bad("Corps de requête illisible."))?;
        let events = parse_events(chunk.e.get()).map_err(AppError::bad)?;
        // Hors des chaînes, un saut de ligne n'est qu'une espace : on l'ôte, le fichier compte un morceau par ligne.
        let events_json: String = chunk.e.get().chars().filter(|c| *c != '\n' && *c != '\r').collect();
        Ok((chunk.d, events_json, events))
    })
    .await
    .map_err(blocking_failed)??;

    // Sans identifiant stable, le visiteur n'a pas consenti : rien n'est gardé.
    let Some(distinct) = ingest::stable_id(distinct.as_deref()) else {
        return Ok(Outcome::Stop);
    };
    let visitor = ingest::stable_visitor(project.id, &distinct);
    let now = Utc::now();

    let (existing, used) = {
        let client = state.pool.get().await?;
        let existing = client
            .query_opt(
                "SELECT visitor_id, raw_bytes, started_at, last_action_ms FROM recordings
                  WHERE project_id = $1 AND id = $2",
                &[&project.id, &rid],
            )
            .await?;
        let used = state.usage.used(&client, project.id).await?;
        (existing, used)
        // La connexion est rendue ici : la compression et l'écriture sur disque ne la retiennent pas.
    };
    let previous_action = match &existing {
        Some(row) => {
            let owner: i64 = row.get(0);
            let raw: i64 = row.get(1);
            let started: chrono::DateTime<Utc> = row.get(2);
            if owner != visitor || raw >= MAX_RECORDING_BYTES || (now - started).num_minutes() >= MAX_RECORDING_MINUTES
            {
                return Ok(Outcome::Stop);
            }
            row.get::<_, Option<i64>>(3)
        }
        None => None,
    };
    let stats = summarise(&events, previous_action).map_err(AppError::bad)?;
    drop(events);
    // Un enregistrement commence par l'état complet d'une page : sans lui, rien n'est rejouable.
    if existing.is_none() && stats.pages == 0 {
        return Ok(Outcome::Stop);
    }
    if used >= i64::from(project.recording.max_mb) << 20 {
        return Ok(Outcome::Stop);
    }
    let urls: Vec<referrer::ParsedUrl> = stats.hrefs.iter().filter_map(|h| referrer::parse_url(h)).collect();
    if urls.len() != stats.hrefs.len() || urls.iter().any(|u| !project.accepts_host(&u.host)) {
        return Ok(Outcome::Ignored);
    }

    let path = file_path(&state.config.data_dir, project.id, rid);
    let raw_len = events_json.len() as i64 + 1;
    let stored = tokio::task::spawn_blocking(move || append_chunk(&path, &events_json))
        .await
        .map_err(blocking_failed)?
        .map_err(blocking_failed)? as i64;
    state.usage.add(project.id, stored);

    let entry = urls.first();
    let started = now - chrono::Duration::milliseconds(stats.span_ms);
    let client = state.pool.get().await?;
    client
        .execute(
            "INSERT INTO recordings (project_id, id, visitor_id, started_at, last_at, active_ms, events, chunks, bytes,
                                     raw_bytes, clicks, pages, entry_host, entry_path, device, browser, os, view_w, view_h,
                                     last_action_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
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
                 view_h = COALESCE(recordings.view_h, EXCLUDED.view_h),
                 last_action_ms = GREATEST(recordings.last_action_ms, EXCLUDED.last_action_ms)",
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
                &stats.last_action_ms,
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
        assert_eq!(stats.hrefs, vec!["https://demo.test/a?x=1".to_string()]);
        assert_eq!(stats.last_action_ms, Some(20000));

        // Le temps actif se compte aussi à cheval sur deux envois.
        let next = r#"[{"type":3,"timestamp":22000,"data":{"source":3,"id":1,"x":0,"y":900}}]"#;
        assert_eq!(summarise(&parse_events(next).unwrap(), stats.last_action_ms).unwrap().active_ms, 2000);
        assert_eq!(summarise(&parse_events(next).unwrap(), None).unwrap().active_ms, 0);
        assert_eq!(stats.view, Some((1280, 720)));
    }

    #[test]
    fn rejects_what_is_not_a_recording() {
        for bad in [
            "[]",
            "{}",
            "[1,2]",
            r#"[{"type":9,"timestamp":1}]"#,
            r#"[{"type":3}]"#,
            r#"[{"type":3,"timestamp":-4}]"#,
            r#"[{"type":4,"timestamp":1,"data":{"width":10,"height":10}}]"#,
            r#"[{"type":4,"timestamp":1,"data":{"href":["https://demo.test/"]}}]"#,
        ] {
            assert!(analyse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hostile_payloads_are_read_without_being_loaded() {
        // Des millions de valeurs là où un nombre est attendu : parcourues, jamais gardées.
        let huge = format!(
            r#"[{{"type":3,"timestamp":1,"data":{{"source":[{}0],"type":{{"a":[1,2,3]}}}}}}]"#,
            "0,".repeat(200_000)
        );
        let stats = analyse(&huge).unwrap();
        assert_eq!((stats.events, stats.clicks, stats.active_ms), (1, 0, 0));
        // Un envoi qui dépasse le nombre d'événements permis est refusé pendant la lecture.
        let flood = format!("[{}]", vec![r#"{"type":3,"timestamp":1}"#; MAX_EVENTS_PER_CHUNK + 1].join(","));
        assert!(parse_events(&flood).is_err());
        let long =
            format!(r#"[{{"type":4,"timestamp":1,"data":{{"href":"https://demo.test/{}"}}}}]"#, "a".repeat(10_000));
        assert_eq!(analyse(&long).unwrap().hrefs[0].len(), 2048);
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
