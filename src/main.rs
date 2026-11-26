//! Synalyse — mesure d'audience et analyse produit auto-hébergées.

mod analysis;
mod api;
mod auth;
mod config;
mod db;
mod error;
mod ids;
mod ingest;
mod referrer;
mod ua;
mod web;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use deadpool_postgres::Pool;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::error::AppError;

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub config: Arc<Config>,
    pub writer: mpsc::Sender<ingest::writer::Msg>,
    pub projects: Arc<ingest::ProjectCache>,
    pub salts: Arc<ingest::SaltStore>,
    pub logins: Arc<auth::LoginLimiter>,
}

const USAGE: &str = "Usage :
  synalyse serve                         démarre le serveur (applique les migrations)
  synalyse migrate                       applique les migrations puis s'arrête
  synalyse admin create --email <e-mail> [--org <nom>] [--name <nom>]
                                        crée un compte propriétaire et son organisation ;
                                        le mot de passe vient de SYNALYSE_ADMIN_PASSWORD ou est généré

Configuration (variables d'environnement) :
  SYNALYSE_DATABASE_URL     postgres://utilisateur:motdepasse@hote/base   (obligatoire)
  SYNALYSE_LISTEN           adresse d'écoute, 127.0.0.1:8292 par défaut
  SYNALYSE_BASE_URL         adresse publique du service
  SYNALYSE_TRUSTED_PROXIES  nombre de mandataires de confiance devant le service (0 par défaut)";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["serve"] => serve().await,
        ["migrate"] => migrate_only().await,
        ["admin", "create", rest @ ..] => admin_create(rest).await,
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(message) = result {
        eprintln!("synalyse : {message}");
        std::process::exit(1);
    }
}

async fn open() -> Result<(Config, Pool), String> {
    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url)?;
    let applied = db::migrate(&pool).await.map_err(|e| format!("migrations : {e}"))?;
    if !applied.is_empty() {
        tracing::info!(?applied, "migrations appliquées");
    }
    Ok((config, pool))
}

async fn migrate_only() -> Result<(), String> {
    open().await.map(|_| ())
}

fn option<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    args.windows(2).find(|pair| pair[0] == name).map(|pair| pair[1])
}

async fn admin_create(args: &[&str]) -> Result<(), String> {
    let email = option(args, "--email").ok_or("--email est obligatoire")?.trim().to_lowercase();
    if !email.contains('@') {
        return Err("adresse e-mail invalide".into());
    }
    let org = option(args, "--org").unwrap_or("Mon organisation").to_string();
    let name = option(args, "--name").unwrap_or("").to_string();
    let (password, generated) = match std::env::var("SYNALYSE_ADMIN_PASSWORD") {
        Ok(p) if p.chars().count() >= 12 => (p, false),
        Ok(_) => return Err("SYNALYSE_ADMIN_PASSWORD doit compter au moins 12 caractères".into()),
        Err(_) => (ids::hex(&ids::random_bytes::<12>()), true),
    };
    let (_, pool) = open().await?;
    let hash = ids::hash_password(&password);
    let mut client = pool.get().await.map_err(|e| e.to_string())?;
    let tx = client.transaction().await.map_err(|e| e.to_string())?;
    let exists = tx
        .query_opt("SELECT 1 FROM users WHERE lower(email) = $1", &[&email])
        .await
        .map_err(|e| e.to_string())?
        .is_some();
    if exists {
        return Err(format!("un compte existe déjà pour {email}"));
    }
    let user_id: i64 = tx
        .query_one(
            "INSERT INTO users (email, name, password_hash) VALUES ($1, $2, $3) RETURNING id",
            &[&email, &name, &hash],
        )
        .await
        .map_err(|e| e.to_string())?
        .get(0);
    let org_id: i64 = tx
        .query_one("INSERT INTO organizations (name) VALUES ($1) RETURNING id", &[&org])
        .await
        .map_err(|e| e.to_string())?
        .get(0);
    tx.execute("INSERT INTO memberships (org_id, user_id, role) VALUES ($1, $2, 'owner')", &[&org_id, &user_id])
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    println!("Compte créé : {email} (organisation n° {org_id}, « {org} »)");
    if generated {
        println!("Mot de passe généré, affiché cette seule fois : {password}");
    }
    Ok(())
}

async fn health(State(state): State<AppState>) -> Result<&'static str, AppError> {
    state.pool.get().await?.query_one("SELECT 1", &[]).await?;
    Ok("ok")
}

async fn tracker() -> Response {
    let mut response = include_str!("../tracker/c.js").into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/javascript; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert("cross-origin-resource-policy", HeaderValue::from_static("cross-origin"));
    response
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, AppError::NotFound).into_response()
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(web::index))
        .route("/assets/{name}", get(web::asset))
        .route("/healthz", get(health))
        .route("/c.js", get(tracker))
        .route("/i", post(ingest::collect).options(ingest::preflight))
        .nest("/api", api::router())
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}

async fn serve() -> Result<(), String> {
    let (config, pool) = open().await?;
    let (writer, inbox) = mpsc::channel(20_000);
    let writer_task = tokio::spawn(ingest::writer::run(pool.clone(), inbox));
    let listen = config.listen;
    let state = AppState {
        pool,
        config: Arc::new(config),
        writer,
        projects: Arc::default(),
        salts: Arc::default(),
        logins: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|e| format!("écoute sur {listen} : {e}"))?;
    tracing::info!(%listen, "synalyse écoute");
    axum::serve(listener, app(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| e.to_string())?;
    // Le routeur est détruit : le canal se ferme et le rédacteur écrit ce qui reste.
    if tokio::time::timeout(Duration::from_secs(10), writer_task).await.is_err() {
        tracing::warn!("arrêt avant la fin de l'écriture des derniers événements");
    }
    Ok(())
}
