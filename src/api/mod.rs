//! API JSON de l'interface et des intégrations.

mod account;
mod projects;
mod stats;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(account::login))
        .route("/auth/logout", post(account::logout))
        .route("/me", get(account::me))
        .route("/orgs/{org_id}/projects", get(projects::list).post(projects::create))
        .route("/projects/{project_id}", get(projects::get).patch(projects::update))
        .route("/projects/{project_id}/tokens", get(projects::list_tokens).post(projects::create_token))
        .route("/projects/{project_id}/tokens/{token_id}", delete(projects::revoke_token))
        .route("/projects/{project_id}/import", post(projects::import).layer(DefaultBodyLimit::max(8 << 20)))
        .route("/projects/{project_id}/stats/summary", get(stats::summary))
        .route("/projects/{project_id}/stats/breakdown", get(stats::breakdown))
        .route("/projects/{project_id}/stats/realtime", get(stats::realtime))
}
