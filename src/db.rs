//! Connexion PostgreSQL, migrations et partitions mensuelles.

use chrono::{Datelike, NaiveDate};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::NoTls;

use crate::error::AppError;

const MIGRATIONS: &[(i32, &str, &str)] = &[
    (1, "init", include_str!("../migrations/0001_init.sql")),
    (2, "funnels", include_str!("../migrations/0002_funnels.sql")),
    (3, "recordings", include_str!("../migrations/0003_recordings.sql")),
];

/// Verrou consultatif : un seul serveur migre à la fois quand plusieurs démarrent ensemble.
const MIGRATION_LOCK: i64 = 0x5359_4E41_4C59_5345;

pub fn connect(database_url: &str) -> Result<Pool, String> {
    let mut config: tokio_postgres::Config =
        database_url.parse().map_err(|e| format!("SYNALYSE_DATABASE_URL invalide : {e}"))?;
    // Les « CREATE … IF NOT EXISTS » des migrations et des partitions n'ont pas à remplir le journal.
    config.options("-c client_min_messages=warning");
    let manager = Manager::from_config(config, NoTls, ManagerConfig { recycling_method: RecyclingMethod::Fast });
    // Délai d'attente d'une connexion : une base saturée donne une erreur, pas un serveur figé.
    Pool::builder(manager)
        .max_size(16)
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .wait_timeout(Some(std::time::Duration::from_secs(5)))
        .build()
        .map_err(|e| e.to_string())
}

pub async fn migrate(pool: &Pool) -> Result<Vec<i32>, AppError> {
    let mut client = pool.get().await?;
    client.execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK]).await?;
    let result = run_migrations(&mut client).await;
    client.execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK]).await?;
    result
}

async fn run_migrations(client: &mut deadpool_postgres::Client) -> Result<Vec<i32>, AppError> {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
        )
        .await?;
    let applied: Vec<i32> =
        client.query("SELECT version FROM schema_migrations", &[]).await?.iter().map(|row| row.get(0)).collect();
    let mut done = Vec::new();
    for (version, name, sql) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        let tx = client.transaction().await?;
        tx.batch_execute(sql).await?;
        tx.execute("INSERT INTO schema_migrations (version, name) VALUES ($1, $2)", &[version, name]).await?;
        tx.commit().await?;
        done.push(*version);
    }
    Ok(done)
}

/// Premier jour du mois d'une date.
pub fn month_of(day: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(day.year(), day.month(), 1).expect("premier du mois")
}

fn next_month(month: NaiveDate) -> NaiveDate {
    if month.month() == 12 {
        NaiveDate::from_ymd_opt(month.year() + 1, 1, 1).expect("janvier")
    } else {
        NaiveDate::from_ymd_opt(month.year(), month.month() + 1, 1).expect("mois suivant")
    }
}

/// Crée, si elles manquent, les partitions `events` et `sessions` du mois donné (bornes en UTC).
pub async fn ensure_partitions(client: &tokio_postgres::Client, month: NaiveDate) -> Result<(), AppError> {
    let month = month_of(month);
    let end = next_month(month);
    for table in ["events", "sessions"] {
        // Les noms et les bornes sont construits à partir de dates, jamais d'une saisie.
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {table}_y{y}m{m:02} PARTITION OF {table}
             FOR VALUES FROM ('{month} 00:00:00+00') TO ('{end} 00:00:00+00')",
            y = month.year(),
            m = month.month(),
        );
        client.batch_execute(&sql).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_boundaries() {
        let d = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        assert_eq!(month_of(d), NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(next_month(month_of(d)), NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
        let f = NaiveDate::from_ymd_opt(2028, 2, 29).unwrap();
        assert_eq!(next_month(month_of(f)), NaiveDate::from_ymd_opt(2028, 3, 1).unwrap());
    }
}
