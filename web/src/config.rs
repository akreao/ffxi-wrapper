//! Settings, all from the environment (compose.yaml sets them).

use std::path::PathBuf;

#[derive(Clone)]
pub struct Config {
    pub db_host: String,
    pub db_port: u16,
    pub db_name: String,
    pub db_user: String,
    pub db_password: String,
    /// 64 hex characters that sign owner links; empty turns owner links off.
    pub admin_secret: String,
    pub server_name: String,
    /// `https://name.example` when a proxy fronts the site (phase 4).
    pub public_url: String,
    /// Names or addresses of the reverse proxy whose `X-Forwarded-For` is believed (`proxy`).
    pub trusted_proxy: String,
    /// Where the site's own SQLite file lives (`/data` in the container).
    pub data_dir: PathBuf,
    pub listen: String,
    /// Debug builds serve `web/ui` from this folder instead of the embedded copy.
    pub ui_dir: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let get = |k: &str| std::env::var(k).unwrap_or_default();
        let or = |k: &str, d: &str| {
            let v = get(k);
            if v.trim().is_empty() {
                d.to_string()
            } else {
                v
            }
        };
        let db_password = get("DB_PASSWORD");
        if db_password.is_empty() {
            return Err("the website's database password is not set: add WEB_DB_PASSWORD to .env (any long random string) and run `docker compose up -d` again".into());
        }
        let db_port = or("DB_PORT", "3306")
            .parse::<u16>()
            .map_err(|_| "DB_PORT is not a port number".to_string())?;
        Ok(Config {
            db_host: or("DB_HOST", "db"),
            db_port,
            db_name: or("DB_NAME", "xidb"),
            db_user: or("DB_USER", "xiadmin"),
            db_password,
            admin_secret: get("ADMIN_SECRET").trim().to_string(),
            server_name: or("SERVER_NAME", "Your server"),
            public_url: get("WEB_PUBLIC_URL").trim().trim_end_matches('/').to_string(),
            trusted_proxy: get("WEB_TRUSTED_PROXY").trim().to_string(),
            data_dir: PathBuf::from(or("DATA_DIR", "/data")),
            listen: or("WEB_LISTEN", "0.0.0.0:8080"),
            ui_dir: std::env::var("WEB_UI_DIR").ok().filter(|s| !s.is_empty()).map(PathBuf::from),
        })
    }

    /// Cookies carry `Secure` when the site is served over HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.public_url.starts_with("https://")
    }

    /// A config for tests: nothing from the environment.
    pub fn for_tests() -> Config {
        Config {
            db_host: "db".into(),
            db_port: 3306,
            db_name: "xidb".into(),
            db_user: "xiadmin".into(),
            db_password: "x".into(),
            admin_secret: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f".into(),
            server_name: "Test Server".into(),
            public_url: String::new(),
            trusted_proxy: String::new(),
            data_dir: PathBuf::from("."),
            listen: "127.0.0.1:0".into(),
            ui_dir: None,
        }
    }
}
