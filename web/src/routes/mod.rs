//! The routes. Add one by writing a handler in the module for its audience
//! (`public`, `session`, `me`, `admin`) and one `.route(...)` line in that
//! module's `router()`; a page needs a file in `web/ui/` and a line in `pages.rs`.

pub mod admin;
pub mod cms;
pub mod me;
pub mod pages;
pub mod public;
pub mod session;
pub mod signup;

use crate::http;
use crate::state::Shared;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::Router;

/// The whole site.
pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .merge(session::api())
        .merge(me::api())
        .merge(admin::api())
        .merge(cms::api())
        .merge(public::api())
        .merge(signup::api())
        .route_layer(middleware::from_fn_with_state(state.clone(), http::guard_changes))
        // Every body here is a few short fields.
        .layer(DefaultBodyLimit::max(8 * 1024));

    Router::new()
        .merge(pages::router())
        .merge(session::pages())
        .nest("/api", api)
        .layer(middleware::from_fn(http::secure_headers))
        .with_state(state)
}
