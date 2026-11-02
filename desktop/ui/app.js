// The app's one window: first-run setup, then the server's controls,
// services, settings, and the Activity and log panes.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const TYPE = "__type__";
const AUTO = "__auto__";
const MAX_LINES = 5000;

let overview = null;
let task = null;
let logGeneration = 0;
let selectedService = null;

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

// The address picker: detect, each address this computer has, 127.0.0.1,
// or one typed in.
function fillAddresses(select, typed, current, auto) {
  select.replaceChildren();
  const add = (value, text) => {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    select.append(o);
  };
  add(AUTO, "Detect automatically (this computer's LAN address)");
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
  show(typed, select.value === TYPE);
  select.onchange = () => show(typed, select.value === TYPE);
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

  if (!overview.configured) {
    showSetup();
  } else {
    showMain();
  }
}

function showSetup() {
  if (!$("setup").hidden) return;
  show($("main"), false);
  show($("setup"), true);
  $("setup-dir").value = overview.stack_dir || overview.default_stack_dir;
  fillAddresses($("setup-address"), $("setup-address-typed"), "", true);
}

function showMain() {
  const first = $("main").hidden;
  show($("setup"), false);
  show($("main"), true);
  $("title").textContent = overview.server_name || "Server";
  $("stack-dir").textContent = `Server folder: ${overview.stack_dir}`;
  $("db-where").textContent =
    overview.db_data === "db-data"
      ? "Database: in a Docker volume (db-data)."
      : `Database: ${overview.db_data} in the server folder.`;
  if (first) {
    fillAddresses($("address"), $("address-typed"), overview.public_ip, overview.public_ip_auto);
    $("name").value = overview.server_name;
    task = overview.task;
    updateButtons();
    refreshServices();
  }
}

// ---------- setup ----------

$("setup-browse").onclick = async () => {
  const picked = await window.__TAURI__.dialog.open({ directory: true, defaultPath: $("setup-dir").value || undefined });
  if (picked) $("setup-dir").value = picked;
};

$("setup-go").onclick = async () => {
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
  if ($("main").hidden) return;
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
    if (s.health === "starting") return ["warn", "starting"];
    if (s.health === "unhealthy") return ["bad", "unhealthy"];
    return ["ok", "running"];
  }
  if (s.state === "exited" && oneShot && s.exit_code === 0) return ["ok", "done"];
  if (s.state === "exited") return ["bad", `stopped (exit ${s.exit_code})`];
  if (s.state === "restarting") return ["bad", "restarting"];
  if (s.state === "absent") return ["", "not started"];
  return ["warn", s.state];
}

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
    const detail = document.createElement("span");
    detail.className = "detail";
    detail.textContent = s.ports.length ? `${text} · ${s.ports.join(", ")}` : text;
    li.append(dot, name, detail);
    li.onclick = () => followLogs(s.service);
    list.append(li);
  }

  const game = services.filter((s) => ["connect", "search", "world", "map"].includes(s.service));
  const up = game.filter((s) => s.state === "running").length;
  const pill = $("state");
  if (task) {
    pill.textContent = { start: "starting", stop: "stopping", restart: "restarting", build: "building" }[task] || task;
    pill.className = "pill warn";
  } else if (game.length && up === game.length) {
    pill.textContent = "running";
    pill.className = "pill ok";
  } else if (up > 0 || services.some((s) => s.state === "restarting")) {
    pill.textContent = "partly running";
    pill.className = "pill bad";
  } else {
    pill.textContent = "stopped";
    pill.className = "pill";
  }
  updateButtons();
}

function updateButtons() {
  const busy = !!task;
  const running = services.some((s) => s.state === "running");
  const game = services.filter((s) => ["connect", "search", "world", "map"].includes(s.service));
  const allUp = game.length > 0 && game.every((s) => s.state === "running");
  $("start").disabled = busy || allUp || !!overview.docker_error;
  $("restart").disabled = busy || !running;
  $("stop").disabled = busy || !running;
  $("build").disabled = busy || !!overview.docker_error;
  show($("cancel"), busy && task !== "stop");
  show($("first-build"), !overview.image_ready && !overview.docker_error);
}

// ---------- tasks ----------

async function runTask(kind) {
  selectTab("activity");
  try {
    await invoke("run_task", { kind });
  } catch (e) {
    append($("activity"), message(e));
  }
}

$("start").onclick = () => runTask("start");
$("stop").onclick = () => runTask("stop");
$("restart").onclick = () => runTask("restart");
$("restart-now").onclick = () => {
  show($("restart-bar"), false);
  runTask("restart");
};
$("build").onclick = () => runTask("build");
$("cancel").onclick = () => invoke("cancel_task");

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
    invoke("overview").then((o) => {
      overview = o;
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
$("tab-logs").onclick = () => selectTab("logs");

// ---------- settings ----------

$("save-settings").onclick = async () => {
  show($("settings-error"), false);
  try {
    const a = await invoke("set_public_ip", { publicIp: pickedAddress($("address"), $("address-typed")) });
    const b = await invoke("set_server_name", { name: $("name").value });
    if (a || b) show($("restart-bar"), true);
    overview = await invoke("overview");
    showMain();
    $("title").textContent = overview.server_name || "Server";
  } catch (e) {
    $("settings-error").textContent = message(e);
    show($("settings-error"), true);
  }
};

$("open-dir").onclick = () => invoke("open_stack_dir").catch((e) => alert(message(e)));

// ---------- start ----------

refresh();
setInterval(refreshServices, 3000);
// Docker may be started after the app (Docker Desktop on Windows).
setInterval(() => {
  if (overview && overview.docker_error) refresh();
}, 10000);
