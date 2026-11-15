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
    for (const kid of kids.flat(Infinity)) {
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
    content: '<path d="M5 4h14v16H5z"/><path d="M8 8h8M8 12h8M8 16h4"/>',
    signup: '<path d="M7 4.5h7.5L18 8v11.5H7z"/><path d="M10 12h5M12.5 9.5v5"/>',
    menu: '<path d="M4 7h16M4 12h16M4 17h16"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
    moon: '<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>',
    search: '<circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/>',
    lock: '<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
    key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M17 6l3 3M15 8l2 2"/>',
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

  // The home answer is cached by the server; the shell and the page share one request.
  let homeP = null;
  const homeInfo = () => (homeP = homeP || api("/public/home"));

  // For HTML the server rendered from Markdown (raw HTML already turned into text there); never for anything else.
  function rendered(el, html) {
    el.innerHTML = html;
    return el;
  }
  const prose = (html, tag) => rendered(h(tag || "div", { class: "prose" }), html);

  async function copyText(text, btn) {
    try { await navigator.clipboard.writeText(text); } catch {
      const t = h("textarea", { "aria-hidden": "true" });
      t.value = text;
      document.body.append(t);
      t.select();
      try { document.execCommand("copy"); } catch { /* nothing more to try */ }
      t.remove();
    }
    const was = btn.textContent;
    btn.textContent = "Copied";
    setTimeout(() => { btn.textContent = was; }, 1500);
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
    ["Players", [["accounts", "Accounts", "/admin/accounts"], ["characters", "Characters", "/admin/characters"], ["bans", "Bans", "/admin/bans"]]],
    ["Site", [["news", "News posts", "/admin/news"], ["content", "Site content", "/admin/content"], ["invites", "Invite codes", "/admin/invites"]]],
    ["Economy", [["items", "Items", null], ["auction", "Auction house", null]]],
    ["Records", [["audit", "Audit logs", null], ["log", "Admin log", "/admin/log"]]],
  ];

  function adminShell() {
    const current = body.dataset.nav || page;
    const nav = h("nav", { id: "sidebar", "aria-label": "Admin screens" },
      h("div", { class: "brand" }, logo(), h("div", {}, h("div", { class: "brand-name" }, "Admin panel"), h("div", { class: "brand-sub" }, me.server_name))),
      h("a", { class: "sidebar-back", href: "/", title: "Back to the site" }, icon("back"), h("span", {}, "Back to the site")),
      // The Site group is the owner's; moderators don't see it.
      ADMIN_NAV.filter(([heading]) => heading !== "Site" || me.role === "owner").map(([heading, items]) => h("div", { class: "nav-group" },
        h("div", { class: "nav-heading" }, heading),
        items.map(([id, label, href]) => href
          ? h("a", { class: "nav-item" + (current === id ? " active" : ""), href, title: label, "aria-current": current === id ? "page" : false }, icon(id), h("span", {}, label))
          : h("a", { class: "nav-item soon", "aria-disabled": "true", title: `${label}: later` }, icon(id), h("span", {}, label), h("em", {}, "Later"))))),
      h("div", { class: "sidebar-foot" }, "LandSandBoat", h("br"), "Site 0.8.0"));
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
    const links = [["home", "Home", "/"], ["news", "News", "/news"], ["online", "Who's online", "/online"], ["rankings", "Rankings", "/rankings"], ["rules", "Rules", "/rules"]];
    if (me.accid) links.push(["me", "My account", "/me"]);
    const active = page === "me-character" ? "me" : page === "post" ? "news" : page;
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
    // The footer text is the owner's (Site content); until it arrives the page has no footer text.
    const foot = h("footer", { class: "site-foot" });
    body.append(foot);
    homeInfo().then((i) => { if (i.footer_html) foot.append(prose(i.footer_html, "span")); }).catch(() => {});
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

  // One post in a list: title, who and when, a short text, a link to the rest.
  function postCard(p) {
    return h("article", { class: "card post" + (p.pinned ? " pinned" : "") },
      h("div", { class: "post-head" },
        h("h3", {}, h("a", { href: `/news/${p.id}` }, p.title)),
        p.pinned ? h("span", { class: "badge gm plain" }, "Pinned") : null,
        h("span", { class: "byline" }, `${p.author} · ${date(p.published)}`)),
      p.excerpt ? h("p", {}, p.excerpt) : null,
      h("a", { class: "more", href: `/news/${p.id}` }, "Read more"));
  }

  pages.home = async function (main) {
    const info = await homeInfo().catch(() => ({ up: false, online: null, server_name: me.server_name, title: me.server_name, news: [] }));
    setOnline(info.online);
    const connect = info.connect_html || info.connect_address
      ? h("div", { class: "connect" },
        h("h3", {}, "How to connect"),
        info.connect_html ? prose(info.connect_html) : null,
        info.connect_address ? (() => {
          const btn = h("button", { class: "small", type: "button" }, "Copy");
          btn.addEventListener("click", () => copyText(info.connect_address, btn));
          return h("div", { class: "copy-line" }, h("code", {}, info.connect_address), btn);
        })() : null)
      : null;
    main.append(h("section", { class: "card hero" },
      h("div", {},
        h("h2", {}, info.title || info.server_name || me.server_name),
        h("div", { class: "meta" },
          h("span", { class: "badge " + (info.up ? "ok" : "bad") }, info.up ? "Online" : "Offline"),
          info.online != null ? h("span", { class: "badge info plain" }, `${info.online} ${info.online === 1 ? "player" : "players"} in game`) : null),
        info.description_html ? prose(info.description_html) : null,
        h("div", { class: "cta" },
          me.role === "visitor"
            ? [info.signup_open ? h("a", { class: "btn-link primary", href: "/signup" }, "Create account") : null,
              h("a", { class: info.signup_open ? "btn-link" : "btn-link primary", href: "/signin" }, "Sign in")]
            : h("a", { class: "btn-link primary", href: me.accid ? "/me" : "/admin" }, me.accid ? "My account" : "Admin panel"))),
      connect));
    const news = info.news || [];
    const aside = h("aside", { class: "stack" });
    main.append(h("div", { class: "side-grid" },
      h("section", { class: "posts", "aria-label": "News" },
        news.length ? news.map(postCard) : h("section", { class: "card pad" }, h("p", { class: "muted" }, "No news yet.")),
        news.length ? h("div", { class: "pager card" }, h("span", { class: "count" }, "Newer posts first"), h("a", { class: "btn-link", href: "/news" }, "All news")) : null),
      aside));
    aside.append(h("section", { class: "card pad" },
      h("h2", {}, "Server"),
      h("dl", { class: "kv" },
        h("dt", {}, "Status"), h("dd", {}, info.up ? "Online" : "Offline"),
        h("dt", {}, "In game"), h("dd", {}, info.online == null ? "–" : `${info.online} ${info.online === 1 ? "player" : "players"}`),
        h("dt", {}, "Rules"), h("dd", {}, h("a", { href: "/rules" }, "House rules")))));
    onlineNow(aside);
  };

  pages.news = async function (main) {
    const st = { page: Math.max(1, Number(q.get("page")) || 1) };
    const list = h("section", { class: "posts", "aria-label": "News" });
    const pager = h("div", { class: "pager card" });
    main.append(h("h2", {}, "News"), list, pager);
    async function load() {
      const d = await api(`/public/news?page=${st.page}`);
      list.replaceChildren(...(d.items.length ? d.items.map(postCard) : [h("section", { class: "card pad" }, h("p", { class: "muted" }, "No news yet."))]));
      const pagesN = Math.max(1, Math.ceil(d.total / d.per));
      const go = (n) => { st.page = n; history.replaceState(null, "", n > 1 ? `?page=${n}` : location.pathname); load().catch((ex) => list.replaceChildren(errorCard(ex.message))); window.scrollTo(0, 0); };
      pager.replaceChildren(
        h("span", { class: "count" }, d.total ? `Page ${d.page} of ${pagesN}` : ""),
        h("div", { class: "pages", "aria-label": "Pages" },
          h("button", { type: "button", "aria-label": "Newer posts", disabled: d.page <= 1, onclick: () => go(d.page - 1) }, "‹"),
          h("button", { type: "button", class: "on", "aria-current": "page" }, String(d.page)),
          h("button", { type: "button", "aria-label": "Older posts", disabled: d.page >= pagesN, onclick: () => go(d.page + 1) }, "›")));
      pager.hidden = pagesN <= 1;
    }
    await load();
  };

  pages.post = async function (main) {
    let p;
    try { p = await api(`/public/news/${encodeURIComponent(pathId())}`); } catch (ex) {
      main.append(h("a", { class: "back-link", href: "/news" }, "‹ All news"), errorCard(ex.status === 404 ? "That post doesn't exist, or hasn't been published." : ex.message));
      return;
    }
    document.title = `${p.title} · ${me.server_name}`;
    main.append(h("a", { class: "back-link", href: "/news" }, "‹ All news"),
      h("article", { class: "card post full" + (p.pinned ? " pinned" : "") },
        h("div", { class: "post-head" }, h("h3", {}, p.title), p.pinned ? h("span", { class: "badge gm plain" }, "Pinned") : null,
          h("span", { class: "byline" }, `${p.author} · ${date(p.published)}`)),
        prose(p.body_html)));
  };

  pages.rules = async function (main) {
    const d = await api("/public/content/rules");
    main.append(h("h2", {}, "House rules"), h("section", { class: "card pad" }, prose(d.html)));
  };

  // "Online now": the busiest zones, from the same cached list as the online page.
  async function onlineNow(main) {
    let j;
    try { j = await api("/public/online"); } catch { return; }
    const shown = j.players.filter((p) => p.zone);
    main.prepend(h("section", { class: "card pad" },
      h("div", { class: "card-head" }, h("h2", {}, "Online now"), h("a", { class: "muted", href: "/online" }, `${j.count} ${j.count === 1 ? "player" : "players"}`)),
      shown.length ? zoneBars(shown) : h("p", { class: "muted" }, j.count ? "Everyone in game is anonymous." : "Nobody is in game right now.")));
  }

  // ---- who is online, rankings ----

  // Zone names arrive as the database spells them ("Valkurm_Dunes", "Riverne-Site_B01").
  const zoneLabel = (z) => (z ? z.replace(/_/g, " ") : "");
  const NATION_NAME = (n) => NATIONS[n] || "";
  const clock = (s) => new Date(s * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

  // The same list feeds the home page's zone bars and the online page's table.
  function zoneBars(players) {
    const by = new Map();
    for (const p of players) if (p.zone) by.set(p.zone, (by.get(p.zone) || 0) + 1);
    const top = [...by].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 6);
    const max = top.length ? top[0][1] : 1;
    return h("ul", { class: "bars" }, top.map(([zone, n]) => h("li", {},
      h("span", {}, zoneLabel(zone)), h("b", {}, n), h("div", { class: "bar" }, h("span", { width: `${Math.round((n / max) * 100)}%` })))));
  }

  pages.online = async function (main) {
    const j = await api("/public/online");
    setOnline(j.count);
    const rows = j.players.map((p) => h("tr", {},
      h("td", {}, p.name),
      p.anonymous
        ? h("td", { class: "muted", colspan: "2" }, "Anonymous")
        : [jobCell({ mjob: p.mjob, mlvl: p.mlvl, sjob: p.sjob, slvl: p.slvl }), h("td", {}, zoneLabel(p.zone) || "–")],
      h("td", { class: "hide-phone" }, NATION_NAME(p.nation))));
    main.append(h("section", { class: "card table-card" },
      h("div", { class: "card-head" }, h("h2", {}, "Who's online"),
        h("span", { class: "muted" }, `${j.count} ${j.count === 1 ? "player" : "players"} · updated ${clock(j.updated)}`)),
      j.players.length
        ? h("div", { class: "table-wrap" }, h("table", { class: "data" },
          h("thead", {}, h("tr", {}, h("th", {}, "Character"), h("th", {}, "Job"), h("th", {}, "Zone"), h("th", { class: "hide-phone" }, "Nation"))),
          h("tbody", {}, rows)))
        : h("p", { class: "empty" }, "Nobody is in game right now."),
      h("div", { class: "pager" }, h("span", { class: "count" }, "Characters set to /anon show only their name and nation, as in the game's search. Game masters are not listed."))));
  };

  const BOARDS = [["level", "Level"], ["job", "By job"], ["craft", "Crafting"], ["playtime", "Play time"]];
  const CRAFTS = [[48, "Fishing"], [49, "Woodworking"], [50, "Smithing"], [51, "Goldsmithing"], [52, "Clothcraft"], [53, "Leathercraft"], [54, "Bonecraft"], [55, "Alchemy"], [56, "Cooking"], [57, "Synergy"]];
  const ORDINAL = ["1st", "2nd", "3rd"];

  pages.rankings = async function (main) {
    let board = BOARDS.some(([id]) => id === q.get("board")) ? q.get("board") : "level";
    let job = JOBS.map((j) => j.toLowerCase()).includes(q.get("job")) ? q.get("job") : "war";
    let skill = CRAFTS.some(([id]) => String(id) === q.get("skill")) ? q.get("skill") : "49";
    let pageNo = Math.max(1, parseInt(q.get("page"), 10) || 1);
    const card = h("section", { class: "card table-card" });
    main.append(card);

    const go = () => {
      const p = new URLSearchParams({ board });
      if (board === "job") p.set("job", job);
      if (board === "craft") p.set("skill", skill);
      if (pageNo > 1) p.set("page", pageNo);
      history.replaceState(null, "", "/rankings?" + p);
      draw();
    };
    const select = (label, value, options, onchange) => {
      const el = h("select", { "aria-label": label, onchange: (e) => onchange(e.target.value) },
        options.map(([v, text]) => h("option", { value: v, selected: String(v) === String(value) }, text)));
      return el;
    };

    async function draw() {
      const path = `/public/rankings/${board}` + (board === "job" ? `?job=${job}&page=${pageNo}` : board === "craft" ? `?skill=${skill}&page=${pageNo}` : `?page=${pageNo}`);
      let j;
      try { j = await api(path); } catch (ex) { card.replaceChildren(errorCard(ex.message)); return; }
      const valueHead = { level: "Level", job: "Level", craft: "Skill", playtime: "Hours" }[board];
      const detail = (r) => (r.job ? `${JOB_NAMES[r.job - 1]}` : "");
      const showJob = board === "level";
      const tabs = h("div", { class: "segmented", role: "group", "aria-label": "Ranking" },
        BOARDS.map(([id, label]) => h("button", { type: "button", class: id === board ? "on" : false, "aria-pressed": id === board, onclick: () => { board = id; pageNo = 1; go(); } }, label)));
      const extra = board === "job"
        ? select("Job", job, JOB_NAMES.map((n, i) => [JOBS[i].toLowerCase(), n]), (v) => { job = v; pageNo = 1; go(); })
        : board === "craft" ? select("Craft", skill, CRAFTS, (v) => { skill = v; pageNo = 1; go(); }) : null;
      const first = pageNo === 1 ? j.rows.slice(0, 3) : [];
      const rest = pageNo === 1 ? j.rows.slice(3) : j.rows;
      const podium = first.length ? h("div", { class: "podium" }, first.map((r, i) =>
        h("div", { class: i === 0 ? "first" : false }, h("span", { class: "place" }, ORDINAL[i]), h("strong", {}, r.name),
          h("span", {}, board === "level" ? jobLabel(r.job, r.value) : `${r.value}${board === "playtime" ? " h" : ""}`)))) : null;
      const pager = j.pages > 1 ? h("div", { class: "pages" },
        h("button", { type: "button", disabled: pageNo <= 1, "aria-label": "Previous page", onclick: () => { pageNo--; go(); } }, "‹"),
        h("button", { type: "button", class: "on" }, pageNo),
        h("button", { type: "button", disabled: pageNo >= j.pages, "aria-label": "Next page", onclick: () => { pageNo++; go(); } }, "›")) : null;
      card.replaceChildren(
        h("div", { class: "toolbar" }, tabs, h("span", { class: "spacer" }), extra),
        podium,
        j.rows.length
          ? h("div", { class: "table-wrap" }, h("table", { class: "data" },
            h("thead", {}, h("tr", {}, h("th", { class: "rank" }, "#"), h("th", {}, "Character"), showJob ? h("th", {}, "Highest job") : null, h("th", { class: "num" }, valueHead), h("th", { class: "hide-phone" }, "Nation"))),
            h("tbody", {}, rest.map((r) => h("tr", {}, h("td", { class: "rank" }, r.rank), h("td", {}, r.name), showJob ? h("td", {}, detail(r)) : null,
              h("td", { class: "num" }, num(r.value)), h("td", { class: "hide-phone" }, NATION_NAME(r.nation)))))))
          : h("p", { class: "empty" }, "Nobody is ranked here yet."),
        h("div", { class: "pager" },
          h("span", { class: "count" }, `${j.total ? `Top ${j.total}. ` : ""}Game masters, banned accounts and characters set to /anon are left out of the level and job boards. Updated ${clock(j.updated)}.`),
          pager));
    }
    draw();
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
    homeInfo().then((i) => {
      const foot = document.querySelector(".signin-foot");
      if (i.signup_open && foot) foot.prepend(h("a", { href: "/signup" }, "Create an account"), " · ");
    }).catch(() => {});
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

  pages.signup = async function () {
    document.getElementById("server-name").textContent = me.server_name;
    themeButton(document.querySelector(".theme-toggle"));
    if (me.role !== "visitor") { location.replace(me.accid ? "/me" : "/admin"); return; }
    const form = document.getElementById("form");
    const err = document.getElementById("err");
    const errText = document.getElementById("err-text");
    const go = document.getElementById("go");
    const show = (message) => { errText.textContent = message; err.hidden = !message; };
    let info = { open: false, invite_required: true };
    try { info = await api("/public/signup"); } catch (ex) { show(ex.message); }
    document.getElementById("invite-row").hidden = !info.invite_required;
    if (!info.open) {
      document.getElementById("fields").hidden = true;
      document.getElementById("closed").hidden = false;
      return;
    }
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      show("");
      if (form.password.value !== form.password2.value) return show("The two passwords don't match.");
      go.disabled = true;
      try {
        const out = await api("/public/signup", {
          method: "POST",
          body: { login: form.login.value, password: form.password.value, invite: form.invite.value },
        });
        location.href = out.next || "/me?welcome=1";
      } catch (ex) {
        show(ex.message);
        if (ex.status !== 409) form.password.value = form.password2.value = "";
        if (ex.data && ex.data.closed) location.reload();
      } finally {
        go.disabled = false;
      }
    });
  };

  pages.me = async function (main) {
    let data;
    try { data = await api("/me/account"); } catch (ex) { main.append(errorCard(ex.message)); return; }
    const { account, characters } = data;
    if (q.has("welcome")) {
      const info = await homeInfo().catch(() => ({}));
      main.append(h("section", { class: "card pad connect" },
        h("h2", {}, "Welcome, ", account.login),
        h("p", {}, "Your account is ready. To play, set up the game client and connect to this server:"),
        info.connect_html ? prose(info.connect_html) : null,
        info.connect_address ? (() => {
          const btn = h("button", { class: "small", type: "button" }, "Copy");
          btn.addEventListener("click", () => copyText(info.connect_address, btn));
          return h("div", { class: "copy-line" }, h("code", {}, info.connect_address), btn);
        })() : null,
        h("p", { class: "hint" }, "Sign in to the game with your login and password, then make a character there.")));
    }
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

  // The job grid and the container browser, shared by My account and the admin panel.
  function jobsCard(c) {
    const unlocked = c.jobs.filter((j) => j.level > 0);
    return h("section", { class: "card pad" },
      h("div", { class: "card-head" }, h("h2", {}, "Jobs"), h("span", { class: "legend" }, h("span", {}, `Level cap ${c.cap}`), h("span", {}, `${unlocked.length} of 22 unlocked`))),
      h("div", { class: "jobs" }, c.jobs.map((j, i) => {
        const locked = j.level <= 0;
        const role = c.mjob === i + 1 ? "main" : c.sjob === i + 1 ? "sub" : "";
        return h("div", { class: "jobtile" + (role ? " " + role : "") + (locked ? " locked" : "") },
          role ? h("span", { class: "tag" }, role === "main" ? "Main" : "Sub") : null,
          h("abbr", { title: JOB_NAMES[i] }, JOBS[i]), h("b", {}, locked ? "Locked" : String(j.level)),
          h("div", { class: "lv" }, locked ? null : h("span", { width: Math.min(100, Math.round((j.level / 99) * 100)) + "%" })),
          h("span", { class: "name" }, JOB_NAMES[i]));
      })));
  }

  function inventoryCard(inv) {
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
    showContainer(0);
    return h("section", { class: "card table-card" },
      h("div", { class: "toolbar" }, seg),
      h("div", { class: "table-wrap" }, h("table", { class: "data" },
        h("thead", {}, h("tr", {}, h("th", { class: "num" }, "Slot"), h("th", {}, "Item"), h("th", { class: "num" }, "Qty"))), tbody)));
  }

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

    main.append(panel("jobs", false, jobsCard(c)));
    main.append(panel("inventory", false, inventoryCard(inv)));
    wireTabs(main);
  };

  const ACTION_BADGE = { ban: "bad", unban: "ok", gm_level: "gm", site_role: "info", password_reset: "busy", password_change: "info" };

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

  // ---- the admin panel: players, bans and the log ----

  const ACTION_LABEL = { ban: "Ban", unban: "Unban", gm_level: "GM level", site_role: "Website role", password_reset: "Password reset", totp_reset: "Two-step reset", password_change: "Password change", sign_in: "Sign-in", sign_in_failed: "Sign-in failed", news_create: "News draft", news_edit: "News edit", news_publish: "News published", news_unpublish: "News unpublished", news_pin: "News pinned", news_unpin: "News unpinned", news_delete: "News deleted", content_edit: "Content edit", content_reset: "Content reset" };
  const ACTION_TONE = { ban: "bad", unban: "ok", gm_level: "gm", site_role: "info", password_reset: "busy", totp_reset: "busy", password_change: "info", sign_in: "off", sign_in_failed: "busy", news_publish: "ok", news_delete: "bad", news_unpublish: "busy" };

  const pathId = () => location.pathname.split("/").pop();
  const fmtDT = (s) => (s ? new Date(s * 1000).toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" }) : "–");
  const nowSec = () => Math.floor(Date.now() / 1000);
  function inWords(sec) {
    if (sec < 3600) return `${Math.max(1, Math.round(sec / 60))} min`;
    if (sec < 86400) return `${Math.round(sec / 3600)} h`;
    const d = Math.round(sec / 86400);
    return `${d} ${d === 1 ? "day" : "days"}`;
  }
  const parseDetail = (a) => { try { return (a.detail && JSON.parse(a.detail)) || {}; } catch { return {}; } };
  const actorName = (a) => a.actor_login || (a.actor_kind === "owner" ? "Owner" : a.actor_kind === "system" ? "The site" : "Unknown");
  const gmBadge = (n) => (n > 0 ? h("span", { class: "badge gm", title: "In-game GM level" }, `GM ${n}`) : null);
  // Website roles belong to accounts and are set by an owner; GM levels belong to characters
  // and only control in-game commands.
  const ROLE_NAME = { player: "Player", moderator: "Moderator", owner: "Owner" };
  const roleBadge = (r) => (r === "owner" || r === "moderator" ? h("span", { class: "badge info", title: "Website role" }, ROLE_NAME[r]) : null);

  // What an action-log row says, for the History lists and the log's Details column.
  function describe(a) {
    const who = actorName(a), d = parseDetail(a);
    switch (a.action) {
      case "ban": return { tone: "bad", what: `Banned ${d.until ? "until " + fmtDT(d.until) : "until someone lifts it"} by ${who}` };
      case "unban": return a.actor_kind === "system" ? { tone: "ok", what: "Ban ended" } : { tone: "ok", what: `Unbanned by ${who}` };
      case "gm_level": return { tone: "gm", what: `GM level ${d.before} → ${d.after} by ${who}` };
      case "site_role": return { tone: "info", what: `Website role ${ROLE_NAME[d.before] || d.before} → ${ROLE_NAME[d.after] || d.after} by ${who}` };
      case "password_reset": return { tone: "info", what: `Password reset by ${who}` };
      case "totp_reset": return { tone: "info", what: `Two-step sign-in removed by ${who}` };
      case "news_create": case "news_edit": case "news_publish": case "news_unpublish": case "news_pin": case "news_unpin": case "news_delete":
        return { tone: "info", what: `${ACTION_LABEL[a.action]}: "${a.target_name || ""}" by ${who}` };
      case "content_edit": case "content_reset":
        return { tone: "info", what: `${ACTION_LABEL[a.action]}: ${a.target_name || ""} (${d.before_size ?? "–"} → ${d.after_size ?? "–"} characters) by ${who}` };
      default: return { tone: "", what: `${ACTION_LABEL[a.action] || a.action} by ${who}` };
    }
  }
  function timeline(rows) {
    if (!rows.length) return h("p", { class: "muted" }, "Nothing has been done here from the admin panel yet.");
    return h("ul", { class: "timeline" }, rows.map((a) => {
      const d = describe(a);
      return h("li", { class: d.tone }, h("time", {}, fmtDT(a.at)), h("span", { class: "what" }, d.what), a.reason ? h("span", { class: "why" }, a.reason) : null);
    }));
  }

  function segmented(options, value, onPick, label) {
    return h("div", { class: "segmented", "aria-label": label || "Filter" }, options.map(([v, text]) =>
      h("button", { type: "button", class: v === value ? "on" : false, "aria-pressed": v === value ? "true" : "false", onclick: () => onPick(v) }, text)));
  }

  // A confirm dialog: fields in `body`, a required-reason check in `onSubmit` (which throws to show a message).
  function confirmDialog({ title, danger, icon: iconName, body, confirm, onSubmit }) {
    const err = h("p", { class: "error", role: "alert", hidden: true });
    const go = h("button", { class: danger ? "danger" : "primary", type: "submit" }, confirm);
    const dlg = h("dialog", { class: "dialog", "aria-label": title },
      h("form", { method: "dialog" },
        h("div", { class: "dialog-head" }, iconName ? h("span", { class: "icon" }, icon(iconName)) : null, h("h2", {}, title)),
        h("div", { class: "dialog-body" }, body, err),
        h("div", { class: "dialog-foot" }, h("button", { type: "button", onclick: () => dlg.close() }, "Cancel"), go)));
    dlg.addEventListener("close", () => dlg.remove());
    dlg.querySelector("form").addEventListener("submit", async (e) => {
      e.preventDefault();
      err.hidden = true;
      go.disabled = true;
      try { await onSubmit(dlg); } catch (ex) { err.textContent = ex.message; err.hidden = false; }
      go.disabled = false;
    });
    document.body.append(dlg);
    dlg.showModal();
    return dlg;
  }
  const facts = (rows) => h("div", { class: "facts" }, rows.map(([k, v]) => h("div", {}, h("span", {}, k), h("span", {}, v))));
  const reasonField = (id, label, required, hint) => [
    h("label", { for: id }, label, " ", required ? h("span", { class: "req" }, "*") : h("span", { class: "opt" }, "(optional)")),
    h("textarea", { id, rows: "3", maxlength: "500", placeholder: hint }),
  ];

  // A searchable, sortable, paged table whose rows come from the server. State lives in the URL.
  function tableView(o) {
    const st = Object.assign({}, o.defaults);
    for (const [k, v] of q) if (k in o.defaults) st[k] = typeof o.defaults[k] === "number" ? Number(v) || o.defaults[k] : v;
    const toolbarBox = h("div", { class: "toolbar-extra" });
    const thead = h("thead");
    const tbody = h("tbody");
    const pager = h("div", { class: "pager" });
    let data = null, seq = 0;
    const search = o.search ? h("div", { class: "search" }, icon("search"),
      h("input", { type: "search", placeholder: o.search, "aria-label": o.search, value: st.q, autocomplete: "off" })) : null;
    const toolbar = h("form", { class: "toolbar", role: "search", onsubmit: (e) => e.preventDefault() }, search, toolbarBox);
    toolbar.style.display = "contents";
    const card = h("section", { class: "card table-card" },
      h("div", { class: "toolbar" }, search, h("span", { class: "toolbar-slot" }, toolbarBox)),
      h("div", { class: "table-wrap" }, h("table", { class: "data" }, thead, tbody)), pager);
    const ctx = { st, data: () => data, set(k, v) { st[k] = v; st.page = 1; load(); }, reload: () => load() };

    function save() {
      const p = new URLSearchParams();
      for (const k in st) if (st[k] !== o.defaults[k] && st[k] !== "") p.set(k, st[k]);
      history.replaceState(null, "", location.pathname + (p.toString() ? "?" + p : "") + location.hash);
    }
    async function load() {
      save();
      const my = ++seq;
      const p = new URLSearchParams();
      for (const k in st) if (st[k] !== "" && st[k] != null) p.set(k, st[k]);
      let d;
      try { d = await api(`${o.endpoint}?${p}`); } catch (ex) {
        if (ex.status === 401) { location.href = "/signin?next=" + encodeURIComponent(location.pathname); return; }
        tbody.replaceChildren(h("tr", { class: "empty-row" }, h("td", { colspan: String(o.columns.length) }, ex.message)));
        return;
      }
      if (my !== seq) return;
      data = d;
      draw();
    }
    function sortTh(c) {
      if (!c.key) return h("th", { class: c.cls || false }, c.label);
      const on = st.sort === c.key;
      return h("th", { class: c.cls || false, "aria-sort": on ? (st.dir === "desc" ? "descending" : "ascending") : false },
        h("button", { class: "sort", type: "button", onclick: () => { if (on) st.dir = st.dir === "desc" ? "asc" : "desc"; else { st.sort = c.key; st.dir = c.firstDesc ? "desc" : "asc"; } st.page = 1; load(); } }, c.label));
    }
    function pageButtons(pages) {
      const cur = st.page, out = [];
      const nums = new Set([1, pages, cur - 1, cur, cur + 1].filter((n) => n >= 1 && n <= pages));
      let prev = 0;
      for (const n of [...nums].sort((a, b) => a - b)) {
        if (n - prev > 1) out.push(h("span", { class: "gap" }, "…"));
        out.push(h("button", { type: "button", class: n === cur ? "on" : false, "aria-current": n === cur ? "page" : false, onclick: () => { st.page = n; load(); } }, String(n)));
        prev = n;
      }
      return out;
    }
    function draw() {
      thead.replaceChildren(h("tr", {}, o.columns.map(sortTh)));
      const items = data.items;
      tbody.replaceChildren(...(items.length ? items.map((it) => o.row(it, data, ctx)) : [h("tr", { class: "empty-row" }, h("td", { colspan: String(o.columns.length) }, o.empty || "Nothing found."))]));
      toolbarBox.replaceChildren(...(o.toolbar ? o.toolbar(ctx, data) : []));
      const pages = Math.max(1, Math.ceil(data.total / st.per));
      const from = data.total ? (st.page - 1) * st.per + 1 : 0, to = Math.min(data.total, st.page * st.per);
      pager.replaceChildren(
        h("span", { class: "count" }, `${from}–${to} of ${data.total} ${o.noun}`),
        h("div", { class: "pages", "aria-label": "Pages" },
          h("button", { type: "button", "aria-label": "Previous page", disabled: st.page <= 1, onclick: () => { st.page--; load(); } }, "‹"),
          pageButtons(pages),
          h("button", { type: "button", "aria-label": "Next page", disabled: st.page >= pages, onclick: () => { st.page++; load(); } }, "›")),
        h("label", {}, "Rows ", h("select", { onchange: (e) => ctx.set("per", Number(e.target.value)) },
          [25, 50, 100].map((n) => h("option", { value: n, selected: n === st.per }, String(n))))));
    }
    if (search) {
      let t;
      search.querySelector("input").addEventListener("input", (e) => { clearTimeout(t); t = setTimeout(() => ctx.set("q", e.target.value.trim()), 250); });
    }
    return { el: card, load, ctx };
  }

  // ---- accounts ----

  pages.accounts = async function (main) {
    const view = tableView({
      endpoint: "/admin/accounts",
      defaults: { q: "", status: "all", sort: "login", dir: "asc", page: 1, per: 25 },
      search: "Login or character name",
      noun: "accounts",
      columns: [{ label: "ID", key: "id", cls: "num hide-phone" }, { label: "Login", key: "login" }, { label: "Characters" }, { label: "Status" },
        { label: "Last login", key: "last_login", firstDesc: true }, { label: "Created", key: "created", cls: "hide-phone", firstDesc: true }],
      toolbar: (ctx, d) => [segmented([["all", `All ${d.counts.accounts}`], ["online", `Online ${d.counts.online}`], ["banned", `Banned ${d.counts.banned}`]], ctx.st.status, (v) => ctx.set("status", v), "Status")],
      row: (a) => h("tr", { class: a.status & 1 ? false : "is-banned" },
        h("td", { class: "num id hide-phone" }, a.id),
        h("td", {}, h("a", { href: `/admin/accounts/${a.id}` }, a.login), roleBadge(a.role) ? [" ", roleBadge(a.role)] : null),
        a.characters.length ? h("td", {}, a.characters.join(", "), a.gm > 0 ? [" ", gmBadge(a.gm)] : null) : h("td", { class: "muted" }, "None yet"),
        h("td", {}, a.status & 1 ? (a.online ? h("span", { class: "badge ok" }, "Online") : h("span", { class: "badge off" }, "Offline")) : h("span", { class: "badge bad" }, "Banned")),
        h("td", {}, a.last_login ? when(a.last_login) : h("span", { class: "muted" }, "Never")),
        h("td", { class: "hide-phone" }, date(a.created))),
    });
    main.append(view.el);
    await view.load();
  };

  function banDialog(a, done) {
    const days = { v: 7 };
    const ends = h("p", { class: "hint" });
    const seg = h("div", { class: "segmented left", id: "ban-length", "aria-label": "How long" });
    const paint = () => {
      seg.replaceChildren(...[[1, "1 day"], [7, "7 days"], [30, "30 days"], [0, "Until I lift it"]].map(([n, text]) =>
        h("button", { type: "button", class: n === days.v ? "on" : false, "aria-pressed": n === days.v ? "true" : "false", onclick: () => { days.v = n; paint(); } }, text)));
      ends.textContent = days.v ? `Ends ${fmtDT(nowSec() + days.v * 86400)}.` : "It stays until someone unbans the account.";
    };
    paint();
    const online = a.characters.filter((c) => c.online);
    confirmDialog({
      title: `Ban ${a.login}?`, danger: true, icon: "bans", confirm: "Ban account",
      body: [
        h("p", {}, "They can't log in until the ban ends. ", online.length ? `${online.map((c) => c.name).join(", ")} ${online.length === 1 ? "is" : "are"} in game now and stay${online.length === 1 ? "s" : ""} in until they log out: a ban doesn't remove anyone who is already playing.` : "A ban doesn't remove anyone who is already in game; it stops the next login."),
        facts([["Account", `${a.login} (${a.id})`], ["Characters", a.characters.map((c) => c.name).join(", ") || "None"]]),
        h("label", { for: "ban-length" }, "How long"), seg, ends,
        reasonField("ban-reason", "Reason", true, "What happened. Kept in the admin log and shown on the Bans page."),
      ],
      onSubmit: async (dlg) => {
        const reason = dlg.querySelector("#ban-reason").value.trim();
        if (!reason) throw new Error("Say why: a reason is needed.");
        await api(`/admin/accounts/${a.id}/ban`, { method: "POST", body: { reason, until: days.v ? nowSec() + days.v * 86400 : undefined } });
        dlg.close();
        done();
      },
    });
  }

  function unbanDialog(login, accid, done) {
    confirmDialog({
      title: `Unban ${login}?`, icon: "bans", confirm: "Unban",
      body: [h("p", {}, "They can log in again straight away."), ...reasonField("unban-reason", "Reason", true, "Why it ends early. Kept in the admin log.")],
      onSubmit: async (dlg) => {
        const reason = dlg.querySelector("#unban-reason").value.trim();
        if (!reason) throw new Error("Say why: a reason is needed.");
        await api(`/admin/accounts/${accid}/unban`, { method: "POST", body: { reason } });
        dlg.close();
        done();
      },
    });
  }

  function passwordDialog(a, done) {
    const result = h("div", { class: "copy-line", hidden: true }, h("code", { id: "new-pw" }), h("button", { type: "button", id: "copy-pw" }, "Copy"));
    const shownNote = h("p", { class: "hint", hidden: true }, "Give it to them yourself. The site doesn't keep it and won't show it again.");
    const dlg = confirmDialog({
      title: `Reset ${a.login}'s password?`, icon: "key", confirm: "Set new password",
      body: [
        h("p", {}, "Their old password stops working and their web sign-ins end. They can change it themselves once they are in."),
        h("label", { for: "pw-new-admin" }, "New password ", h("span", { class: "opt" }, "(leave empty to make one up)")),
        h("input", { id: "pw-new-admin", type: "text", autocomplete: "off", maxlength: "32" }),
        result, shownNote,
        h("label", { for: "pw-reason" }, "Reason ", h("span", { class: "opt" }, "(optional)")),
        h("input", { id: "pw-reason", type: "text", maxlength: "500", placeholder: "Such as: forgot it, asked on Discord" }),
      ],
      onSubmit: async (d) => {
        const typed = d.querySelector("#pw-new-admin").value;
        const out = await api(`/admin/accounts/${a.id}/password`, { method: "POST", body: { new_password: typed || undefined, reason: d.querySelector("#pw-reason").value } });
        if (out.password) {
          d.querySelector("#new-pw").textContent = out.password;
          result.hidden = false;
          shownNote.hidden = false;
          d.querySelector("#copy-pw").onclick = () => navigator.clipboard && navigator.clipboard.writeText(out.password);
        }
        // Nothing more to enter: the one button now closes.
        const go = d.querySelector("button[type=submit]");
        go.textContent = "Done";
        go.type = "button";
        go.onclick = () => { d.close(); done(); };
        d.querySelector("#pw-new-admin").disabled = true;
        if (!out.password) { d.close(); done(); }
      },
    });
    return dlg;
  }

  function totpDialog(a, done) {
    confirmDialog({
      title: `Remove ${a.login}'s two-step sign-in?`, icon: "key", danger: true, confirm: "Remove two-step",
      body: [h("p", {}, "For a player who lost their authenticator. They sign in with the password alone until they set it up again."), ...reasonField("totp-reason", "Reason", true, "For example: lost their phone, confirmed on Discord.")],
      onSubmit: async (dlg) => {
        const reason = dlg.querySelector("#totp-reason").value.trim();
        if (!reason) throw new Error("Say why: a reason is needed.");
        await api(`/admin/accounts/${a.id}/totp-reset`, { method: "POST", body: { reason } });
        dlg.close();
        done();
      },
    });
  }

  // The owner's control for an account's website role. Owners can't change their own role
  // or another Owner's; only the owner link (from the desktop app) can take Owner away.
  function roleCard(a, d, owner, done) {
    if (!owner) return null;
    const pick = h("select", { id: "site-role", disabled: !d.can_act || d.own }, ["player", "moderator", "owner"].map((r) => h("option", { value: r, selected: r === d.role }, ROLE_NAME[r])));
    const save = h("button", { class: "primary", type: "button", disabled: !d.can_act || d.own }, "Save");
    save.addEventListener("click", () => {
      const role = pick.value;
      if (role === d.role) return;
      confirmDialog({
        title: `Make ${a.login} ${role === "owner" ? "an Owner" : "a " + ROLE_NAME[role]}?`, icon: "key", danger: role !== "player", confirm: `Set ${ROLE_NAME[role]}`,
        body: [h("p", {}, role === "owner"
          ? "Owners can do everything on this site, including resetting passwords, changing GM levels and making other Owners. Only the owner link from the desktop app can take Owner away again."
          : role === "moderator"
            ? "Moderators can sign in to this admin panel, see accounts and characters, and ban or unban players."
            : "Players only see their own account. Their admin access ends at once."),
          h("p", {}, "This doesn't change any GM level: GM levels only control in-game commands. Their web sign-ins end, so they sign in again with the new role."),
          ...reasonField("role-reason", "Reason", true, "Why. Kept in the admin log.")],
        onSubmit: async (dlg) => {
          const reason = dlg.querySelector("#role-reason").value.trim();
          if (!reason) throw new Error("Say why: a reason is needed.");
          await api(`/admin/accounts/${a.id}/role`, { method: "POST", body: { role, reason } });
          dlg.close();
          done();
        },
      });
    });
    const hint = d.own ? "You can't change your own website role." : !d.can_act ? "Only the owner link from the desktop app can change another Owner's role." : "Set by owners only. GM levels give no website role.";
    return h("div", {}, h("h2", {}, "Website role"),
      h("dl", { class: "kv" }, h("dt", {}, h("label", { for: "site-role" }, "Role")), h("dd", {}, pick)),
      h("div", { class: "save-row" }, h("span", { class: "hint grow" }, hint), save));
  }

  pages.account = async function (main) {
    const id = pathId();
    async function draw() {
      let d;
      try { d = await api(`/admin/accounts/${encodeURIComponent(id)}`); } catch (ex) { main.replaceChildren(errorCard(ex.status === 404 ? "There is no such account." : ex.message)); return; }
      const a = d.account, banned = !(a.status & 1), owner = me.role === "owner";
      document.title = `${a.login} · Accounts`;
      const online = a.characters.filter((c) => c.online);
      const actions = [];
      if (d.can_act && owner) actions.push(h("button", { type: "button", onclick: () => passwordDialog(a, draw) }, icon("key"), "Reset password"));
      if (d.can_act && owner && a.has_totp) actions.push(h("button", { type: "button", onclick: () => totpDialog(a, draw) }, "Remove two-step"));
      if (d.can_act) actions.push(banned
        ? h("button", { class: "primary", type: "button", onclick: () => unbanDialog(a.login, a.id, draw) }, "Unban")
        : h("button", { class: "danger", type: "button", onclick: () => banDialog(a, draw) }, icon("bans"), "Ban account"));
      const ban = a.ban;
      main.replaceChildren(...[
        h("div", { class: "page-head" },
          h("div", { class: "ident" }, h("span", { class: "avatar" }, a.login[0].toUpperCase()),
            h("div", {}, h("h2", {}, a.login, " ", banned ? h("span", { class: "badge bad" }, "Banned") : h("span", { class: "badge ok" }, "Active"),
              online.length ? [" ", h("span", { class: "badge info plain" }, `${online.length} ${online.length === 1 ? "character" : "characters"} in game`)] : null),
              h("div", { class: "sub" }, "Account ", h("code", {}, a.id), ` · created ${date(a.created)}`))),
          h("div", { class: "toolbar" }, actions)),
        d.can_act ? null : h("div", { class: "update-bar lock" }, icon("lock"), h("p", {}, d.gm_floor ? "A character on this account has an in-game GM level, so only an owner can ban or unban it." : "This account's website role is the same as yours or higher, so you can look but not change it.")),
        banned ? h("div", { class: "update-bar lock" }, icon("bans"), h("p", {}, ban
          ? [h("strong", {}, ban.until ? `Banned until ${fmtDT(ban.until)}. ` : "Banned until someone lifts it. "), ban.reason || "No reason recorded."]
          : "Banned without a record (the ban was made outside this site).")) : null,
        h("div", { class: "grid-2" },
          h("div", { class: "stack" },
            h("section", { class: "card table-card" },
              h("div", { class: "card-head" }, h("h2", {}, "Characters"), h("span", { class: "muted" }, `${a.characters.length} on this account`)),
              h("div", { class: "table-wrap" }, h("table", { class: "data" },
                h("thead", {}, h("tr", {}, h("th", {}, "Name"), h("th", {}, "Job"), h("th", {}, "Zone"), h("th", { class: "hide-phone" }, "GM level"), h("th", {}, "Last played"))),
                h("tbody", {}, a.characters.length ? a.characters.map((c) => h("tr", {},
                  h("td", {}, h("span", { class: "dot" + (c.online ? " ok" : "") }), h("a", { href: `/admin/characters/${c.charid}` }, c.name)),
                  jobCell(c), h("td", {}, c.zone || "–"), h("td", { class: "hide-phone muted" }, c.gm), h("td", {}, c.online ? "Now" : date(c.last_logout))))
                  : [h("tr", { class: "empty-row" }, h("td", { colspan: "5" }, "No characters yet."))])))),
            h("section", { class: "card pad" }, h("div", { class: "card-head" }, h("h2", {}, "History"), h("a", { href: "/admin/log" }, "All in the admin log")), timeline(d.history))),
          h("section", { class: "card pad" }, h("h2", {}, "Account"),
            h("dl", { class: "kv" },
              h("dt", {}, "Login"), h("dd", {}, a.login),
              h("dt", {}, "Account ID"), h("dd", {}, h("code", {}, a.id)),
              h("dt", {}, "Status"), h("dd", {}, banned ? h("span", { class: "badge bad" }, "Banned") : h("span", { class: "badge ok" }, "Active")),
              h("dt", {}, "Created"), h("dd", {}, fmtDT(a.created)),
              h("dt", {}, "Last login"), h("dd", {}, a.last_login ? fmtDT(a.last_login) : "Never"),
              h("dt", {}, "Two-step sign-in"), h("dd", {}, a.has_totp ? "On" : "Off"),
              owner ? null : [h("dt", {}, "Website role"), h("dd", {}, ROLE_NAME[d.role] || d.role)],
              h("dt", {}, "Highest in-game GM level"), h("dd", {}, String(a.gm))),
            roleCard(a, d, owner, draw)))].filter(Boolean));
    }
    await draw();
  };

  // ---- characters ----

  pages.characters = async function (main) {
    const view = tableView({
      endpoint: "/admin/characters",
      defaults: { q: "", online: "", sort: "name", dir: "asc", page: 1, per: 25 },
      search: "Character name",
      noun: "characters",
      columns: [{ label: "Name", key: "name" }, { label: "Job" }, { label: "Zone" }, { label: "Account", key: "account", cls: "hide-phone" }, { label: "Nation", cls: "hide-phone" },
        { label: "Play time", key: "playtime", cls: "num", firstDesc: true }],
      toolbar: (ctx) => [h("label", { class: "editor-filter" }, h("input", { type: "checkbox", class: "switch", checked: ctx.st.online === "1", onchange: (e) => ctx.set("online", e.target.checked ? "1" : "") }), " Online only")],
      row: (c) => h("tr", {},
        h("td", {}, h("span", { class: "dot" + (c.online ? " ok" : "") }), h("a", { href: `/admin/characters/${c.charid}` }, c.name), c.gm > 0 ? [" ", gmBadge(c.gm)] : null),
        jobCell(c), h("td", { class: "zone" }, c.zone || "–"),
        h("td", { class: "hide-phone" }, h("a", { href: `/admin/accounts/${c.accid}` }, c.login)),
        h("td", { class: "hide-phone" }, NATIONS[c.nation] || "–"), h("td", { class: "num" }, playtime(c.playtime))),
    });
    main.append(view.el);
    await view.load();
  };

  const GM_LEVELS = ["0, none", "1", "2", "3", "4", "5, every command"];

  pages.character = async function (main) {
    const id = pathId();
    let d, inv;
    try { [d, inv] = await Promise.all([api(`/admin/characters/${encodeURIComponent(id)}`), api(`/admin/characters/${encodeURIComponent(id)}/inventory`)]); } catch (ex) {
      main.append(errorCard(ex.status === 404 ? "There is no such character." : ex.message)); return;
    }
    const c = d.character, owner = me.role === "owner";
    document.title = `${c.name} · Characters`;
    const tab = (tid, label, on) => h("button", { class: "subtab" + (on ? " active" : ""), role: "tab", id: "tab-" + tid, "aria-controls": "p-" + tid, "aria-selected": on ? "true" : "false" }, label);
    const panel = (tid, on, ...kids) => h("div", { id: "p-" + tid, class: "tabpanel", role: "tabpanel", "aria-labelledby": "tab-" + tid, hidden: !on }, kids);
    const stat = (label, value, sub) => h("section", { class: "card stat" }, h("div", { class: "stat-label" }, label), h("div", { class: "stat-value" }, value), h("div", { class: "stat-sub" }, sub || ""));

    const mayGm = owner && (d.can_act || d.own);
    const gm = h("select", { id: "gm", disabled: !mayGm || c.online }, GM_LEVELS.map((t, i) => h("option", { value: i, selected: i === c.gm }, t)));
    const save = h("button", { class: "primary", type: "button", disabled: !mayGm || c.online }, "Save");
    save.addEventListener("click", () => {
      const level = Number(gm.value);
      if (level === c.gm) return;
      confirmDialog({
        title: `Change ${c.name}'s GM level?`, icon: "key", danger: level >= 3, confirm: `Set level ${level}`,
        body: [h("p", {}, `From ${c.gm} to ${level}. It applies the next time they log in or change zone. It only changes which GM commands they can use in game, not their website role.`),
          ...reasonField("gm-reason", "Reason", true, "Why. Kept in the admin log.")],
        onSubmit: async (dlg) => {
          const reason = dlg.querySelector("#gm-reason").value.trim();
          if (!reason) throw new Error("Say why: a reason is needed.");
          await api(`/admin/characters/${c.charid}/gm-level`, { method: "POST", body: { level, reason } });
          location.reload();
        },
      });
    });
    const gmHint = !owner ? "Only an owner can change GM levels." : !mayGm ? "This character's account has the Owner website role." : c.online ? "Locked while in game." : "";

    main.append(
      h("div", { class: "page-head" },
        h("div", { class: "ident" }, h("span", { class: "avatar" }, c.name[0].toUpperCase()),
          h("div", {}, h("h2", {}, c.name, " ", c.online ? h("span", { class: "badge ok" }, "In game") : h("span", { class: "badge off" }, "Offline"), c.gm > 0 ? [" ", gmBadge(c.gm)] : null),
            h("div", { class: "sub" }, "Character ", h("code", {}, c.charid), " · account ", h("a", { href: `/admin/accounts/${c.accid}` }, c.login), roleBadge(d.role) ? [" ", roleBadge(d.role)] : null, ` · ${NATIONS[c.nation] || "Unknown"}`)))),
      ...(c.online ? [h("div", { class: "update-bar lock" }, icon("lock"), h("p", {}, h("strong", {}, `${c.name} is in game, so this page is read-only. `),
        "The map server holds an online character and writes it back when they log out, which would undo changes made here. Editing opens again when they log out."))] : []),
      h("nav", { class: "subtabs", role: "tablist", "aria-label": "Character sections" }, tab("overview", "Overview", true), tab("jobs", "Jobs"), tab("inventory", "Inventory"), tab("history", "History")));

    main.append(panel("overview", true,
      h("div", { class: "stats four" },
        stat("Job", [jobLabel(c.mjob, c.mlvl), c.sjob >= 1 ? h("small", {}, ` / ${jobLabel(c.sjob, c.slvl)}`) : null], `Level cap ${c.cap}`),
        stat("Zone", c.zone || "–", c.online ? "In game now" : `Logged out ${fmtDT(c.last_logout)}`),
        stat("Gil", num(c.gil), "In inventory"),
        stat("Play time", playtime(c.playtime), c.created ? `Since ${date(c.created)}` : "")),
      h("div", { class: "grid-2" },
        h("section", { class: "card pad" }, h("h2", {}, "Character"),
          h("dl", { class: "kv" },
            h("dt", {}, "Name"), h("dd", {}, c.name),
            h("dt", {}, "Nation"), h("dd", {}, NATIONS[c.nation] || "Unknown"),
            h("dt", {}, "Created"), h("dd", {}, fmtDT(c.created)),
            h("dt", {}, "Last logout"), h("dd", {}, fmtDT(c.last_logout)))),
        h("section", { class: "card pad" }, h("h2", {}, "Game master"),
          h("dl", { class: "kv" }, h("dt", {}, h("label", { for: "gm" }, "GM level")), h("dd", {}, gm,
            h("p", { class: "hint" }, "Controls in-game GM commands only. It gives no website role: website roles are set on the account's page."))),
          h("div", { class: "save-row" }, h("span", { class: "hint grow" }, gmHint), save)))));
    main.append(panel("jobs", false, jobsCard(c)));
    main.append(panel("inventory", false, inventoryCard(inv)));
    main.append(panel("history", false, h("section", { class: "card pad" },
      h("div", { class: "card-head" }, h("h2", {}, "History"), h("span", { class: "muted" }, "From the admin log, for this character and its account")),
      timeline(d.history))));
    wireTabs(main);
  };

  // ---- bans ----

  pages.bans = async function (main) {
    const reload = () => view.load();
    const view = tableView({
      endpoint: "/admin/bans",
      defaults: { q: "", page: 1, per: 25 },
      search: "Account or character",
      noun: "banned accounts",
      empty: "Nobody is banned.",
      columns: [{ label: "Account" }, { label: "Characters", cls: "hide-phone" }, { label: "Banned", cls: "hide-phone" }, { label: "Ends" }, { label: "Reason" }, { label: "" }],
      row: (b, d) => {
        const rec = b.ban;
        const ends = !rec ? h("span", { class: "muted" }, "Unknown")
          : rec.until ? [h("span", { class: "badge busy" }, rec.until > d.now ? `in ${inWords(rec.until - d.now)}` : "due now"), h("div", { class: "hint" }, fmtDT(rec.until))]
          : h("span", { class: "badge bad" }, "Until lifted");
        return h("tr", { class: "is-banned" },
          h("td", {}, h("a", { href: `/admin/accounts/${b.accid}` }, b.login), " ", h("span", { class: "id" }, b.accid)),
          h("td", { class: "hide-phone" }, b.characters.join(", ") || "–"),
          h("td", { class: "hide-phone" }, rec && rec.banned_at ? fmtDT(rec.banned_at) : h("span", { class: "muted" }, "Unknown")),
          h("td", {}, ends),
          h("td", { class: "wrap" }, rec && rec.reason ? rec.reason : h("span", { class: "muted" }, "No record")),
          h("td", { class: "row-actions" }, h("button", { type: "button", onclick: () => unbanDialog(b.login, b.accid, reload) }, "Unban")));
      },
    });
    main.append(view.el,
      h("p", { class: "hint" }, "A ban sets the account's status to banned and keeps the dates and reason in ", h("code", {}, "accounts_banned"), ", so the login server refuses it even while this site is off. A ban that ends on its own is lifted by this site within a minute of its end, so it only ends while the site is running. A ban doesn't remove anyone who is already in game. To ban someone, open their account. Ended bans are in the ", h("a", { href: "/admin/log?action=unban" }, "admin log"), "."));
    await view.load();
  };

  // ---- the admin log ----

  pages.log = async function (main) {
    let facetsDone = false;
    const view = tableView({
      endpoint: "/admin/log",
      defaults: { q: "", actor: "", action: "", days: "30", page: 1, per: 25 },
      search: "Account, character or reason",
      noun: "entries",
      empty: "No entries match.",
      columns: [{ label: "When" }, { label: "Who" }, { label: "Action" }, { label: "Target" }, { label: "Details" }],
      toolbar: (ctx, d) => {
        const sel = (label, key, opts) => h("select", { "aria-label": label, onchange: (e) => ctx.set(key, e.target.value) },
          opts.map(([v, t]) => h("option", { value: v, selected: v === ctx.st[key] }, t)));
        return [
          sel("Who", "actor", [["", "Anyone"], ...d.actors.map((a) => [a, a])]),
          sel("Action", "action", [["", "Any action"], ...d.actions.map((a) => [a, ACTION_LABEL[a] || a])]),
          sel("When", "days", [["1", "Today"], ["7", "Last 7 days"], ["30", "Last 30 days"], ["0", "Everything"]]),
        ];
      },
      row: (a) => {
        const dsc = describe(a);
        const target = a.target_kind === "news" ? (a.action === "news_delete" ? h("span", {}, a.target_name || "Post") : h("a", { href: `/admin/news/${a.target_id}` }, a.target_name || a.target_id))
          : a.target_kind === "content" ? h("a", { href: "/admin/content" }, a.target_name || "Site content")
          : a.target_id && a.target_kind ? h("a", { href: `/admin/${a.target_kind === "account" ? "accounts" : "characters"}/${a.target_id}` }, a.target_name || a.target_id) : h("span", { class: "muted" }, "–");
        const detail = a.action === "sign_in_failed" ? `Login "${a.actor_login || ""}": ${a.reason || ""}` : [dsc.what, a.reason ? `. ${a.reason}` : ""].join("");
        return h("tr", {},
          h("td", {}, when(a.at)), h("td", {}, a.action === "sign_in_failed" ? h("span", { class: "muted" }, "Unknown") : actorName(a)),
          h("td", {}, h("span", { class: "badge " + (ACTION_TONE[a.action] || "info") }, ACTION_LABEL[a.action] || a.action)),
          h("td", {}, target), h("td", { class: "wrap" }, a.action === "sign_in" ? (a.actor_kind === "owner" ? "Owner link" : "Game account") : detail));
      },
    });
    void facetsDone;
    main.append(view.el);
    await view.load();
  };

  // ---- the owner's editors: news posts and site content ----

  let editorSeq = 0;

  // A Markdown box with a small toolbar and Write/Preview tabs. Preview asks the server, so it
  // shows exactly what visitors get.
  function mdEditor({ value, short, max, onInput }) {
    const n = ++editorSeq;
    const ta = h("textarea", { class: "body-input" + (short ? " short" : ""), "aria-label": "Text (Markdown)", spellcheck: "true" });
    ta.value = value || "";
    const view = h("div", { class: "preview", id: `pv-${n}`, role: "tabpanel", hidden: true });
    const write = h("div", { id: `wr-${n}`, role: "tabpanel" }, ta);
    const count = h("span", { class: "count-note" });
    const tabW = h("button", { type: "button", class: "subtab active", role: "tab", "aria-selected": "true", "aria-controls": `wr-${n}` }, "Write");
    const tabP = h("button", { type: "button", class: "subtab", role: "tab", "aria-selected": "false", "aria-controls": `pv-${n}` }, "Preview");
    function show(preview) {
      tabW.classList.toggle("active", !preview); tabP.classList.toggle("active", preview);
      tabW.setAttribute("aria-selected", String(!preview)); tabP.setAttribute("aria-selected", String(preview));
      write.hidden = preview; view.hidden = !preview;
    }
    tabW.addEventListener("click", () => show(false));
    tabP.addEventListener("click", async () => {
      show(true);
      view.replaceChildren(h("p", { class: "muted" }, "Loading the preview…"));
      try {
        const out = await api("/admin/preview", { method: "POST", body: { markdown: ta.value } });
        rendered(view, "");
        view.append(prose(out.html));
      } catch (ex) { view.replaceChildren(errorCard(ex.message)); }
    });
    // Wrap the selection (or put the marks around nothing) and keep the caret in the right place.
    function wrap(before, after, fallback) {
      const a = ta.selectionStart, b = ta.selectionEnd;
      const sel = ta.value.slice(a, b) || fallback;
      ta.setRangeText(before + sel + after, a, b, "end");
      ta.setSelectionRange(a + before.length, a + before.length + sel.length);
      ta.focus();
      changed();
    }
    function linePrefix(prefix) {
      const a = ta.selectionStart;
      const start = ta.value.lastIndexOf("\n", a - 1) + 1;
      ta.setRangeText(prefix, start, start, "end");
      ta.focus();
      changed();
    }
    const tool = (label, title, fn, extra) => h("button", { class: "icon-button", type: "button", title, "aria-label": title, onclick: fn }, extra || label);
    const bar = h("div", { class: "fmt" },
      tool("B", "Bold", () => wrap("**", "**", "bold")),
      tool("I", "Italic", () => wrap("*", "*", "italic"), h("i", {}, "I")),
      tool("H", "Heading", () => linePrefix("## ")),
      tool("•", "List", () => linePrefix("- ")),
      tool("↗", "Link", () => wrap("[", "](https://)", "link text")),
      count,
      h("nav", { class: "subtabs", role: "tablist", "aria-label": "Editor view" }, tabW, tabP));
    function changed() {
      count.textContent = max ? `${ta.value.length.toLocaleString()} / ${max.toLocaleString()}` : "";
      count.classList.toggle("error", !!max && ta.value.length > max);
      if (onInput) onInput();
    }
    ta.addEventListener("input", changed);
    changed();
    return { el: h("div", {}, bar, write, view), get: () => ta.value, set: (v) => { ta.value = v; show(false); changed(); }, focus: () => ta.focus() };
  }

  function statusLine() {
    const el = h("span", { class: "editor-status", role: "status" });
    return { el, say: (text, dirty) => { el.textContent = text; el.classList.toggle("dirty", !!dirty); } };
  }

  pages["admin-news"] = async function (main) {
    const idParam = pathId();
    const editing = idParam !== "news" && /^\d+$/.test(idParam) ? Number(idParam) : null;
    const list = await api("/admin/news");
    let post = null;
    if (editing != null) post = await api(`/admin/news/${editing}`);
    document.title = `${post ? post.title : "New post"} · News posts`;

    let dirty = false;
    const status = statusLine();
    const title = h("input", { class: "title-input", type: "text", maxlength: String(list.max_title), placeholder: "Title", "aria-label": "Title", value: post ? post.title : "" });
    const author = h("input", { type: "text", maxlength: String(list.max_author), placeholder: "Staff", "aria-label": "Author name", value: post ? post.author : "" });
    const pinned = h("input", { type: "checkbox", class: "switch", "aria-label": "Pin to the top of the home page" });
    pinned.checked = !!(post && post.pinned);
    const markDirty = () => { dirty = true; status.say("Changes not saved", true); };
    title.addEventListener("input", markDirty);
    author.addEventListener("input", markDirty);
    pinned.addEventListener("change", markDirty);
    const editor = mdEditor({ value: post ? post.body : "", max: list.max_body, onInput: markDirty });
    const err = h("p", { class: "error", role: "alert", hidden: true });
    const published = post && post.published != null;
    window.addEventListener("beforeunload", (e) => { if (dirty) { e.preventDefault(); e.returnValue = ""; } });

    async function save(publish) {
      err.hidden = true;
      const body = { title: title.value, body: editor.get(), author: author.value, pinned: pinned.checked };
      if (publish !== undefined) body.publish = publish;
      buttons.forEach((b) => { b.disabled = true; });
      try {
        let out;
        if (post) out = await api(`/admin/news/${post.id}`, { method: "PUT", body });
        else out = await api("/admin/news", { method: "POST", body });
        dirty = false;
        location.href = `/admin/news/${out.id}`;
      } catch (ex) {
        err.textContent = ex.message; err.hidden = false;
        buttons.forEach((b) => { b.disabled = false; });
      }
    }
    const bDiscard = h("button", { type: "button", onclick: () => { dirty = false; location.reload(); } }, "Discard");
    const bDraft = h("button", { type: "button", onclick: () => save(false) }, "Save draft");
    const bPublish = h("button", { type: "button", class: "primary", onclick: () => save(true) }, "Publish");
    const bSave = h("button", { type: "button", class: "primary", onclick: () => save(undefined) }, "Save changes");
    const buttons = published ? [bDiscard, bSave] : [bDiscard, bDraft, bPublish];
    status.say(post ? (published ? "Published" : "Draft") : "New post", false);

    const side = [];
    if (post) {
      const bUnpub = h("button", { type: "button", onclick: () => save(false) }, "Unpublish");
      const bDel = h("button", { type: "button", class: "danger", onclick: () => confirmDialog({
        title: `Delete "${post.title}"?`, danger: true, icon: "bans", confirm: "Delete post",
        body: h("p", {}, published ? "It disappears from the site now and can't be brought back. Unpublish instead to keep it as a draft." : "The draft is gone for good."),
        onSubmit: async () => { await api(`/admin/news/${post.id}`, { method: "DELETE", body: {} }); dirty = false; location.href = "/admin/news"; },
      }) }, "Delete");
      side.push(h("section", { class: "card pad" },
        h("h2", {}, "Publishing"),
        h("dl", { class: "kv" },
          h("dt", {}, "Status"), h("dd", {}, h("span", { class: "badge " + (published ? "ok" : "off") }, published ? "Published" : "Draft")),
          h("dt", {}, "Pinned"), h("dd", {}, pinned),
          h("dt", {}, "Author"), h("dd", {}, author),
          published ? [h("dt", {}, "Shown"), h("dd", {}, fmtDT(post.published))] : null,
          h("dt", {}, "Edited"), h("dd", {}, fmtDT(post.updated), post.updated_by ? ` by ${post.updated_by}` : "")),
        h("div", { class: "form-actions" }, published ? bUnpub : null, bDel)));
    } else {
      side.push(h("section", { class: "card pad" },
        h("h2", {}, "Publishing"),
        h("dl", { class: "kv" },
          h("dt", {}, "Pinned"), h("dd", {}, pinned),
          h("dt", {}, "Author"), h("dd", {}, author)),
        h("p", { class: "hint" }, "The author is the name shown under the post, not your login. Left empty it reads “Staff”.")));
    }
    side.push(h("section", { class: "card table-card" },
      h("div", { class: "card-head" }, h("h2", {}, "Posts"), h("a", { class: "btn-link", href: "/admin/news" }, "New post")),
      h("table", { class: "data" }, h("tbody", {}, list.items.length ? list.items.map((p) =>
        h("tr", {}, h("td", { class: "wrap" },
          h("a", { href: `/admin/news/${p.id}`, "aria-current": post && post.id === p.id ? "page" : false }, p.title),
          h("div", { class: "hint" }, p.published == null ? h("span", { class: "badge off" }, "Draft") : [p.pinned ? "Pinned · " : "", date(p.published)], ` · ${p.author}`))))
        : h("tr", {}, h("td", { class: "muted" }, "No posts yet."))))));

    main.append(h("div", { class: "editor-grid" },
      h("section", { class: "card editor-card" }, title, editor.el,
        h("div", { class: "editor-bar" }, status.el, err, buttons)),
      h("div", { class: "stack" }, side)));
    if (!post) title.focus();
  };

  pages["admin-content"] = async function (main) {
    const d = await api("/admin/content");
    main.append(h("p", { class: "hint" }, "Text the public pages show. Each block has a default; “Reset to default” puts it back. Markdown works everywhere except the title; raw HTML is shown as text."));
    for (const b of d.blocks) {
      let cur = b;
      const status = statusLine();
      const err = h("p", { class: "error", role: "alert", hidden: true });
      let field, get, set;
      const dirtyNow = () => status.say("Changes not saved", true);
      if (b.plain) {
        const input = h("input", { class: "title-input", type: "text", maxlength: String(b.max), "aria-label": b.label });
        input.value = b.body;
        input.addEventListener("input", dirtyNow);
        field = input; get = () => input.value; set = (v) => { input.value = v; };
      } else {
        const ed = mdEditor({ value: b.body, short: b.max <= 2000, max: b.max, onInput: dirtyNow });
        field = ed.el; get = ed.get; set = ed.set;
      }
      const note = (c) => (c.is_default ? "Showing the default" : `Saved ${fmtDT(c.updated)}${c.updated_by ? " by " + c.updated_by : ""}`);
      const bSave = h("button", { type: "button", class: "primary" }, "Save");
      const bReset = h("button", { type: "button" }, "Reset to default");
      status.say(note(b), false);
      bSave.addEventListener("click", async () => {
        err.hidden = true; bSave.disabled = true;
        try {
          await api(`/admin/content/${b.key}`, { method: "PUT", body: { body: get() } });
          cur = Object.assign({}, cur, { is_default: false, updated: nowSec(), updated_by: me.login || "Owner link" });
          status.say(note(cur), false);
        } catch (ex) { err.textContent = ex.message; err.hidden = false; }
        bSave.disabled = false;
      });
      bReset.addEventListener("click", () => confirmDialog({
        title: `Reset "${b.label}"?`, icon: "bans", confirm: "Reset to default",
        body: h("p", {}, "The text goes back to what the site says until it is edited. What is there now is lost."),
        onSubmit: async (dlg) => {
          await api(`/admin/content/${b.key}`, { method: "DELETE", body: {} });
          set(b.default); cur = Object.assign({}, cur, { is_default: true });
          status.say(note(cur), false);
          dlg.close();
        },
      }));
      main.append(h("section", { class: "card editor-card block-card" },
        h("div", { class: "card-head" }, h("h2", {}, b.label)),
        h("p", { class: "hint" }, b.hint),
        field,
        h("div", { class: "editor-bar" }, status.el, err, bReset, bSave)));
    }
  };

  pages["admin-invites"] = async function (main) {
    const d = await api("/admin/invites");
    const err = h("p", { class: "error", role: "alert", hidden: true });
    const fail = (ex) => { err.textContent = ex.message; err.hidden = false; };
    const reload = () => location.reload();

    const need = h("input", { type: "checkbox", class: "switch", "aria-label": "Sign-up needs an invite code" });
    need.checked = d.invite_required;
    need.addEventListener("change", async () => {
      try { await api("/admin/signup", { method: "PUT", body: { invite_required: need.checked } }); } catch (ex) { need.checked = !need.checked; fail(ex); }
    });
    main.append(h("section", { class: "card pad" },
      h("h2", {}, "Sign-up"),
      h("dl", { class: "kv" },
        h("dt", {}, "Server allows accounts"), h("dd", {}, h("span", { class: "badge " + (d.lsb_open ? "ok" : "off") }, d.lsb_open ? "Open" : "Closed")),
        h("dt", {}, "Needs an invite code"), h("dd", {}, need)),
      h("p", { class: "hint" }, "“Open” follows LSB's login.ACCOUNT_CREATION setting (Game settings in the desktop app). Without a code anyone who finds the page can make an account, up to 3 an hour per address and 30 a day.")));

    const note = h("input", { type: "text", maxlength: String(d.max_note), placeholder: "Who it is for", "aria-label": "Note" });
    const uses = h("input", { type: "number", min: "1", max: String(d.max_uses), value: "1", "aria-label": "Uses" });
    const days = h("input", { type: "number", min: "0", max: "365", value: "0", "aria-label": "Days it works" });
    const shown = h("div", { class: "stack" });
    const make = h("form", { class: "card pad form-grid", novalidate: true },
      h("h2", {}, "New invite code"),
      h("label", {}, "Note"), note,
      h("label", {}, "Uses"), uses,
      h("label", {}, "Works for (days, 0 = no end)"), days,
      err,
      h("div", { class: "form-actions" }, h("button", { class: "primary", type: "submit" }, "Make code")),
      shown);
    make.addEventListener("submit", async (e) => {
      e.preventDefault();
      err.hidden = true;
      try {
        const out = await api("/admin/invites", { method: "POST", body: { note: note.value, max_uses: Number(uses.value), days: Number(days.value) } });
        const btn = h("button", { class: "small", type: "button" }, "Copy");
        btn.addEventListener("click", () => copyText(out.code, btn));
        shown.replaceChildren(h("p", { class: "hint" }, "Copy it now: it isn't shown again."), h("div", { class: "copy-line" }, h("code", {}, out.code), btn),
          h("button", { type: "button", onclick: reload }, "Done"));
      } catch (ex) { fail(ex); }
    });

    const badge = { active: "ok", "used up": "off", expired: "off", revoked: "bad" };
    main.append(make, h("section", { class: "card table-card" },
      h("div", { class: "card-head" }, h("h2", {}, "Codes")),
      h("div", { class: "table-wrap" }, h("table", { class: "data" },
        h("thead", {}, h("tr", {}, ["Code", "Note", "Uses", "Ends", "Made", "State", ""].map((t) => h("th", {}, t)))),
        h("tbody", {}, d.items.length ? d.items.map((i) => h("tr", {},
          h("td", {}, h("code", {}, `…${i.hint}`)),
          h("td", { class: "wrap" }, i.note || "–"),
          h("td", { class: "num" }, `${i.uses} / ${i.max_uses}`),
          h("td", {}, i.expires ? fmtDT(i.expires) : "No end"),
          h("td", {}, `${date(i.created)} · ${i.created_by}`),
          h("td", {}, h("span", { class: "badge " + (badge[i.state] || "off") }, i.state)),
          h("td", {}, i.state === "active" ? h("button", { class: "small danger", type: "button", onclick: () => confirmDialog({
            title: "Revoke this code?", danger: true, icon: "bans", confirm: "Revoke",
            body: h("p", {}, "It stops working now. Accounts already made with it stay."),
            onSubmit: async () => { await api(`/admin/invites/${i.id}/revoke`, { method: "POST", body: {} }); reload(); },
          }) }, "Revoke") : null)))
          : h("tr", { class: "empty-row" }, h("td", { colspan: "7" }, "No codes yet.")))))));
  };

  // ---- start ----

  (async function start() {
    try { me = await api("/me"); } catch { me = { role: "visitor", server_name: "Server" }; }
    if (page === "signin") return pages.signin();
    if (page === "signup") return pages.signup();
    if (body.classList.contains("app")) {
      if (!isAdmin()) { location.replace("/signin?next=" + encodeURIComponent(location.pathname)); return; }
      adminShell();
      if (page !== "dashboard") homeInfo().then((i) => setOnline(i.online)).catch(() => setOnline(null));
    } else {
      siteShell();
      homeInfo().then((i) => setOnline(i.online)).catch(() => setOnline(null));
    }
    const main = document.getElementById("main");
    if (pages[page] && main) {
      try { await pages[page](main); } catch (ex) { main.append(errorCard(ex.message)); }
    }
  })();
})();
