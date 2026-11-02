// The app's one window: first-run setup, then a sidebar of screens:
// Overview (status, services, the Activity and log panes), Game rules,
// Game client, Backups and Settings. The header's Start, Stop and the
// More menu work from every screen.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const TYPE = "__type__";
const AUTO = "__auto__";
const MAX_LINES = 5000;
const GAME_SERVERS = ["connect", "search", "world", "map"];

let overview = null;
let task = null;
let logGeneration = 0;
let selectedService = null;
let page = null;

// ---------- helpers ----------

function show(el, on) {
  el.hidden = !on;
}

function append(pre, line) {
  const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 40;
  pre.append(line + "\n");
  // Keep the pane from growing without end during a long build.
  while (pre.childNodes.length > MAX_LINES) pre.firstChild.remove();
  if (atBottom) pre.scrollTop = pre.scrollHeight;
}

function message(err) {
  return typeof err === "string" ? err : (err && err.message) || String(err);
}

function configured() {
  return !!(overview && overview.configured);
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    // Not allowed here; copy through a selected field instead.
  }
  const field = document.createElement("textarea");
  field.value = text;
  document.body.append(field);
  field.select();
  const copied = document.execCommand("copy");
  field.remove();
  if (!copied) throw new Error(`Couldn't copy. The address is ${text}`);
}

// The address picker: detect, each address this computer has, 127.0.0.1,
// or one typed in. `typedRow` is what shows for a typed one.
function fillAddresses(select, typed, typedRow, current, auto) {
  select.replaceChildren();
  const add = (value, text) => {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    select.append(o);
  };
  add(AUTO, "Detect automatically (LAN address)");
  const known = new Set();
  for (const a of overview.addresses) {
    known.add(a.ip);
    add(a.ip, a.vpn ? `${a.ip} (${a.vpn})` : `${a.ip} (${a.interface})`);
  }
  if (!known.has("127.0.0.1")) add("127.0.0.1", "127.0.0.1 (this computer only)");
  known.add("127.0.0.1");
  add(TYPE, "Type an address…");
  if (auto) {
    select.value = AUTO;
  } else if (known.has(current)) {
    select.value = current;
  } else {
    select.value = TYPE;
    typed.value = current || "";
  }
  show(typedRow, select.value === TYPE);
  select.onchange = () => show(typedRow, select.value === TYPE);
}

// The picker's choice: null to detect, or an address.
function pickedAddress(select, typed) {
  if (select.value === AUTO) return null;
  if (select.value === TYPE) return typed.value.trim();
  return select.value;
}

// ---------- overview ----------

async function refresh() {
  try {
    overview = await invoke("overview");
  } catch (e) {
    $("docker-error").textContent = message(e);
    show($("docker-error"), true);
    return;
  }
  $("docker-error").textContent = overview.docker_error || "";
  show($("docker-error"), !!overview.docker_error);
  $("docker").textContent = overview.docker_version ? `Docker Compose ${overview.docker_version}` : "";

  if (!overview.configured) {
    showSetup();
  } else {
    showMain();
  }
}

function showSetup() {
  if (!$("setup").hidden) return;
  show($("sidebar"), false);
  show($("status"), false);
  show($("header-actions"), false);
  for (const id of ["overview-page", "game-page", "client-page", "backups-page", "settings-page"]) show($(id), false);
  $("page-title").textContent = "Welcome";
  show($("setup"), true);
  $("setup-dir").value = overview.stack_dir || overview.default_stack_dir;
  fillAddresses($("setup-address"), $("setup-address-typed"), $("setup-address-typed"), "", true);
}

function showMain() {
  const first = page === null;
  show($("setup"), false);
  show($("sidebar"), true);
  show($("status"), true);
  show($("header-actions"), true);
  drawServerInfo();
  if (first) {
    task = overview.task;
    fillServerForm();
    showPage("overview");
    refreshServices();
    refreshClient();
    loadGameSettings();
  }
  updateButtons();
}

// The bits of the overview that come from .env.
function drawServerInfo() {
  $("brand-sub").textContent = overview.server_name || "Nameless";
  const auto = overview.public_ip_auto;
  $("connect-addr").textContent = overview.public_ip || "not detected yet";
  $("connect-addr").title = overview.public_ip || "";
  $("connect-auto").textContent = auto ? "Detected on each start" : "";
  $("stack-dir").textContent = overview.stack_dir || "";
  $("db-where").textContent =
    overview.db_data === "db-data"
      ? "The database is in a Docker volume (db-data)."
      : `The database is in ${overview.db_data} in the server folder.`;
}

function fillServerForm() {
  fillAddresses($("address"), $("address-typed"), $("address-typed-row"), overview.public_ip, overview.public_ip_auto);
  $("name").value = overview.server_name;
}

// ---------- pages ----------

const NAV = {
  overview: ["nav-overview", "overview-page", "Overview"],
  game: ["nav-game", "game-page", "Game rules"],
  client: ["nav-client", "client-page", "Game client"],
  backups: ["nav-backups", "backups-page", "Backups"],
  settings: ["nav-settings", "settings-page", "Settings"],
};

function showPage(next) {
  const moved = next !== page;
  page = next;
  for (const [name, [nav, main]] of Object.entries(NAV)) {
    const on = name === next;
    show($(main), on);
    $(nav).classList.toggle("active", on);
    if (on) $(nav).setAttribute("aria-current", "page");
    else $(nav).removeAttribute("aria-current");
  }
  $("page-title").textContent = NAV[next][2];
  if (!moved) return;
  if (next === "client") {
    refreshClient();
    if (!$("panel-client-cfg").hidden) loadClientSettings();
  }
  if (next === "backups") refreshBackups();
  if (next === "settings") {
    show($("settings-saved"), false);
    show($("settings-error"), false);
    fillServerForm();
  }
  if (next === "game") show($("game-saved"), false);
}

for (const [name, [nav]] of Object.entries(NAV)) $(nav).onclick = () => showPage(name);

// Settings and Game client: one section at a time, per page.
function showPanel(id) {
  const tab = document.querySelector(`.subtab[data-panel="${id}"]`);
  for (const other of tab.parentElement.querySelectorAll(".subtab")) {
    const on = other === tab;
    other.classList.toggle("active", on);
    if (on) other.setAttribute("aria-current", "true");
    else other.removeAttribute("aria-current");
    show($(other.dataset.panel), on);
  }
  if (id === "panel-client-cfg") loadClientSettings();
}
const showSettingsPanel = showPanel;
for (const tab of document.querySelectorAll(".subtab")) tab.onclick = () => showPanel(tab.dataset.panel);

// The sidebar shows only icons below 1000 px (style.css), so each item gets
// its name as a tooltip and accessible name then.
const narrow = window.matchMedia("(max-width: 1000px)");
function labelNav() {
  for (const item of document.querySelectorAll(".nav-item")) {
    const name = item.querySelector("span").textContent;
    if (narrow.matches) {
      item.title = name;
      item.setAttribute("aria-label", name);
    } else {
      item.removeAttribute("title");
      item.removeAttribute("aria-label");
    }
  }
}
labelNav();
narrow.addEventListener("change", labelNav);

$("connect-settings").onclick = (e) => {
  e.preventDefault();
  showSettingsPanel("panel-server");
  showPage("settings");
};
$("ov-client-link").onclick = (e) => {
  e.preventDefault();
  showPage("client");
};
$("copy-addr").onclick = async () => {
  try {
    await copyText($("connect-addr").textContent);
    $("copy-addr").classList.add("done");
    setTimeout(() => $("copy-addr").classList.remove("done"), 1500);
  } catch (e) {
    alert(message(e));
  }
};

// ---------- setup ----------

$("setup-browse").onclick = async () => {
  const picked = await window.__TAURI__.dialog.open({ directory: true, defaultPath: $("setup-dir").value || undefined });
  if (picked) $("setup-dir").value = picked;
};

$("setup-form").onsubmit = async (e) => {
  e.preventDefault();
  const button = $("setup-go");
  button.disabled = true;
  show($("setup-error"), false);
  try {
    await invoke("setup", {
      stackDir: $("setup-dir").value,
      publicIp: pickedAddress($("setup-address"), $("setup-address-typed")),
      serverName: $("setup-name").value,
    });
    await refresh();
  } catch (e) {
    $("setup-error").textContent = message(e);
    show($("setup-error"), true);
  } finally {
    button.disabled = false;
  }
};

// ---------- services ----------

let services = [];

async function refreshServices() {
  if (!configured()) return;
  try {
    services = await invoke("services");
  } catch (e) {
    services = [];
  }
  drawServices();
}

function serviceState(s) {
  // One-shot services are done when they exit cleanly.
  const oneShot = s.service === "db-init" || s.service === "meshes";
  if (s.state === "running") {
    if (s.health === "starting") return ["busy", "starting"];
    if (s.health === "unhealthy") return ["bad", "unhealthy"];
    return ["ok", "running"];
  }
  if (s.state === "exited" && oneShot && s.exit_code === 0) return ["ok", "done"];
  if (s.state === "exited") return ["bad", `stopped (exit ${s.exit_code})`];
  if (s.state === "restarting") return ["bad", "restarting"];
  if (s.state === "absent") return ["", "not started"];
  return ["busy", s.state];
}

const TASK_NAMES = {
  start: "Starting",
  stop: "Stopping",
  restart: "Restarting",
  build: "Building",
  backup: "Backing up",
  restore: "Restoring a backup",
  "client-download": "Downloading the client",
  "client-install": "Installing game data",
  "client-update": "Updating game data",
};

function drawServices() {
  const list = $("services");
  list.replaceChildren();
  for (const s of services) {
    const [cls, text] = serviceState(s);
    const li = document.createElement("li");
    if (s.service === selectedService) li.classList.add("selected");
    const dot = document.createElement("span");
    dot.className = `dot ${cls}`;
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = s.service;
    const state = document.createElement("span");
    state.className = "state";
    state.textContent = s.ports.length ? `${text} · ${s.ports.join(", ")}` : text;
    li.title = state.textContent;
    li.append(dot, name, state);
    li.onclick = () => followLogs(s.service);
    list.append(li);
  }

  const game = services.filter((s) => GAME_SERVERS.includes(s.service));
  const up = game.filter((s) => s.state === "running").length;
  let cls, text, sub;
  if (task) {
    cls = "busy";
    text = TASK_NAMES[task] || task;
    sub = "Progress shows in the Activity log.";
  } else if (game.length && up === game.length) {
    cls = "ok";
    text = "Running";
    sub = "All four game servers are up.";
  } else if (up > 0 || services.some((s) => s.state === "restarting")) {
    cls = "bad";
    text = "Partly running";
    sub = `${up} of ${GAME_SERVERS.length} game servers are up.`;
  } else {
    cls = "off";
    text = "Off";
    sub = "Start the server from the top right.";
  }
  $("status").className = `status ${cls}`;
  $("status-text").textContent = text;
  $("ov-dot").className = `dot ${cls}`;
  $("ov-status").textContent = text;
  $("ov-status-sub").textContent = sub;
  const fine = services.filter((s) => serviceState(s)[0] === "ok").length;
  $("ov-services").textContent = !services.length ? "" : fine === services.length ? "All up" : `${fine} of ${services.length} up`;
  updateButtons();
}

function updateButtons() {
  if (!overview) return;
  const busy = !!task;
  const running = services.some((s) => s.state === "running");
  const game = services.filter((s) => GAME_SERVERS.includes(s.service));
  const allUp = game.length > 0 && game.every((s) => s.state === "running");
  $("start").disabled = busy || allUp || !!overview.docker_error;
  $("restart").disabled = busy || !running;
  $("stop").disabled = busy || !running;
  $("build").disabled = busy || !!overview.docker_error;
  show($("cancel"), busy && task !== "stop");
  for (const id of ["client-download", "client-install", "client-update", "backup-now"]) $(id).disabled = busy;
  for (const b of document.querySelectorAll("#backups button")) b.disabled = busy;
  show($("first-build"), configured() && !overview.image_ready && !overview.docker_error);
}

// ---------- tasks ----------

async function runTask(kind, file) {
  selectTab("activity");
  try {
    await invoke("run_task", { kind, file: file || null });
  } catch (e) {
    append($("activity"), message(e));
    alert(message(e));
  }
}

// The More menu closes once one of its items is picked.
function fromMenu(fn) {
  return () => {
    $("more").open = false;
    fn();
  };
}

$("start").onclick = () => runTask("start");
$("stop").onclick = () => runTask("stop");
$("restart").onclick = fromMenu(() => runTask("restart"));
$("build").onclick = fromMenu(() => runTask("build"));
$("restart-now").onclick = () => {
  show($("restart-bar"), false);
  runTask("restart");
};
$("cancel").onclick = () => invoke("cancel_task");
document.addEventListener("click", (e) => {
  if (!$("more").contains(e.target)) $("more").open = false;
});

listen("task", (event) => {
  const { kind, running, ok } = event.payload;
  task = running ? kind : null;
  if (running) {
    append($("activity"), `── ${kind} ──`);
  } else {
    append($("activity"), ok ? `── ${kind} finished ──` : `── ${kind} failed ──`);
    if (kind === "build" && ok && services.some((s) => s.state === "running")) {
      show($("restart-bar"), true);
    }
    if (["start", "restart"].includes(kind) && ok) show($("restart-bar"), false);
    refreshClient();
    refreshBackups();
    invoke("overview").then((o) => {
      overview = o;
      drawServerInfo();
      updateButtons();
    });
  }
  refreshServices();
});

listen("task-line", (event) => append($("activity"), event.payload));

// ---------- logs ----------

async function followLogs(service) {
  selectedService = service;
  $("log-service").textContent = service;
  $("logs").replaceChildren();
  selectTab("logs");
  drawServices();
  try {
    logGeneration = await invoke("follow_logs", { service });
  } catch (e) {
    append($("logs"), message(e));
  }
}

listen("log-line", (event) => {
  if (event.payload.generation === logGeneration) append($("logs"), event.payload.line);
});

function selectTab(which) {
  const logs = which === "logs";
  $("tab-activity").classList.toggle("active", !logs);
  $("tab-logs").classList.toggle("active", logs);
  show($("activity"), !logs);
  show($("logs"), logs);
}

$("tab-activity").onclick = () => selectTab("activity");
$("tab-logs").onclick = () => (selectedService ? selectTab("logs") : null);

// ---------- game client ----------

let clientStatus = null;

async function refreshClient() {
  if (!configured()) return;
  try {
    clientStatus = await invoke("client_status");
  } catch (e) {
    return;
  }
  const c = clientStatus;
  $("client-state").textContent = !c.program
    ? "vana-client isn't downloaded yet."
    : `vana-client is in ${c.dir}.`;
  if (c.program && c.outdated) $("client-state").textContent += " A newer build of vana-client is available.";
  show($("client-download-box"), !c.program || c.outdated);
  $("client-download-title").textContent = c.program ? "Update vana-client" : "Download vana-client";
  $("client-download").textContent = c.program ? "Update" : "Download";
  // Play works as soon as the client is here: without game data, its first
  // start offers to download it from Square Enix or use files on disk.
  show($("client-play-box"), c.program);
  $("client-play-title").textContent = c.installed ? "First time playing?" : "No game data yet";
  $("client-play-hint").textContent = c.installed
    ? "Create your account on the client's login screen (Create account), then log in. Play points the game at this server."
    : "Press Play: the client's first start downloads the game from Square Enix and installs it, or uses game files already on this computer. Then create your account on its login screen (Create account) and log in.";
  show($("client-install-box"), c.program && !c.installed);
  show($("client-play"), c.program);
  show($("client-update"), c.installed);
  if (c.installer && !$("client-installer").value) $("client-installer").value = c.installer;

  $("ov-client").textContent = !c.program ? "Not downloaded" : c.installed ? "Ready" : "Sets up on Play";
  $("ov-client-link").textContent = c.program ? "Game client" : "Set it up";
  show($("ov-play"), c.program);
  updateButtons();
}

function play() {
  invoke("play").catch((e) => alert(message(e)));
}

$("client-download").onclick = () => runTask("client-download");
$("client-install").onclick = () => runTask("client-install", $("client-installer").value.trim());
$("client-update").onclick = () => runTask("client-update");
$("client-play").onclick = play;
$("ov-play").onclick = play;
$("client-browse").onclick = async () => {
  const picked = await window.__TAURI__.dialog.open({
    filters: [{ name: "FFXI installer", extensions: ["exe"] }],
    defaultPath: clientStatus ? clientStatus.dir : undefined,
  });
  if (picked) $("client-installer").value = picked;
};

// ---------- settings ----------

$("server-form").onsubmit = async (e) => {
  e.preventDefault();
  show($("settings-error"), false);
  show($("settings-saved"), false);
  try {
    const a = await invoke("set_public_ip", { publicIp: pickedAddress($("address"), $("address-typed")) });
    const b = await invoke("set_server_name", { name: $("name").value });
    if (a || b) show($("restart-bar"), true);
    overview = await invoke("overview");
    drawServerInfo();
    fillServerForm();
    show($("settings-saved"), true);
  } catch (e) {
    $("settings-error").textContent = message(e);
    show($("settings-error"), true);
  }
};

$("open-dir").onclick = () => invoke("open_stack_dir").catch((e) => alert(message(e)));

$("theme").value = document.documentElement.dataset.theme || "";
$("theme").onchange = () => window.setTheme($("theme").value);

// ---------- backups ----------

const REASONS = {
  manual: "made by hand",
  daily: "daily, on Stop",
  "before-restore": "before a restore",
};

function size(bytes) {
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(1)} MB`;
  if (bytes >= 1 << 10) return `${Math.round(bytes / (1 << 10))} KB`;
  return `${bytes} bytes`;
}

async function refreshBackups() {
  if (!configured()) return;
  let b;
  try {
    b = await invoke("backups");
  } catch (e) {
    $("backups-state").textContent = message(e);
    return;
  }
  const list = $("backups");
  list.replaceChildren();
  for (const backup of b.backups) {
    const li = document.createElement("li");
    const text = document.createElement("div");
    const when = document.createElement("div");
    when.className = "when";
    when.textContent = backup.created.slice(0, 16).replace("T", " ");
    const what = document.createElement("div");
    what.className = "what muted";
    what.textContent = `${REASONS[backup.reason] || backup.reason} · ${size(backup.size)}`;
    text.append(when, what);
    const restore = document.createElement("button");
    restore.type = "button";
    restore.textContent = "Restore";
    restore.disabled = !!task;
    restore.onclick = () => restoreBackup(backup);
    li.append(text, restore);
    list.append(li);
  }
  show(list, b.backups.length > 0);
  $("backups-state").textContent = b.backups.length ? `In ${b.folder}` : "No backups yet.";
}

async function restoreBackup(backup) {
  const sure = await window.__TAURI__.dialog.ask(
    `Replace the database with the backup from ${backup.created.slice(0, 16).replace("T", " ")}?\n\n` +
      "Everything since then (accounts, characters, items) is replaced. The game servers stop while it loads, " +
      "and the current database is backed up first, so this can be undone by restoring that.",
    { title: "Restore a backup", kind: "warning", okLabel: "Restore", cancelLabel: "Cancel" },
  );
  if (sure) runTask("restore", backup.file);
}

$("backup-now").onclick = () => runTask("backup");
$("open-backups").onclick = () => invoke("open_backups").catch((e) => alert(message(e)));

// ---------- settings forms: Game rules and the client's settings ----------

// One form: `get` and `set` are its load and save commands. Each setting is
// `file.key`, with one card per group and one row per setting.
function settingsForm(prefix, get, set, onSaved) {
  const form = $(`${prefix}-form`);
  const errorEl = $(`${prefix}-error`);
  const savedEl = $(`${prefix}-saved`);
  let data = null;

  const id = (s) => `${s.file}.${s.key}`;
  const value = (input) => (input.type === "checkbox" ? (input.checked ? 1 : 0) : Number(input.value));
  const fail = (text) => {
    errorEl.textContent = text;
    show(errorEl, true);
  };

  function draw(values) {
    form.replaceChildren();
    let group = null;
    let card = null;
    for (const s of data.settings) {
      if (s.group !== group) {
        group = s.group;
        card = document.createElement("section");
        card.className = "card group";
        const h = document.createElement("h2");
        h.textContent = group;
        card.append(h);
        form.append(card);
      }
      const row = document.createElement("div");
      row.className = "setting";
      const text = document.createElement("div");
      text.className = "setting-text";
      const label = document.createElement("label");
      label.htmlFor = `${prefix}-${id(s)}`.replace(".", "-");
      label.textContent = s.label;
      text.append(label);
      if (s.hint) {
        const hint = document.createElement("p");
        hint.className = "hint";
        hint.textContent = s.hint;
        text.append(hint);
      }
      let input;
      if (s.type === "bool" || s.type === "switch") {
        input = document.createElement("input");
        input.type = "checkbox";
        input.className = "switch";
        input.checked = values[id(s)] === 1;
      } else if (s.type === "choice") {
        input = document.createElement("select");
        input.className = "setting-control";
        for (const [v, t] of s.options) {
          const o = document.createElement("option");
          o.value = v;
          o.textContent = t;
          input.append(o);
        }
        input.value = values[id(s)];
      } else {
        input = document.createElement("input");
        input.type = "number";
        input.className = "setting-control";
        input.min = s.min;
        input.max = s.max;
        input.step = s.whole ? 1 : 0.1;
        input.value = values[id(s)];
      }
      input.id = label.htmlFor;
      input.dataset.setting = id(s);
      const mark = () => {
        row.classList.toggle("changed", value(input) !== s.default);
        show(savedEl, false);
      };
      input.addEventListener("input", mark);
      input.addEventListener("change", mark);
      row.classList.toggle("changed", value(input) !== s.default);
      row.append(text, input);
      card.append(row);
    }
  }

  async function load() {
    try {
      data = await invoke(get);
    } catch (e) {
      fail(message(e));
      return null;
    }
    show(errorEl, false);
    draw(data.values);
    return data;
  }

  $(`${prefix}-save`).onclick = async () => {
    if (!data) return;
    show(savedEl, false);
    const values = {};
    for (const s of data.settings) {
      const input = form.querySelector(`[data-setting="${id(s)}"]`);
      if (input.type === "number" && input.value.trim() === "") {
        fail(`${s.label} needs a value.`);
        input.focus();
        return;
      }
      values[id(s)] = value(input);
    }
    try {
      const result = await invoke(set, { values });
      show(errorEl, false);
      await load();
      savedEl.textContent = onSaved(result);
      show(savedEl, true);
    } catch (e) {
      fail(message(e));
    }
  };

  $(`${prefix}-reset`).onclick = () => {
    if (!data) return;
    const defaults = {};
    for (const s of data.settings) defaults[id(s)] = s.default;
    draw(defaults);
    show(savedEl, false);
  };

  return { load };
}

const gameRules = settingsForm("game", "game_settings", "set_game_settings", (restart) => {
  if (restart) show($("restart-bar"), true);
  return restart ? "Saved. Restart the server to use them." : "Saved.";
});
const loadGameSettings = gameRules.load;

const clientSettings = settingsForm("client-cfg", "client_settings", "set_client_settings", () => "Saved. The game uses them the next time it starts.");
async function loadClientSettings() {
  const data = await clientSettings.load();
  if (data) $("client-cfg-path").textContent = data.path;
}

// ---------- tray ----------

listen("tray", (event) => {
  if (!task) runTask(event.payload);
});

// ---------- start ----------

refresh();
setInterval(refreshServices, 3000);
// Docker may be started after the app (Docker Desktop on Windows).
setInterval(() => {
  if (overview && overview.docker_error) refresh();
}, 10000);
