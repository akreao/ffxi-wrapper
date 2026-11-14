//! `web serve` runs the site; `web healthcheck` asks a running one if it is well
//! (the image has no curl, so compose's healthcheck runs this).

use ffxi_web::{app_state, auth, config::Config, game::MariaDb, router, state::AppState, status::NetProbe, store::Store, Parts};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    match std::env::args().nth(1).as_deref() {
        Some("healthcheck") => std::process::exit(healthcheck().await),
        Some("serve") | None => serve().await,
        Some(other) => {
            eprintln!("usage: web serve | web healthcheck (got {other:?})");
            std::process::exit(2);
        }
    }
}

/// GET /healthz on the loopback address; exit 0 when it answers 200.
async fn healthcheck() -> i32 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port = std::env::var("WEB_LISTEN").ok().and_then(|l| l.rsplit(':').next().and_then(|p| p.parse::<u16>().ok())).unwrap_or(8080);
    let run = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        s.write_all(b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.ok()?;
        let mut out = String::new();
        s.read_to_string(&mut out).await.ok()?;
        out.starts_with("HTTP/1.1 200").then_some(())
    };
    match tokio::time::timeout(Duration::from_secs(2), run).await {
        Ok(Some(())) => 0,
        _ => 1,
    }
}

async fn serve() {
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("web: {e}");
            std::process::exit(1);
        }
    };
    if wrapper_core::owner_link::parse_secret(&cfg.admin_secret).is_err() {
        if cfg.admin_secret.is_empty() {
            tracing::info!("ADMIN_SECRET is empty: owner links are off; game-account sign-in still works");
        } else {
            tracing::warn!("ADMIN_SECRET isn't 64 hex characters: owner links are off");
        }
    }
    if let Err(e) = std::fs::create_dir_all(&cfg.data_dir) {
        eprintln!("web: can't create {}: {e}", cfg.data_dir.display());
        std::process::exit(1);
    }
    let store = match Store::open(&cfg.data_dir).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("web: can't open {}/web.db: {e}", cfg.data_dir.display());
            std::process::exit(1);
        }
    };
    let dummy_hash = tokio::task::spawn_blocking(|| auth::hash_password("not a real password", auth::BCRYPT_COST).expect("bcrypt works"))
        .await
        .expect("hashing finishes");
    let game = Arc::new(MariaDb::connect(&cfg));
    let listen = cfg.listen.clone();
    let state = app_state(Parts {
        cfg,
        game,
        store,
        probe: Arc::new(NetProbe::new()),
        clock: AppState::system_clock(),
        dummy_hash,
        bcrypt_cost: auth::BCRYPT_COST,
    });
    spawn_housekeeping(state.clone());
    spawn_ban_lifter(state.clone());
    spawn_proxy_lookup(state.clone());

    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("web: can't listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("listening on {listen}");
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("the server runs");
}

/// Every ten minutes: drop expired sessions and spent nonces, forget quiet rate-limit keys.
fn spawn_housekeeping(state: ffxi_web::Shared) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(600));
        loop {
            tick.tick().await;
            let now = state.now();
            if let Err(e) = state.store.purge(now).await {
                tracing::warn!("purge failed: {e}");
            }
            state.limiter.sweep(now);
        }
    });
}

/// Every minute: lift bans whose end has passed (LSB never does).
fn spawn_ban_lifter(state: ffxi_web::Shared) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let n = ffxi_web::routes::admin::lift_expired_bans(&state).await;
            if n > 0 {
                tracing::info!("lifted {n} ban(s) that ran out");
            }
        }
    });
}

/// Finds the proxy container's address now and again every 15 seconds, so a recreated proxy
/// (new address) is trusted within moments and a removed one stops being trusted.
fn spawn_proxy_lookup(state: ffxi_web::Shared) {
    if state.cfg.trusted_proxy.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            state.proxies.refresh().await;
        }
    });
}
