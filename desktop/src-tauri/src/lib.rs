mod backup;
mod client;
mod clientcfg;
mod config;
mod envfile;
mod host;
mod net;
mod settings;
mod stack;
mod task;
mod tray;

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use envfile::EnvFile;
use task::{Ctx, Running};

struct App {
    /// Where the app keeps config.json.
    config_dir: PathBuf,
    /// The app's data folder, the default home of the server's files.
    data_dir: PathBuf,
    /// The stack files bundled with this app version.
    bundled: PathBuf,
    /// The folder with the server's files, once setup has picked it.
    dir: Mutex<Option<PathBuf>>,
    /// The running start, stop or build, if any. Only one at a time.
    task: task::Slot,
    logs: Mutex<Option<Child>>,
    /// Bumped on every log switch so the UI can drop lines from the old stream.
    logs_generation: AtomicU64,
}

impl App {
    fn dir(&self) -> Result<PathBuf, String> {
        self.dir.lock().unwrap().clone().ok_or_else(|| "Finish setup first.".to_owned())
    }

    fn env(&self) -> Option<EnvFile> {
        EnvFile::load(&self.dir().ok()?.join(".env")).ok()
    }

    /// Changes `.env` with no other change to it in between.
    fn edit_env<T>(&self, change: impl FnOnce(&mut EnvFile) -> Result<T, String>) -> Result<T, String> {
        envfile::edit(&self.dir()?.join(".env"), change)
    }
}

/// Runs `work` off the main thread, which would otherwise stop drawing the
/// window while Docker answers.
async fn off_main<T: Send + 'static>(
    handle: AppHandle,
    work: impl FnOnce(&AppHandle, &App) -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || work(&handle, &handle.state::<App>()))
        .await
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct Overview {
    configured: bool,
    stack_dir: Option<String>,
    default_stack_dir: String,
    docker_version: Option<String>,
    docker_error: Option<String>,
    public_ip: String,
    public_ip_auto: bool,
    addresses: Vec<net::Address>,
    server_name: String,
    /// "a folder" or "a Docker volume", for the Settings screen.
    db_data: String,
    /// The server image is here; without it the next start downloads or builds it.
    image_ready: bool,
    task: Option<String>,
}

#[tauri::command]
async fn overview(handle: AppHandle) -> Result<Overview, String> {
    off_main(handle, |_, app| {
        let dir = app.dir.lock().unwrap().clone();
        let env = app.env();
        let get = |key: &str| env.as_ref().and_then(|e| e.get(key)).unwrap_or_default();
        let (docker_version, docker_error) = match stack::check_docker() {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        let image_ready = docker_error.is_none() && app.dir().is_ok_and(|dir| stack::image_ready(&dir));
        Overview {
            configured: env.is_some(),
            stack_dir: dir.map(|d| d.display().to_string()),
            default_stack_dir: config::default_stack_dir(&app.data_dir).display().to_string(),
            docker_version,
            docker_error,
            public_ip: get("PUBLIC_IP"),
            public_ip_auto: get("PUBLIC_IP_AUTO") != "no",
            addresses: net::lan_addresses(),
            server_name: get("SERVER_NAME"),
            db_data: get("DB_DATA"),
            image_ready,
            task: app.task.lock().unwrap().as_ref().map(|r| r.kind.clone()),
        }
    })
    .await
}

/// First run: puts the server's files in `stack_dir` and writes its `.env`.
/// A folder that already has a `.env` (an earlier install, or a clone of
/// the repository set up by hand) keeps it.
#[tauri::command]
async fn setup(
    handle: AppHandle,
    stack_dir: String,
    public_ip: Option<String>,
    server_name: String,
) -> Result<(), String> {
    off_main(handle, move |_, app| {
        if app.task.lock().unwrap().is_some() {
            return Err("Wait for the current task to finish.".to_owned());
        }
        let dir = PathBuf::from(stack_dir.trim());
        if !dir.is_absolute() {
            return Err("Pick a full folder path, such as /run/media/SSD/FFXI.".into());
        }
        let server_name = server_name.trim().to_owned();
        envfile::check_server_name(&server_name)?;
        if let Some(ip) = &public_ip {
            net::check_address(ip)?;
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
        stack::install(&app.bundled, &dir).map_err(|e| format!("Couldn't copy the server's files: {e}"))?;
        let env_path = dir.join(".env");
        if !env_path.exists() {
            // New passwords can't open a database made with older ones.
            if stack::db_location(&dir) == "db-data" && stack::volume_exists("db-data") {
                return Err(format!(
                    "Docker already has this app's database from an earlier setup (the volume {}_db-data), \
                     and its passwords are in that setup's .env. Pick the folder that setup used, or delete \
                     the old database with `docker volume rm {}_db-data` to start fresh.",
                    stack::PROJECT,
                    stack::PROJECT
                ));
            }
            write_env(&dir, &env_path, public_ip, &server_name)?;
        }
        config::save(&app.config_dir, &config::Config { stack_dir: Some(dir.clone()) })?;
        *app.dir.lock().unwrap() = Some(dir);
        Ok(())
    })
    .await?
}

fn write_env(dir: &Path, path: &Path, public_ip: Option<String>, server_name: &str) -> Result<(), String> {
    let mut env = EnvFile::parse(stack::ENV_EXAMPLE).at(path);
    env.set("DB_PASSWORD", &envfile::random_password(24)?);
    env.set("DB_ROOT_PASSWORD", &envfile::random_password(24)?);
    env.set("DB_DATA", stack::db_location(dir));
    env.set("SERVER_NAME", server_name);
    // Empty follows the client this app version pins, so updating the app
    // moves the client along.
    env.set("CLIENT_COMMIT", "");
    apply_public_ip(&mut env, public_ip);
    env.save().map_err(|e| e.to_string())
}

/// Sets the address players are sent to: a pinned one, or with `None`,
/// this computer's LAN address now (re-detected on every start).
fn apply_public_ip(env: &mut EnvFile, public_ip: Option<String>) {
    match public_ip {
        Some(ip) => {
            env.set("PUBLIC_IP", ip.trim());
            env.set("PUBLIC_IP_AUTO", "no");
        }
        None => {
            env.set("PUBLIC_IP", &net::best_lan_ip().unwrap_or_else(|| "127.0.0.1".into()));
            env.set("PUBLIC_IP_AUTO", "yes");
        }
    }
}

/// Saves the address players are sent to. Returns whether the running
/// server needs a restart to use it.
#[tauri::command]
async fn set_public_ip(handle: AppHandle, public_ip: Option<String>) -> Result<bool, String> {
    off_main(handle, move |_, app| {
        if let Some(ip) = &public_ip {
            net::check_address(ip)?;
        }
        let changed = app.edit_env(|env| {
            let before = env.get("PUBLIC_IP");
            apply_public_ip(env, public_ip);
            Ok(before != env.get("PUBLIC_IP"))
        })?;
        Ok(changed && running(app))
    })
    .await?
}

/// Saves the server name. Returns whether the running server needs a restart.
#[tauri::command]
async fn set_server_name(handle: AppHandle, name: String) -> Result<bool, String> {
    off_main(handle, move |_, app| {
        let name = name.trim().to_owned();
        envfile::check_server_name(&name)?;
        let changed = app.edit_env(|env| {
            let before = env.get("SERVER_NAME");
            env.set("SERVER_NAME", &name);
            Ok(before.as_deref() != Some(name.as_str()))
        })?;
        Ok(changed && running(app))
    })
    .await?
}

#[derive(Serialize)]
struct GameSettings {
    settings: &'static [settings::Setting],
    values: settings::Values,
}

#[tauri::command]
async fn game_settings(handle: AppHandle) -> Result<GameSettings, String> {
    off_main(handle, |_, app| Ok(GameSettings { settings: settings::SETTINGS, values: settings::load(&app.dir()?) })).await?
}

/// Saves the game settings. Returns whether the running server needs a restart.
#[tauri::command]
async fn set_game_settings(handle: AppHandle, values: settings::Values) -> Result<bool, String> {
    off_main(handle, move |_, app| {
        let changed = settings::save(&app.dir()?, &values)?;
        Ok(changed && running(app))
    })
    .await?
}

#[derive(Serialize)]
struct ClientSettings {
    /// The config file they're in.
    path: String,
    settings: &'static [settings::Setting],
    values: clientcfg::Values,
}

/// The game client's settings, from its `config.json`.
#[tauri::command]
async fn client_settings(handle: AppHandle) -> Result<ClientSettings, String> {
    off_main(handle, |_, app| {
        let path = client::config_path(&app.dir()?);
        Ok(ClientSettings { path: path.display().to_string(), settings: clientcfg::SETTINGS, values: clientcfg::load(&path) })
    })
    .await?
}

/// Saves the game client's settings; the game uses them the next time it starts.
#[tauri::command]
async fn set_client_settings(handle: AppHandle, values: clientcfg::Values) -> Result<(), String> {
    off_main(handle, move |_, app| clientcfg::save(&client::config_path(&app.dir()?), &values)).await?
}

fn running(app: &App) -> bool {
    app.dir()
        .and_then(|dir| stack::status(&dir))
        .is_ok_and(|s| s.iter().any(|s| s.service == "map" && s.state == "running"))
}

#[tauri::command]
async fn services(handle: AppHandle) -> Result<Vec<stack::ServiceStatus>, String> {
    off_main(handle, |_, app| stack::status(&app.dir()?)).await?
}

#[derive(Serialize, Clone)]
struct TaskEvent {
    kind: String,
    running: bool,
    ok: bool,
}

/// One step of a task, run in order until one fails.
enum Step {
    /// Re-detects this computer's address, unless one is pinned.
    Address,
    /// Checks that nothing else holds the ports the stack publishes.
    Ports,
    /// Runs a command (`docker compose`, curl or vana-client), streaming its output.
    Run(std::process::Command),
    /// Points the client at this server.
    ClientConfig,
    /// Downloads the vana-client built from this commit.
    Download(std::process::Command, String),
    /// Puts a downloaded vana-client, built from this commit, in place.
    FinishDownload(String),
    /// Backs up the database.
    Backup,
    /// Backs up a running database when there's no backup from the last day.
    DailyBackup,
    /// Replaces the database with this backup.
    Restore(String),
}

/// Runs `start`, `restart` (the LSB servers, to apply settings), `stop`,
/// `build`, or for the game client `client-download`, `client-install`
/// (from the installer `file`) or `client-update`, streaming its output as
/// `task-line` events.
#[tauri::command]
async fn run_task(handle: AppHandle, kind: String, file: Option<String>) -> Result<(), String> {
    off_main(handle, move |handle, app| start_task(handle, app, kind, file)).await?
}

fn start_task(handle: &AppHandle, app: &App, kind: String, file: Option<String>) -> Result<(), String> {
    let dir = app.dir()?;
    let mut slot = app.task.lock().unwrap();
    if let Some(running) = slot.as_ref() {
        return Err(format!("Wait for the current {} to finish.", running.kind));
    }
    let mut cmd = stack::compose(&dir);
    cmd.args(["--progress", "plain"]);
    let steps = match kind.as_str() {
        "start" => {
            // Builds the server image first if it isn't here yet.
            cmd.args(["up", "--detach"]);
            vec![Step::Address, Step::Ports, Step::Run(cmd)]
        }
        "restart" => {
            // db-init applies a new address; the servers read their settings
            // when they start. The database keeps running.
            cmd.args(["up", "--detach", "--force-recreate", "db-init"]).args(stack::GAME_SERVERS);
            vec![Step::Address, Step::Run(cmd)]
        }
        "stop" => {
            cmd.arg("down");
            vec![Step::DailyBackup, Step::Run(cmd)]
        }
        "backup" => vec![Step::Backup],
        "restore" => vec![Step::Restore(file.ok_or("Pick a backup to restore.")?)],
        "build" => {
            cmd.args(["build", "db-init"]);
            vec![Step::Run(cmd)]
        }
        "client-download" => {
            let commit = client_commit(app);
            vec![Step::Download(client::download(&dir, &commit)?, commit.clone()), Step::FinishDownload(commit), Step::ClientConfig]
        }
        "client-install" | "client-update" => {
            if !client::program(&dir).is_file() {
                return Err("Download vana-client first.".into());
            }
            let cmd = if kind == "client-install" {
                let installer = PathBuf::from(file.ok_or("Pick the installer (FFXIFullSetup_US.part1.exe).")?);
                if !installer.is_file() {
                    return Err(format!("{} isn't there.", installer.display()));
                }
                client::install(&dir, &installer)
            } else {
                client::update(&dir)
            };
            vec![Step::ClientConfig, Step::Run(cmd)]
        }
        _ => return Err(format!("Unknown task {kind}")),
    };

    *slot = Some(Running { kind: kind.clone(), child: None, cancelled: false });
    drop(slot);
    let _ = handle.emit("task", TaskEvent { kind: kind.clone(), running: true, ok: true });

    let ctx = Ctx { handle: handle.clone(), dir, slot: app.task.clone() };
    let handle = handle.clone();
    std::thread::spawn(move || {
        let result = steps.into_iter().try_for_each(|step| {
            ctx.check_cancelled()?;
            match step {
                Step::Address => envfile::edit(&ctx.dir.join(".env"), |env| {
                    if env.get("PUBLIC_IP_AUTO").as_deref() != Some("no") {
                        apply_public_ip(env, None);
                    }
                    ctx.line(format!("Players are sent to {}.", env.get("PUBLIC_IP").unwrap_or_default()));
                    Ok(())
                }),
                Step::Ports => stack::check_ports(&ctx.dir),
                Step::Run(cmd) => ctx.run(cmd),
                Step::ClientConfig => client::write_config(&ctx.dir, CLIENT_SERVER),
                // curl's exit code 22 is an HTTP error, such as 404.
                Step::Download(cmd, commit) => ctx.run(cmd).map_err(|e| {
                    if e.contains("exit code 22") {
                        format!(
                            "There's no vana-client build for CLIENT_COMMIT {} yet. The repository's Game client \
                             workflow publishes one once that commit is pinned on main; check that it ran.",
                            &commit[..12]
                        )
                    } else {
                        e
                    }
                }),
                Step::FinishDownload(commit) => client::finish_download(&ctx.dir, &commit),
                Step::Backup => backup::create(&ctx, backup::Reason::Manual, None),
                Step::DailyBackup => {
                    backup::daily(&ctx);
                    Ok(())
                }
                Step::Restore(file) => backup::restore(&ctx, &file),
            }
        });
        if let Err(e) = &result {
            ctx.line(e.clone());
        }
        ctx.slot.lock().unwrap().take();
        let _ = handle.emit("task", TaskEvent { kind, running: false, ok: result.is_ok() });
    });
    Ok(())
}

/// The address the app's own client logs in to: the server on this
/// computer. The map server's address (`PUBLIC_IP`) comes from the server
/// after login, and is one of this computer's own addresses too.
const CLIENT_SERVER: &str = "127.0.0.1";

/// The fork commit the client is built from: `.env`'s, or this app
/// version's when `.env` has none (a setup from before the client).
fn client_commit(app: &App) -> String {
    app.env()
        .and_then(|e| e.get("CLIENT_COMMIT"))
        .filter(|c| !c.is_empty())
        .or_else(|| EnvFile::parse(stack::ENV_EXAMPLE).get("CLIENT_COMMIT"))
        .unwrap_or_default()
}

#[tauri::command]
async fn client_status(handle: AppHandle) -> Result<client::Status, String> {
    off_main(handle, |_, app| Ok(client::status(&app.dir()?, &client_commit(app)))).await?
}

/// Opens the game, pointed at this server.
#[tauri::command]
async fn play(handle: AppHandle) -> Result<(), String> {
    off_main(handle, |_, app| play_now(app)).await?
}

fn play_now(app: &App) -> Result<(), String> {
    let dir = app.dir()?;
    // Without game data, the client's own first start sets it up.
    if !client::status(&dir, &client_commit(app)).program {
        return Err("Download vana-client first, on the Game client screen.".into());
    }
    client::write_config(&dir, CLIENT_SERVER)?;
    client::play(&dir)
}

/// Play from the tray: a problem brings the window up to say so.
fn play_from_tray(handle: &AppHandle) {
    let handle = handle.clone();
    std::thread::spawn(move || {
        if let Err(e) = play_now(&handle.state::<App>()) {
            tray::show_main(&handle);
            let _ = handle.emit("task-line", format!("Couldn't start the game: {e}"));
        }
    });
}

#[tauri::command]
fn cancel_task(app: State<App>) {
    if let Some(running) = app.task.lock().unwrap().as_mut() {
        running.cancel();
    }
}

#[derive(Serialize, Clone)]
struct LogLine {
    generation: u64,
    line: String,
}

/// Follows one service's logs as `log-line` events, replacing any earlier
/// stream. Returns the stream's generation. `None` just stops streaming.
#[tauri::command]
fn follow_logs(handle: AppHandle, app: State<App>, service: Option<String>) -> Result<u64, String> {
    let generation = app.logs_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let mut slot = app.logs.lock().unwrap();
    if let Some(mut old) = slot.take() {
        let _ = old.kill();
        let _ = old.wait();
    }
    let Some(service) = service else {
        return Ok(generation);
    };
    let mut cmd = stack::compose(&app.dir()?);
    cmd.args(["logs", "--follow", "--no-log-prefix", "--tail", "500"]).arg(&service);
    let (child, _) = stack::spawn_lines(cmd, move |line| {
        let _ = handle.emit("log-line", LogLine { generation, line });
    })
    .map_err(|e| e.to_string())?;
    *slot = Some(child);
    Ok(generation)
}

#[derive(Serialize)]
struct Backups {
    folder: String,
    backups: Vec<backup::Backup>,
}

#[tauri::command]
async fn backups(handle: AppHandle) -> Result<Backups, String> {
    off_main(handle, |_, app| {
        let folder = backup::folder(&app.dir()?);
        Ok(Backups { backups: backup::list(&folder), folder: folder.display().to_string() })
    })
    .await?
}

#[tauri::command]
fn open_backups(app: State<App>) -> Result<(), String> {
    let folder = backup::folder(&app.dir()?);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    host::open(folder.as_os_str())
}

#[tauri::command]
fn open_stack_dir(app: State<App>) -> Result<(), String> {
    host::open(app.dir()?.as_os_str())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A second copy would fight the first over the stack; opening the
        // app again brings this window forward instead.
        .plugin(tauri_plugin_single_instance::init(|handle, _, _| tray::show_main(handle)))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let bundled = app.path().resource_dir()?.join("stack");
            // FFXI_WRAPPER_STACK_DIR runs the app on a stack folder as-is,
            // such as a clone of the repository while working on the app.
            let dir = match std::env::var_os("FFXI_WRAPPER_STACK_DIR") {
                Some(dir) => Some(PathBuf::from(dir)),
                None => {
                    let dir = config::load(&config_dir).stack_dir;
                    // Each app version brings its own compose.yaml and scripts.
                    if let Some(dir) = &dir {
                        if let Err(e) = stack::install(&bundled, dir) {
                            eprintln!("Couldn't update the server's files in {}: {e}", dir.display());
                        }
                    }
                    dir
                }
            };
            // Without a tray (a desktop that shows none), closing the window quits.
            if let Err(e) = tray::add(app) {
                eprintln!("Couldn't add the tray icon: {e}");
            }
            app.manage(App {
                config_dir,
                data_dir,
                bundled,
                dir: Mutex::new(dir),
                task: Arc::new(Mutex::new(None)),
                logs: Mutex::new(None),
                logs_generation: AtomicU64::new(0),
            });
            Ok(())
        })
        // Closing the window leaves the app in the tray, when there is one.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.app_handle().tray_by_id(tray::ID).is_some() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            setup,
            set_public_ip,
            set_server_name,
            game_settings,
            set_game_settings,
            client_settings,
            set_client_settings,
            services,
            run_task,
            client_status,
            play,
            cancel_task,
            follow_logs,
            backups,
            open_backups,
            open_stack_dir
        ])
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                let app = handle.state::<App>();
                let follower = app.logs.lock().unwrap().take();
                if let Some(mut child) = follower {
                    let _ = child.kill();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_writes_a_complete_env() {
        let dir = std::env::temp_dir().join(format!("setup-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        write_env(&dir, &path, Some("192.168.1.50".into()), "Home").unwrap();
        let env = EnvFile::load(&path).unwrap();
        let db = env.get("DB_PASSWORD").unwrap();
        assert_eq!(db.len(), 24);
        assert_ne!(Some(db), env.get("DB_ROOT_PASSWORD"));
        assert_eq!(env.get("PUBLIC_IP").as_deref(), Some("192.168.1.50"));
        assert_eq!(env.get("PUBLIC_IP_AUTO").as_deref(), Some("no"));
        assert_eq!(env.get("SERVER_NAME").as_deref(), Some("Home"));
        assert_eq!(env.get("DB_DATA").as_deref(), Some(stack::db_location(&dir)));
        // Everything else comes from .env.example.
        assert_eq!(env.get("DATA_DIR").as_deref(), Some("./data"));
        assert!(env.get("LSB_COMMIT").is_some_and(|c| c.len() == 40));
        assert_eq!(env.get("CLIENT_COMMIT").as_deref(), Some(""));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
