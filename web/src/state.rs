//! What every request handler shares.

use crate::config::Config;
use crate::game::Game;
use crate::limits::Limiter;
use crate::status::Probe;
use crate::store::Store;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct AppState {
    pub cfg: Config,
    pub game: Arc<dyn Game>,
    pub store: Store,
    pub limiter: Limiter,
    pub probe: Arc<dyn Probe>,
    /// The public JSON answers, kept 30 seconds and dropped on any edit.
    pub public_cache: crate::cache::PublicCache,
    pub clock: Clock,
    /// The newest two-step code step accepted for each account, so a code works once.
    pub totp_seen: Mutex<HashMap<i64, u64>>,
    /// A real hash to verify against when a login doesn't exist, so timing doesn't tell.
    pub dummy_hash: String,
    /// bcrypt cost for new hashes: 12 like LSB; tests use less.
    pub bcrypt_cost: u32,
    /// At most this many bcrypt checks or hashes run at once; a burst of sign-ins queues.
    pub bcrypt_slots: tokio::sync::Semaphore,
    /// What LSB's `login.ACCOUNT_CREATION` was when last looked up (time, value); `None` = unknown.
    pub account_creation: Mutex<Option<(i64, Option<bool>)>>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Does LSB's login server allow new accounts? Looked up at most once a minute; a folder
    /// that can't be read, or a setting that can't be understood, counts as no.
    pub async fn signup_open(&self) -> bool {
        let now = self.now();
        if let Some((at, v)) = *self.account_creation.lock().unwrap() {
            if (0..60).contains(&(now - at)) {
                return v == Some(true);
            }
        }
        let value = match self.cfg.lsb_settings.clone() {
            Some(dir) => tokio::task::spawn_blocking(move || crate::lsb_settings::account_creation(&dir)).await.ok().flatten(),
            None => None,
        };
        *self.account_creation.lock().unwrap() = Some((now, value));
        value == Some(true)
    }

    pub fn system_clock() -> Clock {
        Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
    }
}
