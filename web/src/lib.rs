//! The website for an ffxi-wrapper server: public pages, My account and the
//! admin panel, in one service that runs beside LandSandBoat's.

pub mod auth;
pub mod cache;
pub mod config;
pub mod content;
pub mod game;
pub mod http;
pub mod limits;
pub mod markdown;
pub mod model;
pub mod proxy;
pub mod routes;
pub mod state;
pub mod status;
pub mod store;

use std::sync::{Arc, Mutex};

pub use routes::router;
pub use state::{AppState, Shared};

/// The pieces a running site is made of; tests build one with fakes.
pub struct Parts {
    pub cfg: config::Config,
    pub game: Arc<dyn game::Game>,
    pub store: store::Store,
    pub probe: Arc<dyn status::Probe>,
    pub clock: state::Clock,
    pub dummy_hash: String,
    pub bcrypt_cost: u32,
}

pub fn app_state(p: Parts) -> Shared {
    let proxies = proxy::Proxies::new(&p.cfg.trusted_proxy);
    Arc::new(AppState {
        cfg: p.cfg,
        game: p.game,
        store: p.store,
        limiter: limits::Limiter::new(),
        proxies,
        probe: p.probe,
        news_cache: cache::PublicCache::default(),
        clock: p.clock,
        totp_seen: Mutex::new(Default::default()),
        dummy_hash: p.dummy_hash,
        bcrypt_cost: p.bcrypt_cost,
        bcrypt_slots: tokio::sync::Semaphore::new(4),
        public_cache: Default::default(),
    })
}
