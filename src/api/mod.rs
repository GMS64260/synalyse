//! API JSON de l'interface et des intégrations.

mod account;
mod people;
mod product;
mod projects;
mod recordings;
mod stats;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};

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
        .route("/projects/{project_id}/funnels", get(product::list_funnels).post(product::create_funnel))
        .route("/projects/{project_id}/funnels/preview", post(product::preview_funnel))
        .route("/projects/{project_id}/funnels/{funnel_id}", put(product::update_funnel).delete(product::delete_funnel))
        .route("/projects/{project_id}/funnels/{funnel_id}/report", get(product::funnel_report))
        .route("/projects/{project_id}/paths", get(product::paths))
        .route("/projects/{project_id}/retention", get(people::retention))
        .route("/projects/{project_id}/visitors", get(people::list))
        .route("/projects/{project_id}/visitors/{visitor_id}", get(people::get))
        .route("/projects/{project_id}/visitors/{visitor_id}/events", get(people::events))
        .route("/projects/{project_id}/recordings", get(recordings::list))
        .route("/projects/{project_id}/recordings/{recording_id}", get(recordings::get).delete(recordings::delete))
        .route("/projects/{project_id}/recordings/{recording_id}/events", get(recordings::events))
}
