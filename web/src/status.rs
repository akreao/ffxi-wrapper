//! Which of the stack's services answer, without Docker's socket: Docker's DNS
//! resolves a service's name only while its container runs, and some services
//! also have a port to try.

use async_trait::async_trait;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::net::{lookup_host, TcpStream};
use tokio::time::timeout;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceState {
    pub id: &'static str,
    pub name: &'static str,
    /// `up`, `down` or `off` (the optional AH bot isn't running).
    pub state: &'static str,
}

#[async_trait]
pub trait Probe: Send + Sync {
    /// The state of every service; `db_ok` is the pool's own `SELECT 1`.
    async fn services(&self, db_ok: bool) -> Vec<ServiceState>;
}

/// (id, shown name, port to connect to: None means the name resolving is enough, optional)
const SERVICES: [(&str, &str, Option<u16>, bool); 5] = [
    ("connect", "Login", Some(54231), false),
    ("search", "Search", Some(54002), false),
    ("world", "World", Some(54003), false),
    ("map", "Map", None, false),
    ("ahbot", "AH bot", None, true),
];

const ONE_SECOND: Duration = Duration::from_secs(1);
const CACHE: Duration = Duration::from_secs(15);

pub struct NetProbe {
    cache: Mutex<Option<(Instant, Vec<ServiceState>)>>,
}

impl NetProbe {
    pub fn new() -> NetProbe {
        NetProbe { cache: Mutex::new(None) }
    }
}

impl Default for NetProbe {
    fn default() -> Self {
        NetProbe::new()
    }
}

async fn check(host: &str, port: Option<u16>) -> bool {
    let target = format!("{host}:{}", port.unwrap_or(0));
    let Ok(Ok(mut addrs)) = timeout(ONE_SECOND, lookup_host(target)).await else { return false };
    let Some(addr) = addrs.next() else { return false };
    match port {
        None => true,
        Some(_) => matches!(timeout(ONE_SECOND, TcpStream::connect(addr)).await, Ok(Ok(_))),
    }
}

#[async_trait]
impl Probe for NetProbe {
    async fn services(&self, db_ok: bool) -> Vec<ServiceState> {
        if let Some((at, list)) = self.cache.lock().unwrap().as_ref() {
            if at.elapsed() < CACHE {
                let mut list = list.clone();
                list[0].state = if db_ok { "up" } else { "down" };
                return list;
            }
        }
        let mut out = vec![ServiceState { id: "db", name: "Database", state: if db_ok { "up" } else { "down" } }];
        let results = futures_join(SERVICES.iter().map(|(id, _, port, _)| check(id, *port))).await;
        for ((id, name, _, optional), up) in SERVICES.iter().zip(results) {
            let state = if up { "up" } else if *optional { "off" } else { "down" };
            out.push(ServiceState { id, name, state });
        }
        *self.cache.lock().unwrap() = Some((Instant::now(), out.clone()));
        out
    }
}

/// Runs the futures at once and collects their results in order.
async fn futures_join<F: std::future::Future<Output = bool> + Send + 'static>(
    futs: impl Iterator<Item = F>,
) -> Vec<bool> {
    let handles: Vec<_> = futs.map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.unwrap_or(false));
    }
    out
}

/// A fixed answer, for tests.
pub struct StaticProbe(pub Vec<ServiceState>);

#[async_trait]
impl Probe for StaticProbe {
    async fn services(&self, db_ok: bool) -> Vec<ServiceState> {
        let mut v = vec![ServiceState { id: "db", name: "Database", state: if db_ok { "up" } else { "down" } }];
        v.extend(self.0.clone());
        v
    }
}
