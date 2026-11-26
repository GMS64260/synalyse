//! Rédacteur : reçoit les événements par un canal et les écrit par lots, en
//! rattachant chacun à une session de visite.
//!
//! Une session regroupe les événements d'un même visiteur tant que deux événements
//! consécutifs ne sont pas séparés de plus de trente minutes. La recherche passe par
//! la base : plusieurs serveurs peuvent écrire pour le même projet, le cache en
//! mémoire n'est qu'un accélérateur.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::{Pool, Transaction};
use tokio::sync::{mpsc, oneshot};

use super::{IngestEvent, KIND_PAGEVIEW};
use crate::db;
use crate::error::AppError;
use crate::ids;

pub enum Msg {
    Event(Box<IngestEvent>),
    /// Répond une fois que tout ce qui précède dans le canal est écrit.
    Flush(oneshot::Sender<()>),
}

const BATCH_MAX: usize = 500;
const BATCH_WAIT: Duration = Duration::from_millis(150);
const SESSION_GAP_MINUTES: i64 = 30;
const SESSION_MAX_HOURS: i64 = 24;
const CACHE_MAX: usize = 50_000;

#[derive(Debug, Clone)]
struct Session {
    id: i64,
    visitor_id: i64,
    started_at: DateTime<Utc>,
    last_at: DateTime<Utc>,
}

impl Session {
    fn accepts(&self, ts: DateTime<Utc>) -> bool {
        ts >= self.started_at
            && ts - self.last_at <= chrono::Duration::minutes(SESSION_GAP_MINUTES)
            && ts - self.started_at <= chrono::Duration::hours(SESSION_MAX_HOURS)
    }
}

#[derive(Default)]
struct Delta {
    last_at: Option<DateTime<Utc>>,
    pageviews: i32,
    events: i32,
    exit_path: Option<String>,
}

#[derive(Default)]
struct State {
    /// (projet, visiteur) → dernière session connue.
    sessions: HashMap<(i64, i64), Session>,
    /// Mois dont les partitions existent.
    months: HashSet<NaiveDate>,
}

pub async fn run(pool: Pool, mut rx: mpsc::Receiver<Msg>) {
    let mut state = State::default();
    while let Some(first) = rx.recv().await {
        let mut batch = Vec::new();
        let mut flushes = Vec::new();
        let mut flush_now = false;
        let mut take = |msg: Msg, batch: &mut Vec<IngestEvent>, flush_now: &mut bool| match msg {
            Msg::Event(event) => batch.push(*event),
            Msg::Flush(reply) => {
                flushes.push(reply);
                *flush_now = true;
            }
        };
        take(first, &mut batch, &mut flush_now);
        let deadline = tokio::time::sleep(BATCH_WAIT);
        tokio::pin!(deadline);
        while !flush_now && batch.len() < BATCH_MAX {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => take(msg, &mut batch, &mut flush_now),
                    None => break,
                },
                _ = &mut deadline => break,
            }
        }
        if !batch.is_empty()
            && let Err(error) = write_batch(&pool, &mut state, &mut batch).await
        {
            // Le cache peut contenir des sessions qui n'ont pas été validées : on repart de la base.
            state.sessions.clear();
            tracing::error!(%error, events = batch.len(), "lot d'événements non écrit");
        }
        for reply in flushes {
            let _ = reply.send(());
        }
    }
}

async fn write_batch(pool: &Pool, state: &mut State, batch: &mut [IngestEvent]) -> Result<(), AppError> {
    batch.sort_by_key(|e| e.ts);
    let mut client = pool.get().await?;

    for event in batch.iter() {
        let month = db::month_of(event.ts.date_naive());
        if !state.months.contains(&month) {
            db::ensure_partitions(&client, month).await?;
            state.months.insert(month);
        }
    }
    if state.sessions.len() > CACHE_MAX {
        state.sessions.clear();
    }

    let tx = client.transaction().await?;
    let mut deltas: HashMap<(i64, DateTime<Utc>, i64), Delta> = HashMap::new();
    for event in batch.iter() {
        let session = resolve_session(&tx, state, event).await?;
        let day = event.ts.with_timezone(&event.tz).date_naive();
        let insert = tx
            .prepare_cached(
                "INSERT INTO events (project_id, ts, session_id, visitor_id, day, kind, name, host, path, title, props)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            )
            .await?;
        tx.execute(
            &insert,
            &[
                &event.project_id,
                &event.ts,
                &session.id,
                &session.visitor_id,
                &day,
                &event.kind,
                &event.name,
                &event.host,
                &event.path,
                &event.title,
                &event.props,
            ],
        )
        .await?;

        let delta = deltas.entry((event.project_id, session.started_at, session.id)).or_default();
        delta.last_at = Some(delta.last_at.map_or(event.ts, |t| t.max(event.ts)));
        delta.events += 1;
        if event.kind == KIND_PAGEVIEW {
            delta.pageviews += 1;
            delta.exit_path = Some(event.path.clone());
        }
    }

    let update = tx
        .prepare_cached(
            "UPDATE sessions
                SET last_at = GREATEST(last_at, $4), pageviews = pageviews + $5, events = events + $6,
                    exit_path = COALESCE($7, exit_path)
              WHERE project_id = $1 AND started_at = $2 AND id = $3",
        )
        .await?;
    for ((project_id, started_at, id), delta) in &deltas {
        tx.execute(
            &update,
            &[project_id, started_at, id, &delta.last_at, &delta.pageviews, &delta.events, &delta.exit_path],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn resolve_session(tx: &Transaction<'_>, state: &mut State, event: &IngestEvent) -> Result<Session, AppError> {
    let candidates = [Some(event.visitor), event.visitor_prev];
    for visitor in candidates.into_iter().flatten() {
        let key = (event.project_id, visitor);
        let cached = state.sessions.get(&key).filter(|s| s.accepts(event.ts)).cloned();
        let found = match cached {
            Some(session) => Some(session),
            None => find_session(tx, event.project_id, visitor, event.ts).await?,
        };
        if let Some(mut session) = found {
            session.last_at = session.last_at.max(event.ts);
            state.sessions.insert(key, session.clone());
            if visitor != event.visitor {
                // Session ouverte avec le sel de la veille : la retrouver aussi sous l'empreinte du jour.
                state.sessions.insert((event.project_id, event.visitor), session.clone());
            }
            return Ok(session);
        }
    }

    let session = Session { id: ids::random_id(), visitor_id: event.visitor, started_at: event.ts, last_at: event.ts };
    let day = event.ts.with_timezone(&event.tz).date_naive();
    let insert = tx
        .prepare_cached(
            "INSERT INTO sessions (project_id, id, visitor_id, identified, started_at, last_at, day, entry_host,
                                   entry_path, exit_path, channel, referrer_host, utm_source, utm_medium, utm_campaign,
                                   device, browser, os, lang, screen_w)
             VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $8, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
        )
        .await?;
    let acq = &event.acquisition;
    tx.execute(
        &insert,
        &[
            &event.project_id,
            &session.id,
            &session.visitor_id,
            &event.identified,
            &session.started_at,
            &day,
            &event.host,
            &event.path,
            &acq.channel,
            &acq.referrer_host,
            &acq.utm_source,
            &acq.utm_medium,
            &acq.utm_campaign,
            &event.agent.device,
            &event.agent.browser,
            &event.agent.os,
            &event.lang,
            &event.screen_w,
        ],
    )
    .await?;
    state.sessions.insert((event.project_id, event.visitor), session.clone());
    Ok(session)
}

async fn find_session(
    tx: &Transaction<'_>,
    project_id: i64,
    visitor: i64,
    ts: DateTime<Utc>,
) -> Result<Option<Session>, AppError> {
    let query = tx
        .prepare_cached(
            "SELECT id, visitor_id, started_at, last_at
               FROM sessions
              WHERE project_id = $1 AND visitor_id = $2
                AND started_at >= $3 AND started_at <= $4 AND last_at >= $5
              ORDER BY last_at DESC
              LIMIT 1",
        )
        .await?;
    let oldest_start = ts - chrono::Duration::hours(SESSION_MAX_HOURS);
    let oldest_activity = ts - chrono::Duration::minutes(SESSION_GAP_MINUTES);
    let row = tx.query_opt(&query, &[&project_id, &visitor, &oldest_start, &ts, &oldest_activity]).await?;
    Ok(row.map(|row| Session { id: row.get(0), visitor_id: row.get(1), started_at: row.get(2), last_at: row.get(3) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_window() {
        let start: DateTime<Utc> = "2026-11-02T10:00:00Z".parse().unwrap();
        let s = Session { id: 1, visitor_id: 1, started_at: start, last_at: start + chrono::Duration::minutes(10) };
        assert!(s.accepts(start + chrono::Duration::minutes(40)), "30 minutes après la dernière activité");
        assert!(!s.accepts(start + chrono::Duration::minutes(41)));
        assert!(!s.accepts(start - chrono::Duration::seconds(1)), "événement antérieur à la session");
        let long = Session { last_at: start + chrono::Duration::hours(24), ..s };
        assert!(!long.accepts(start + chrono::Duration::hours(24) + chrono::Duration::minutes(5)), "durée maximale");
    }
}
