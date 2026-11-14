//! What every request handler shares.

use crate::config::Config;
use crate::game::Game;
use crate::limits::Limiter;
use crate::model::{PublicPlayer, RankRow};
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
    pub clock: Clock,
    /// The newest two-step code step accepted for each account, so a code works once.
    pub totp_seen: Mutex<HashMap<i64, u64>>,
    /// A real hash to verify against when a login doesn't exist, so timing doesn't tell.
    pub dummy_hash: String,
    /// bcrypt cost for new hashes: 12 like LSB; tests use less.
    pub bcrypt_cost: u32,
    /// At most this many bcrypt checks or hashes run at once; a burst of sign-ins queues.
    pub bcrypt_slots: tokio::sync::Semaphore,
    pub public_cache: PublicCache,
}

/// Public answers kept for a short while, so visitors cannot load the database.
type Cached<T> = (i64, Arc<T>);

#[derive(Default)]
pub struct PublicCache {
    pub online: Mutex<Option<Cached<Vec<PublicPlayer>>>>,
    /// Keyed by [`Board::key`]: at most 1 + 22 + 10 + 1 entries.
    pub boards: Mutex<HashMap<String, Cached<Vec<RankRow>>>>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn now(&self) -> i64 {
        (self.clock)()
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
