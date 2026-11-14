// The site's page script: asks the JSON API who is here, draws the shell
// (admin sidebar or public top bar), then fills the page named in
// <body data-page>. Everything is built with textContent, never innerHTML on
// data, and no inline styles (the Content-Security-Policy forbids both).
(function () {
  "use strict";

  const body = document.body;
  const page = body.dataset.page;

  // ---- small helpers ----

  const SVG = "http://www.w3.org/2000/svg";

  // h("a", {class: "x", href: "/y"}, "text", node, ...) -> element
  function h(tag, props, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (v === false || v == null) continue;
      if (k === "class") el.className = v;
      else if (k === "text") el.textContent = v;
      else if (k === "width") el.style.width = v; // CSSOM is allowed; an inline style attribute isn't
      else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? "" : v);
    }
    for (const kid of kids.flat()) {
      if (kid == null || kid === false) continue;
      el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
    }
    return el;
  }

  const ICONS = {
    dashboard: '<rect x="3" y="3" width="7" height="9" rx="1.5"/><rect x="14" y="3" width="7" height="5" rx="1.5"/><rect x="14" y="12" width="7" height="9" rx="1.5"/><rect x="3" y="16" width="7" height="5" rx="1.5"/>',
    settings: '<path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/>',
    accounts: '<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4.4 3.6-7 8-7s8 2.6 8 7"/>',
    characters: '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="11" r="2.5"/><path d="M5.5 17c.6-2 2-3 3.5-3s2.9 1 3.5 3M15 9h3M15 13h3"/>',
    bans: '<circle cx="12" cy="12" r="9"/><path d="M5.6 5.6l12.8 12.8"/>',
    items: '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M4 7.5l8 4.5 8-4.5M12 12v9"/>',
    auction: '<path d="M14 4l6 6M11 7l6 6M12.5 5.5l-6 6 3 3 6-6M8 13l-5 5 3 3 5-5M4 21h9"/>',
    audit: '<path d="M8 3h8l4 4v14H4V3z"/><path d="M8 9h8M8 13h8M8 17h5"/>',
    log: '<path d="M9 4h6v3H9z"/><path d="M15 5.5h3v15H6v-15h3"/><path d="M9 12l2 2 4-4"/>',
    news: '<path d="M4 5h13v14H6a2 2 0 0 1-2-2z"/><path d="M17 9h3v8a2 2 0 0 1-2 2"/><path d="M7.5 9h6M7.5 12.5h6M7.5 16h4"/>',
    signup: '<path d="M7 4.5h7.5L18 8v11.5H7z"/><path d="M10 12h5M12.5 9.5v5"/>',
    menu: '<path d="M4 7h16M4 12h16M4 17h16"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
    moon: '<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>',
    back: '<path d="M15 5l-7 7 7 7"/>',
  };
  function icon(name) {
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("aria-hidden", "true");
    svg.innerHTML = ICONS[name]; // fixed strings above, no data
    return svg;
  }
  const logo = () => h("div", { class: "logo" }, (() => {
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("aria-hidden", "true");
    svg.innerHTML = '<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>';
    return svg;
  })());

  const JOBS = ["WAR", "MNK", "WHM", "BLM", "RDM", "THF", "PLD", "DRK", "BST", "BRD", "RNG", "SAM", "NIN", "DRG", "SMN", "BLU", "COR", "PUP", "DNC", "SCH", "GEO", "RUN"];
  const JOB_NAMES = ["Warrior", "Monk", "White Mage", "Black Mage", "Red Mage", "Thief", "Paladin", "Dark Knight", "Beastmaster", "Bard", "Ranger", "Samurai", "Ninja", "Dragoon", "Summoner", "Blue Mage", "Corsair", "Puppetmaster", "Dancer", "Scholar", "Geomancer", "Rune Fencer"];
  const NATIONS = ["San d'Oria", "Bastok", "Windurst"];
  const CONTAINERS = ["Inventory", "Mog safe", "Storage", "Temporary", "Locker", "Satchel", "Sack", "Case", "Wardrobe", "Mog safe 2", "Wardrobe 2", "Wardrobe 3", "Wardrobe 4", "Wardrobe 5", "Wardrobe 6", "Wardrobe 7", "Wardrobe 8", "Recycle bin"];

  const num = (n) => (n == null ? "–" : Number(n).toLocaleString());
  const jobLabel = (id, lvl) => (id >= 1 && id <= 22 ? `${JOBS[id - 1]} ${lvl}` : "–");
  const jobCell = (r) => h("td", { class: "job" }, jobLabel(r.mjob, r.mlvl), r.sjob >= 1 ? h("small", {}, ` / ${jobLabel(r.sjob, r.slvl)}`) : null);
  const date = (s) => (s ? new Date(s * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }) : "–");
  function playtime(sec) {
    const hrs = Math.floor(sec / 3600);
    return `${hrs.toLocaleString()} h`;
  }
  function when(s) {
    const d = new Date(s * 1000);
    const t = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    const days = Math.floor((new Date().setHours(0, 0, 0, 0) - new Date(d).setHours(0, 0, 0, 0)) / 864e5);
    if (days <= 0) return `Today ${t}`;
    if (days === 1) return `Yesterday ${t}`;
    return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${t}`;
  }

  // ---- the API ----

  let me = { role: "visitor" };

  async function api(path, opts) {
    opts = opts || {};
    const init = { method: opts.method || "GET", headers: {}, credentials: "same-origin" };
    if (opts.body !== undefined) {
      init.headers["Content-Type"] = "application/json";
      init.headers["X-CSRF-Token"] = me.csrf || "";
      init.body = JSON.stringify(opts.body);
    }
    const res = await fetch("/api" + path, init);
    let data = null;
    try { data = await res.json(); } catch { /* not JSON */ }
    if (!res.ok) {
      const err = new Error((data && data.error) || "Something went wrong.");
      err.status = res.status;
      err.data = data || {};
      throw err;
    }
    return data;
  }

  // ---- shells ----

  const q = new URLSearchParams(location.search);
  const isAdmin = () => me.role === "owner" || me.role === "moderator";
  const roleLabel = () => ({ owner: "Owner", moderator: "Moderator", player: "Player" }[me.role] || "");

  function themeButton(btn) {
    const dark = () => (document.documentElement.dataset.theme || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light")) === "dark";
    const paint = () => btn.replaceChildren(icon(dark() ? "sun" : "moon"));
    btn.addEventListener("click", () => { window.setTheme(dark() ? "light" : "dark"); paint(); });
    paint();
  }

  async function signOut() {
    try { await api("/auth/logout", { method: "POST", body: {} }); } catch { /* already out */ }
    location.href = "/";
  }

  function userMenu(inAdmin) {
    const name = me.login || "Owner";
    return h("details", { class: "menu user-menu" },
      h("summary", { class: "user-chip", "aria-label": `Signed in as ${name}, ${roleLabel().toLowerCase()}` },
        h("span", { class: "avatar" }, name[0].toUpperCase()),
        h("span", { class: "who" }, h("strong", {}, name), h("span", {}, roleLabel()))),
      h("div", { class: "menu-list" },
        h("div", { class: "menu-who" }, "Signed in as ", h("strong", {}, name), isAdmin() ? `, ${roleLabel().toLowerCase()}` : "", "."),
        me.accid ? h("a", { href: "/me" }, h("strong", {}, "My account"), h("span", {}, "Your characters and password")) : null,
        isAdmin() ? (inAdmin
          ? h("a", { href: "/" }, h("strong", {}, "View the site"), h("span", {}, "As players see it"))
          : h("a", { href: "/admin" }, h("strong", {}, "Admin panel"), h("span", {}, "Server dashboard"))) : null,
        h("a", { href: "#", onclick: (e) => { e.preventDefault(); signOut(); } }, h("strong", {}, "Sign out"))));
  }

  function serverPill(href) {
    const pill = h("a", { class: "status ok server-pill", href, title: me.server_name },
      h("span", { class: "dot" }), h("span", { class: "server-name" }, me.server_name), h("span", { class: "sep" }, "·"), h("span", { id: "online-count" }, "…"));
    return pill;
  }
  function setOnline(n) {
    const el = document.getElementById("online-count");
    if (el) el.textContent = n == null ? "offline" : `${n} online`;
    const pill = el && el.closest(".status");
    if (pill) { pill.classList.toggle("ok", n != null); pill.classList.toggle("bad", n == null); }
  }

  // The admin panel: dark sidebar + title bar. Entries for later phases carry a "Later" tag.
  const ADMIN_NAV = [
    ["Server", [["dashboard", "Dashboard", "/admin"], ["settings", "Settings", null]]],
    ["Players", [["accounts", "Accounts", null], ["characters", "Characters", null], ["bans", "Bans", null]]],
    ["Site", [["news", "News posts", null]]],
    ["Economy", [["items", "Items", null], ["auction", "Auction house", null]]],
    ["Records", [["audit", "Audit logs", null], ["log", "Admin log", null]]],
  ];

  function adminShell() {
    const current = body.dataset.nav || page;
    const nav = h("nav", { id: "sidebar", "aria-label": "Admin screens" },
      h("div", { class: "brand" }, logo(), h("div", {}, h("div", { class: "brand-name" }, "Admin panel"), h("div", { class: "brand-sub" }, me.server_name))),
      h("a", { class: "sidebar-back", href: "/", title: "Back to the site" }, icon("back"), h("span", {}, "Back to the site")),
      ADMIN_NAV.map(([heading, items]) => h("div", { class: "nav-group" },
        h("div", { class: "nav-heading" }, heading),
        items.map(([id, label, href]) => href
          ? h("a", { class: "nav-item" + (current === id ? " active" : ""), href, title: label, "aria-current": current === id ? "page" : false }, icon(id), h("span", {}, label))
          : h("a", { class: "nav-item soon", "aria-disabled": "true", title: `${label}: later` }, icon(id), h("span", {}, label), h("em", {}, "Later"))))),
      h("div", { class: "sidebar-foot" }, "LandSandBoat", h("br"), "Site 0.1.2"));
    const scrim = h("div", { class: "scrim" });
    const toggle = h("button", { class: "icon-button menu-toggle", "aria-label": "Menu", "aria-controls": "sidebar", "aria-expanded": "false" }, icon("menu"));
    const theme = h("button", { class: "icon-button theme-toggle", "aria-label": "Switch theme", title: "Switch between Light and Dark" });
    const header = h("header", {}, toggle, h("h1", {}, body.dataset.title || ""),
      h("div", { class: "actions" }, serverPill("/admin"), theme, userMenu(true)));
    body.prepend(scrim);
    body.prepend(nav);
    document.getElementById("shell").prepend(header);
    const setOpen = (open) => { body.classList.toggle("nav-open", open); toggle.setAttribute("aria-expanded", open); };
    toggle.addEventListener("click", () => setOpen(!body.classList.contains("nav-open")));
    scrim.addEventListener("click", () => setOpen(false));
    if (q.has("menu")) setOpen(true);
    themeButton(theme);
  }

  // The public pages and My account: a top bar.
  function siteShell() {
    const links = [["home", "Home", "/"]];
    if (me.accid) links.push(["me", "My account", "/me"]);
    const active = page === "me-character" ? "me" : page;
    const toggle = h("button", { class: "icon-button menu-toggle", "aria-label": "Menu", "aria-controls": "site-nav", "aria-expanded": "false" }, icon("menu"));
    const theme = h("button", { class: "icon-button theme-toggle", "aria-label": "Switch theme", title: "Switch between Light and Dark" });
    const top = h("header", { class: "topbar" },
      h("a", { class: "site-brand", href: "/" }, logo(), h("span", {}, h("strong", {}, me.server_name), h("small", {}, "Final Fantasy XI · LandSandBoat"))),
      h("nav", { class: "site-nav", id: "site-nav", "aria-label": "Site" },
        links.map(([id, label, href]) => h("a", { href, class: active === id ? "active" : false, "aria-current": active === id ? "page" : false }, label)),
        me.role === "visitor" ? h("a", { class: "phone-only", href: "/signin" }, "Sign in") : null),
      h("div", { class: "actions" },
        serverPill("/"), theme,
        me.role === "visitor" ? h("a", { class: "btn-link primary", href: "/signin" }, "Sign in") : userMenu(false),
        toggle));
    body.prepend(top);
    body.append(h("footer", { class: "site-foot" }, h("span", {}, `${me.server_name} runs on `, h("a", { href: "https://github.com/LandSandBoat/server" }, "LandSandBoat"), "."), h("span", {}, "Not affiliated with Square Enix.")));
    toggle.addEventListener("click", () => { const o = !body.classList.contains("nav-open"); body.classList.toggle("nav-open", o); toggle.setAttribute("aria-expanded", o); });
    themeButton(theme);
  }

  // Tabs: .subtabs[role=tablist] with panels; the URL hash keeps the open one.
  function wireTabs(root) {
    root.querySelectorAll("[role=tablist]").forEach((list) => {
      const tabs = [...list.querySelectorAll("[role=tab]")];
      const show = (t) => tabs.forEach((x) => {
        const on = x === t;
        x.classList.toggle("active", on);
        x.setAttribute("aria-selected", on);
        const p = document.getElementById(x.getAttribute("aria-controls"));
        if (p) p.hidden = !on;
      });
      tabs.forEach((t) => t.addEventListener("click", () => { show(t); history.replaceState(null, "", location.pathname + "#" + t.id); }));
      const fromHash = tabs.find((t) => "#" + t.id === location.hash);
      if (fromHash) show(fromHash);
    });
  }

  // ---- pages ----

  function errorCard(message) {
    return h("div", { class: "update-bar lock" }, h("p", {}, message));
  }

  const pages = {};

  pages.home = async function (main) {
    const info = await api("/public/home").catch(() => ({ up: false, online: null, server_name: me.server_name }));
    setOnline(info.online);
    main.append(h("section", { class: "card hero" },
      h("div", {},
        h("h2", {}, info.server_name || me.server_name),
        h("div", { class: "meta" },
          h("span", { class: "badge " + (info.up ? "ok" : "bad") }, info.up ? "Online" : "Offline"),
          info.online != null ? h("span", { class: "badge info plain" }, `${info.online} ${info.online === 1 ? "player" : "players"} in game`) : null),
        h("p", {}, "A LandSandBoat server. Sign in with the login and password you play with to see your characters."),
        h("div", { class: "cta" },
          me.role === "visitor"
            ? h("a", { class: "btn-link primary", href: "/signin" }, "Sign in")
            : h("a", { class: "btn-link primary", href: me.accid ? "/me" : "/admin" }, me.accid ? "My account" : "Admin panel")))));
  };

  // `next` may only lead to a page on this site. Browsers read "\\" as "/" and drop tabs and
  // newlines, so "/\\evil.example" would otherwise leave the site: let the URL parser decide.
  function safeNext(next) {
    if (!next) return null;
    try {
      const u = new URL(next, location.origin);
      return u.origin === location.origin ? u.pathname + u.search + u.hash : null;
    } catch { return null; }
  }

  pages.signin = async function () {
    document.getElementById("server-name").textContent = me.server_name;
    themeButton(document.querySelector(".theme-toggle"));
    if (me.role !== "visitor") { location.replace(me.accid ? "/me" : "/admin"); return; }
    const form = document.getElementById("form");
    const err = document.getElementById("err");
    const errText = document.getElementById("err-text");
    const otpRow = document.getElementById("otp-row");
    const go = document.getElementById("go");
    function show(message) { errText.textContent = message; err.hidden = !message; }
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      show("");
      go.disabled = true;
      try {
        const out = await api("/auth/login", {
          method: "POST",
          body: { login: form.login.value, password: form.password.value, otp: form.otp.value || undefined },
        });
        location.href = safeNext(q.get("next")) || out.next || "/me";
      } catch (ex) {
        if (ex.data && ex.data.need_otp) { otpRow.hidden = false; form.otp.focus(); }
        let m = ex.message;
        if (ex.data && typeof ex.data.tries_left === "number" && ex.data.tries_left <= 2) {
          m += ` ${ex.data.tries_left} more ${ex.data.tries_left === 1 ? "try" : "tries"} before this address has to wait 15 minutes.`;
        }
        show(m);
        form.password.value = "";
      } finally {
        go.disabled = false;
      }
    });
  };

  pages.me = async function (main) {
    let data;
    try { data = await api("/me/account"); } catch (ex) { main.append(errorCard(ex.message)); return; }
    const { account, characters } = data;
    main.append(h("div", { class: "page-head" },
      h("div", { class: "ident" },
        h("span", { class: "avatar" }, account.login[0].toUpperCase()),
        h("div", {}, h("h2", {}, account.login, " ", h("span", { class: "badge ok" }, "Active")),
          h("div", { class: "sub" }, `Your account since ${date(account.created)} · ${characters.length} ${characters.length === 1 ? "character" : "characters"}`)))));
    main.append(h("div", { class: "char-cards" }, characters.length
      ? characters.map((c) => h("a", { class: "card char-card", href: `/me/characters/${c.charid}` },
        h("div", { class: "top" }, h("span", { class: "avatar" }, c.name[0].toUpperCase()),
          h("div", {}, h("strong", {}, c.name), h("span", { class: "row2" }, `${NATIONS[c.nation] || "Unknown"} · `,
            c.online ? h("span", { class: "badge ok" }, "In game") : `last played ${date(c.last_logout)}`))),
        h("dl", {},
          h("div", {}, h("dt", {}, "Job"), h("dd", {}, jobLabel(c.mjob, c.mlvl) + (c.sjob >= 1 ? ` / ${jobLabel(c.sjob, c.slvl)}` : ""))),
          h("div", {}, h("dt", {}, "Zone"), h("dd", {}, c.zone || "–")),
          h("div", {}, h("dt", {}, "Play time"), h("dd", {}, playtime(c.playtime))))))
      : h("p", { class: "muted" }, "No characters yet. Make one in the game.")));

    const pw = h("form", { class: "card pad form-grid", id: "pw", novalidate: true },
      h("h2", {}, "Change password"),
      h("label", { for: "pw-old" }, "Current password"), h("input", { id: "pw-old", type: "password", autocomplete: "current-password", maxlength: "32" }),
      h("label", { for: "pw-new" }, "New password"), h("input", { id: "pw-new", type: "password", autocomplete: "new-password", maxlength: "32" }),
      h("p", { class: "hint" }, "At least 8 characters. You use it in the game and here."),
      h("label", { for: "pw-new2" }, "New password again"), h("input", { id: "pw-new2", type: "password", autocomplete: "new-password", maxlength: "32" }),
      h("p", { class: "error", id: "pw-msg", hidden: true, role: "alert" }),
      h("div", { class: "form-actions" }, h("button", { class: "primary", type: "submit" }, "Change password")));
    pw.addEventListener("submit", async (e) => {
      e.preventDefault();
      const msg = pw.querySelector("#pw-msg");
      const say = (t, ok) => { msg.hidden = !t; msg.textContent = t; msg.classList.toggle("ok", !!ok); };
      const [cur, nw, nw2] = ["pw-old", "pw-new", "pw-new2"].map((i) => pw.querySelector("#" + i).value);
      if (nw !== nw2) return say("The two new passwords don't match.");
      try {
        await api("/me/password", { method: "POST", body: { current: cur, new: nw } });
        pw.reset();
        say("Your password is changed. Other devices have been signed out.", true);
      } catch (ex) { say(ex.message); }
    });
    main.append(h("div", { class: "side-grid" }, pw,
      h("section", { class: "card pad" }, h("h2", {}, "Account"),
        h("dl", { class: "kv" },
          h("dt", {}, "Login"), h("dd", {}, account.login),
          h("dt", {}, "Created"), h("dd", {}, date(account.created)),
          h("dt", {}, "Two-step sign-in"), h("dd", {}, account.has_totp ? "On" : "Off")))));
  };

  pages["me-character"] = async function (main) {
    const id = location.pathname.split("/").pop();
    let c, inv;
    try {
      [c, inv] = await Promise.all([api(`/me/characters/${encodeURIComponent(id)}`), api(`/me/characters/${encodeURIComponent(id)}/inventory`)]);
    } catch (ex) { main.append(errorCard(ex.status === 404 ? "That character isn't on your account." : ex.message)); return; }
    document.title = `${c.name} · My account`;
    const tab = (tid, label, on) => h("button", { class: "subtab" + (on ? " active" : ""), role: "tab", id: "tab-" + tid, "aria-controls": "p-" + tid, "aria-selected": on ? "true" : "false" }, label);
    const panel = (tid, on, ...kids) => h("div", { id: "p-" + tid, class: "tabpanel", role: "tabpanel", "aria-labelledby": "tab-" + tid, hidden: !on }, kids);
    const stat = (label, value, sub) => h("section", { class: "card stat" }, h("div", { class: "stat-label" }, label), h("div", { class: "stat-value" }, value), h("div", { class: "stat-sub" }, sub || ""));
    const unlocked = c.jobs.filter((j) => j.level > 0);

    main.append(h("div", { class: "page-head" },
      h("div", { class: "ident" }, h("span", { class: "avatar" }, c.name[0].toUpperCase()),
        h("div", {}, h("h2", {}, c.name, " ", c.online ? h("span", { class: "badge ok" }, "In game") : null),
          h("div", { class: "sub" }, h("a", { href: "/me" }, "My account"), ` · ${NATIONS[c.nation] || "Unknown"}`)))));
    main.append(h("nav", { class: "subtabs", role: "tablist", "aria-label": "Character sections" }, tab("overview", "Overview", true), tab("jobs", "Jobs"), tab("inventory", "Inventory")));

    main.append(panel("overview", true,
      h("div", { class: "stats four" },
        stat("Job", [jobLabel(c.mjob, c.mlvl), c.sjob >= 1 ? h("small", {}, ` / ${jobLabel(c.sjob, c.slvl)}`) : null], `Level cap ${c.cap}`),
        stat("Zone", c.zone || "–", c.online ? "In game now" : `Last played ${date(c.last_logout)}`),
        stat("Gil", num(c.gil), "In inventory"),
        stat("Play time", playtime(c.playtime), "")),
      h("p", { class: "hint" }, "What the server has saved for this character. While you're in game it updates when you zone or log out.")));

    main.append(panel("jobs", false,
      h("section", { class: "card pad" },
        h("div", { class: "card-head" }, h("h2", {}, "Jobs"), h("span", { class: "legend" }, h("span", {}, `Level cap ${c.cap}`), h("span", {}, `${unlocked.length} of 22 unlocked`))),
        h("div", { class: "jobs" }, c.jobs.map((j, i) => {
          const locked = j.level <= 0;
          const role = c.mjob === i + 1 ? "main" : c.sjob === i + 1 ? "sub" : "";
          return h("div", { class: "jobtile" + (role ? " " + role : "") + (locked ? " locked" : "") },
            role ? h("span", { class: "tag" }, role === "main" ? "Main" : "Sub") : null,
            h("abbr", { title: JOB_NAMES[i] }, JOBS[i]), h("b", {}, locked ? "Locked" : String(j.level)),
            h("div", { class: "lv" }, locked ? null : h("span", { width: Math.min(100, Math.round((j.level / 99) * 100)) + "%" })),
            h("span", { class: "name" }, JOB_NAMES[i]));
        })))));

    // Inventory: one container at a time.
    const byContainer = new Map();
    for (const it of inv.items) { if (!byContainer.has(it.location)) byContainer.set(it.location, []); byContainer.get(it.location).push(it); }
    const keys = [...new Set([0, ...byContainer.keys()])].sort((a, b) => a - b);
    const tbody = h("tbody");
    const seg = h("div", { class: "segmented", "aria-label": "Container" });
    function showContainer(k) {
      seg.querySelectorAll("button").forEach((b) => { const on = Number(b.dataset.k) === k; b.classList.toggle("on", on); b.setAttribute("aria-pressed", on); });
      const items = byContainer.get(k) || [];
      tbody.replaceChildren(...(items.length
        ? items.map((it) => h("tr", {}, h("td", { class: "num id" }, it.slot), h("td", {}, it.name || "Unknown item", " ", h("span", { class: "id" }, it.item_id)), h("td", { class: "num" }, num(it.quantity))))
        : [h("tr", { class: "empty-row" }, h("td", { colspan: "3" }, "Nothing here."))]));
    }
    keys.forEach((k) => seg.append(h("button", { type: "button", "data-k": k, onclick: () => showContainer(k) }, `${CONTAINERS[k] || "Container " + k} ${(byContainer.get(k) || []).length}`)));
    main.append(panel("inventory", false,
      h("section", { class: "card table-card" },
        h("div", { class: "toolbar" }, seg),
        h("div", { class: "table-wrap" }, h("table", { class: "data" },
          h("thead", {}, h("tr", {}, h("th", { class: "num" }, "Slot"), h("th", {}, "Item"), h("th", { class: "num" }, "Qty"))), tbody)))));
    showContainer(0);
    wireTabs(main);
  };

  const ACTION_BADGE = { ban: "bad", unban: "ok", gm_level: "gm", password_reset: "busy", password_change: "info" };

  pages.dashboard = async function (main) {
    const counts = h("div", { class: "stats four" });
    const onlineBody = h("tbody");
    const services = h("ul", { class: "services" });
    const servicesNote = h("span", { class: "muted" });
    const zones = h("ul", { class: "bars" });
    const actions = h("tbody");
    const addrHead = h("th", { class: "hide-phone", hidden: true }, "Address");
    main.append(counts,
      h("div", { class: "grid-2" },
        h("section", { class: "card table-card" },
          h("div", { class: "card-head" }, h("h2", {}, "Who is online"), h("span", { class: "muted" }, "From the game database, every 10 s")),
          h("div", { class: "table-wrap" }, h("table", { class: "data" },
            h("thead", {}, h("tr", {}, h("th", {}, "Character"), h("th", {}, "Job"), h("th", {}, "Zone"), h("th", { class: "hide-phone" }, "Account"), addrHead)), onlineBody))),
        h("div", { class: "stack" },
          h("section", { class: "card pad" }, h("div", { class: "card-head" }, h("h2", {}, "Services"), servicesNote), services),
          h("section", { class: "card pad" }, h("div", { class: "card-head" }, h("h2", {}, "Busiest zones")), zones))),
      h("section", { class: "card table-card" },
        h("div", { class: "card-head" }, h("h2", {}, "Recent admin actions")),
        h("div", { class: "table-wrap" }, h("table", { class: "data" },
          h("thead", {}, h("tr", {}, h("th", {}, "When"), h("th", {}, "Who"), h("th", {}, "Action"), h("th", {}, "Target"), h("th", { class: "hide-phone" }, "Reason"))), actions))));

    const stat = (label, value, sub, dot) => h("section", { class: "card stat" }, h("div", { class: "stat-label" }, label),
      h("div", { class: "stat-value" }, dot ? h("span", { class: "dot ok" }) : null, value), h("div", { class: "stat-sub" }, sub || ""));

    function draw(d) {
      const c = d.counts;
      counts.replaceChildren(
        stat("Online now", [String(c.online), " ", h("small", {}, c.online === 1 ? "character" : "characters")], "", true),
        stat("Accounts", num(c.accounts)), stat("Characters", num(c.characters)),
        stat("Banned", [String(c.banned), " ", h("small", {}, "accounts")]));
      setOnline(c.online);
      const showAddr = d.online.some((r) => r.address);
      addrHead.hidden = !showAddr;
      onlineBody.replaceChildren(...(d.online.length ? d.online.map((r) => h("tr", {},
        h("td", {}, r.name, r.gm > 0 ? [" ", h("span", { class: "badge gm" }, `GM ${r.gm}`)] : null, r.connecting ? [" ", h("span", { class: "badge busy plain" }, "Connecting")] : null),
        jobCell(r), h("td", { class: "zone" }, r.zone || "–"), h("td", { class: "hide-phone" }, r.login),
        showAddr ? h("td", { class: "hide-phone id" }, r.address || "") : null))
        : [h("tr", { class: "empty-row" }, h("td", { colspan: "5" }, "Nobody is in game."))]));
      services.replaceChildren(...d.services.map((s) => h("li", {}, h("span", { class: "dot " + (s.state === "up" ? "ok" : s.state === "off" ? "off" : "bad") }), s.name, h("span", { class: "state" }, s.state === "up" ? "running" : s.state === "off" ? "off" : "not answering"))));
      const down = d.services.filter((s) => s.state === "down").length;
      servicesNote.textContent = down ? `${down} not answering` : "All running";
      const top = Math.max(1, ...d.zones.map((z) => z.count));
      zones.replaceChildren(...(d.zones.length ? d.zones.map((z) => h("li", {}, z.zone, h("b", {}, String(z.count)), h("div", { class: "bar" }, h("span", { width: Math.round((z.count / top) * 100) + "%" })))) : [h("li", { class: "muted" }, "Nobody is in game.")]));
      actions.replaceChildren(...(d.actions.length ? d.actions.map((a) => h("tr", {},
        h("td", {}, when(a.at)), h("td", {}, a.actor_login || "Owner"),
        h("td", {}, h("span", { class: "badge " + (ACTION_BADGE[a.action] || "info") }, a.action.replace(/_/g, " "))),
        h("td", {}, a.target_name || "–"), h("td", { class: "hide-phone wrap" }, a.reason || "")))
        : [h("tr", { class: "empty-row" }, h("td", { colspan: "5" }, "Nothing yet. Bans, password resets and other changes made here show up in this list."))]));
    }
    async function refresh() {
      try { draw(await api("/admin/dashboard")); } catch (ex) { if (ex.status === 401) location.href = "/signin?next=/admin"; }
    }
    await refresh();
    setInterval(() => { if (!document.hidden) refresh(); }, 10000);
  };

  // ---- start ----

  (async function start() {
    try { me = await api("/me"); } catch { me = { role: "visitor", server_name: "Server" }; }
    if (page === "signin") return pages.signin();
    if (body.classList.contains("app")) {
      if (!isAdmin()) { location.replace("/signin?next=" + encodeURIComponent(location.pathname)); return; }
      adminShell();
    } else {
      siteShell();
      api("/public/home").then((i) => setOnline(i.online)).catch(() => setOnline(null));
    }
    const main = document.getElementById("main");
    if (pages[page] && main) {
      try { await pages[page](main); } catch (ex) { main.append(errorCard(ex.message)); }
    }
  })();
})();
