// DPI-Bypass frontend — a small hand-rolled SPA (no framework, per spec).
import { api, Profile, Settings, DomainCheck, Strategy } from "./api";
import { listen } from "@tauri-apps/api/event";
import tr from "./i18n/tr.json";
import en from "./i18n/en.json";

type Dict = Record<string, string>;
const DICTS: Record<string, Dict> = { tr, en };

let settings: Settings;
let lang = "tr";

function t(key: string, params: Record<string, string> = {}): string {
  let s = DICTS[lang]?.[key] ?? DICTS["en"][key] ?? key;
  for (const [k, v] of Object.entries(params)) s = s.replace(`{${k}}`, v);
  return s;
}

// Escape user-controlled strings (profile names, domains, imported data) before
// interpolating into innerHTML. CSP blocks inline scripts, but this prevents
// markup injection from a crafted profile too.
function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}

function el(html: string): HTMLElement {
  const tpl = document.createElement("template");
  tpl.innerHTML = html.trim();
  return tpl.content.firstElementChild as HTMLElement;
}

function toast(msg: string, isErr = false) {
  const node = el(`<div class="toast ${isErr ? "err" : ""}">${msg}</div>`);
  document.body.appendChild(node);
  setTimeout(() => node.remove(), 4200);
}

// A persistent top banner used by the background monitor (network change, etc.).
function banner(msg: string, actionLabel: string, action: () => void) {
  document.querySelectorAll(".banner").forEach((b) => b.remove());
  const node = el(`<div class="banner">
      <span>${esc(msg)}</span>
      <span class="spacer"></span>
      <button class="btn" data-a="go">${esc(actionLabel)}</button>
      <button class="btn ghost" data-a="x">✕</button>
    </div>`);
  (node.querySelector('[data-a="go"]') as HTMLButtonElement).onclick = () => {
    node.remove();
    action();
  };
  (node.querySelector('[data-a="x"]') as HTMLButtonElement).onclick = () => node.remove();
  document.body.appendChild(node);
}

type Screen = "home" | "add" | "profiles" | "settings" | "about";
let current: Screen = "home";

const view = () => document.getElementById("view")!;

function renderNav() {
  const nav = document.getElementById("nav")!;
  const items: [Screen, string][] = [
    ["home", t("nav.home")],
    ["add", t("nav.add")],
    ["profiles", t("nav.profiles")],
    ["settings", t("nav.settings")],
    ["about", t("nav.about")],
  ];
  nav.innerHTML = `<div class="brand">DPI<span>-Bypass</span></div>`;
  for (const [id, label] of items) {
    const b = el(`<button class="navbtn ${current === id ? "active" : ""}">${label}</button>`);
    b.onclick = () => go(id);
    nav.appendChild(b);
  }
}

function go(screen: Screen) {
  current = screen;
  renderNav();
  render();
}

function reachLabel(check: DomainCheck): { text: boolean; voice: boolean | null } {
  return {
    text: check.text === "reachable",
    voice: check.voice == null ? null : check.voice === "reachable",
  };
}

// ---------- Home ----------
async function renderHome() {
  const v = view();
  v.innerHTML = `<h1>${t("nav.home")}</h1>`;

  const active = await api.engineStatus().catch(() => false);
  const profiles = await api.listProfiles().catch(() => [] as Profile[]);
  const defId = await api.defaultProfileId().catch(() => null);
  const def = profiles.find((p) => p.id === defId) ?? profiles[0];

  const card = el(`
    <div class="card status-card">
      <div class="status-dot ${active ? "on" : "off"}"></div>
      <div>
        <div class="status-text">${active ? t("status.active") : t("status.inactive")}</div>
        <div class="muted">${t("status.activeProfile")}: ${def ? esc(def.name) : t("status.noProfile")}</div>
      </div>
      <div class="spacer"></div>
      <button class="btn" id="toggle">${active ? t("status.toggleOff") : t("status.toggleOn")}</button>
    </div>`);
  v.appendChild(card);

  if (def) {
    const tr = def.test_results;
    v.appendChild(
      el(`<div class="card">
        <h2>${esc(def.domains[0] ?? "—")}</h2>
        <div class="row">
          <span class="pill ${tr.text ? "ok" : "bad"}">${t("status.text")}: ${tr.text ? "✅" : "❌"}</span>
          <span class="pill ${tr.voice ? "ok" : "bad"}">${t("status.voice")}: ${tr.voice ? "✅" : "❌"}</span>
        </div>
      </div>`),
    );
  }

  // Always-on
  const svc = await api.serviceStatus().catch(() => ({ enabled: false, active: false }));
  const ao = el(`
    <div class="card row">
      <div><strong>${t("status.alwaysOn")}</strong><div class="muted">systemd</div></div>
      <div class="spacer"></div>
      <label class="switch"><input type="checkbox" id="ao" ${svc.enabled ? "checked" : ""}/><span class="slider"></span></label>
    </div>`);
  v.appendChild(ao);

  (card.querySelector("#toggle") as HTMLButtonElement).onclick = async () => {
    try {
      if (active) await api.engineRevert();
      else if (def) await api.engineApply(def.id);
      else return go("add");
      renderHome();
    } catch (e) {
      toast(`${t("common.error")}: ${e}`, true);
    }
  };
  (ao.querySelector("#ao") as HTMLInputElement).onchange = async (ev) => {
    const on = (ev.target as HTMLInputElement).checked;
    try {
      await api.setAlwaysOn(on);
      toast(t("settings.saved"));
    } catch (e) {
      toast(`${t("common.error")}: ${e}`, true);
    }
  };
}

// ---------- Add Domain ----------
async function renderAdd() {
  const v = view();
  v.innerHTML = `<h1>${t("nav.add")}</h1>`;
  const domains = await api.discordDomains().catch(() => ["discord.com"]);

  const card = el(`
    <div class="card">
      <h2>${t("add.prompt")}</h2>
      <div class="row">
        <input type="text" id="domain" placeholder="${t("add.placeholder")}" value="discord.com"/>
        <button class="btn" id="test">${t("add.test")}</button>
      </div>
      <p class="muted" id="msg"></p>
    </div>`);
  v.appendChild(card);

  const msg = card.querySelector("#msg") as HTMLElement;
  (card.querySelector("#test") as HTMLButtonElement).onclick = async () => {
    const domain = (card.querySelector("#domain") as HTMLInputElement).value.trim();
    if (!domain) return;
    const withVoice = domain.includes("discord");
    msg.textContent = t("add.checking");
    try {
      const check = await api.checkDomain(domain, withVoice);
      const r = reachLabel(check);
      if (r.text && (r.voice === null || r.voice)) {
        msg.textContent = t("add.reachable");
        return;
      }
      msg.textContent = t("add.solving");
      const set = domain.includes("discord") ? domains : [domain];
      const outcome = await api.solve(set, withVoice);
      if (outcome.outcome === "already_open") {
        msg.textContent = t("add.reachable");
      } else if (outcome.outcome === "found") {
        const profile = await api.createProfile(set, outcome.strategy, outcome.check);
        msg.innerHTML = `${t("add.found", { name: profile.name })}<br/><span class="muted">${t("add.foundNote")}</span>`;
      } else {
        msg.textContent = t("add.notFound");
      }
    } catch (e) {
      msg.textContent = `${t("common.error")}: ${e}`;
    }
  };
}

// ---------- Profiles ----------
async function renderProfiles() {
  const v = view();
  v.innerHTML = `<h1>${t("nav.profiles")}</h1>`;
  const profiles = await api.listProfiles().catch(() => [] as Profile[]);
  const defId = await api.defaultProfileId().catch(() => null);

  const bar = el(`<div class="row" style="margin-bottom:14px">
    <button class="btn ghost" id="import">${t("profiles.import")}</button></div>`);
  v.appendChild(bar);
  (bar.querySelector("#import") as HTMLButtonElement).onclick = async () => {
    const json = prompt(t("profiles.import") + " (JSON):");
    if (!json) return;
    try {
      await api.importProfile(json);
      renderProfiles();
    } catch (e) {
      toast(`${t("common.error")}: ${e}`, true);
    }
  };

  if (profiles.length === 0) {
    v.appendChild(el(`<div class="card muted">${t("profiles.empty")}</div>`));
    return;
  }

  for (const p of profiles) {
    const fp = p.network_fingerprint;
    const net = fp.iface ? `${fp.link_type ?? "?"} · ${fp.subnet ?? "?"}` : "—";
    const card = el(`
      <div class="card profile">
        <div class="row">
          <span class="title">${esc(p.name)}</span>
          ${p.id === defId ? `<span class="pill ok">${t("profiles.default")}</span>` : ""}
        </div>
        <div class="muted">${t("profiles.network")}: ${esc(net)}</div>
        <div class="muted">${esc(p.domains.join(", "))}</div>
        <div class="row" style="margin-top:8px">
          <button class="btn" data-a="activate">${t("profiles.activate")}</button>
          <button class="btn ghost" data-a="default">${t("profiles.makeDefault")}</button>
          <button class="btn ghost" data-a="rename">${t("profiles.rename")}</button>
          <button class="btn ghost" data-a="advanced">${t("profiles.advanced")}</button>
          <button class="btn ghost" data-a="export">${t("profiles.export")}</button>
          <button class="btn danger" data-a="delete">${t("profiles.delete")}</button>
        </div>
      </div>`);
    card.querySelectorAll("button[data-a]").forEach((btn) => {
      (btn as HTMLButtonElement).onclick = () => profileAction(btn.getAttribute("data-a")!, p);
    });
    v.appendChild(card);
  }
}

async function profileAction(action: string, p: Profile) {
  try {
    switch (action) {
      case "activate":
        await api.engineApply(p.id);
        toast(t("status.active"));
        break;
      case "default":
        await api.setDefaultProfile(p.id);
        renderProfiles();
        break;
      case "rename": {
        const name = prompt(t("profiles.rename"), p.name);
        if (name) {
          await api.renameProfile(p.id, name);
          renderProfiles();
        }
        break;
      }
      case "advanced":
        openStrategyEditor(p);
        break;
      case "export": {
        const json = await api.exportProfile(p.id);
        await navigator.clipboard.writeText(json).catch(() => {});
        toast(t("profiles.export") + " → clipboard");
        break;
      }
      case "delete":
        if (confirm(`${t("profiles.delete")}: ${p.name}?`)) {
          await api.deleteProfile(p.id);
          renderProfiles();
        }
        break;
    }
  } catch (e) {
    toast(`${t("common.error")}: ${e}`, true);
  }
}

// ---------- Advanced strategy editor (Faz 4) ----------
function openStrategyEditor(p: Profile) {
  const s = p.strategy;
  const udp = s.udp_quic;
  const modal = el(`
    <div class="modal-backdrop">
      <div class="modal card">
        <h2>${t("editor.title")}: ${esc(p.name)}</h2>
        <p class="muted">${t("editor.note")}</p>

        <h3>${t("editor.tcp")}</h3>
        <div class="grid2">
          <div class="field"><label>${t("editor.desync")}</label>
            <input type="text" id="t_desync" value="${esc(s.tcp.desync)}"/></div>
          <div class="field"><label>${t("editor.splitPos")}</label>
            <input type="text" id="t_split" value="${esc(s.tcp.split_pos)}"/></div>
          <div class="field"><label>${t("editor.ttl")}</label>
            <input type="text" id="t_ttl" value="${s.tcp.ttl}"/></div>
          <div class="field"><label>${t("editor.fooling")}</label>
            <input type="text" id="t_fooling" value="${esc(s.tcp.fooling)}"/></div>
          <div class="field"><label>${t("editor.repeats")}</label>
            <input type="text" id="t_repeats" value="${s.tcp.repeats}"/></div>
        </div>

        <h3>${t("editor.udp")}
          <label class="inline"><input type="checkbox" id="u_on" ${udp ? "checked" : ""}/> ${t("editor.udpEnable")}</label>
        </h3>
        <div class="grid2" id="udpBox">
          <div class="field"><label>${t("editor.desync")}</label>
            <input type="text" id="u_desync" value="${esc(udp?.desync ?? "fake")}"/></div>
          <div class="field"><label>${t("editor.ttl")}</label>
            <input type="text" id="u_ttl" value="${udp?.ttl ?? 0}"/></div>
          <div class="field"><label>${t("editor.repeats")}</label>
            <input type="text" id="u_repeats" value="${udp?.repeats ?? 2}"/></div>
        </div>

        <div class="row" style="margin-top:14px">
          <button class="btn" data-a="save">${t("editor.save")}</button>
          <button class="btn ghost" data-a="cancel">${t("editor.cancel")}</button>
        </div>
      </div>
    </div>`);

  const close = () => modal.remove();
  (modal.querySelector('[data-a="cancel"]') as HTMLButtonElement).onclick = close;
  modal.addEventListener("click", (e) => {
    if (e.target === modal) close();
  });

  const num = (id: string) => {
    const n = parseInt((modal.querySelector("#" + id) as HTMLInputElement).value, 10);
    return Number.isFinite(n) ? n : 0;
  };
  const str = (id: string) => (modal.querySelector("#" + id) as HTMLInputElement).value.trim();

  (modal.querySelector('[data-a="save"]') as HTMLButtonElement).onclick = async () => {
    const strategy: Strategy = {
      tcp: {
        desync: str("t_desync") || "fake,split2",
        split_pos: str("t_split") || "midsld",
        ttl: num("t_ttl"),
        fooling: str("t_fooling") || "none",
        repeats: Math.max(1, num("t_repeats")),
      },
      udp_quic: (modal.querySelector("#u_on") as HTMLInputElement).checked
        ? {
            desync: str("u_desync") || "fake",
            ttl: num("u_ttl"),
            repeats: Math.max(1, num("u_repeats")),
          }
        : undefined,
    };
    try {
      await api.updateProfileStrategy(p.id, strategy);
      close();
      toast(t("editor.saved"));
      renderProfiles();
    } catch (e) {
      toast(`${t("common.error")}: ${e}`, true);
    }
  };

  document.body.appendChild(modal);
}

// ---------- Settings ----------
function renderSettings() {
  const v = view();
  v.innerHTML = `<h1>${t("nav.settings")}</h1>`;
  const card = el(`
    <div class="card">
      <div class="field">
        <label>${t("settings.language")}</label>
        <select id="lang">
          <option value="tr" ${settings.language === "tr" ? "selected" : ""}>Türkçe</option>
          <option value="en" ${settings.language === "en" ? "selected" : ""}>English</option>
        </select>
      </div>
      <div class="field">
        <label>${t("settings.theme")}</label>
        <select id="theme">
          <option value="dark" ${settings.theme === "dark" ? "selected" : ""}>${t("settings.themeDark")}</option>
          <option value="light" ${settings.theme === "light" ? "selected" : ""}>${t("settings.themeLight")}</option>
        </select>
      </div>
      <div class="field">
        <label>${t("settings.interval")}</label>
        <input type="text" id="interval" value="${settings.auto_test_interval_min}"/>
      </div>
      <div class="field">
        <label><input type="checkbox" id="autostart" ${settings.autostart ? "checked" : ""}/> ${t("settings.autostart")}</label>
      </div>
      <div class="field">
        <label>${t("settings.scope")}</label>
        <select id="scope">
          <option value="discord" ${settings.scope === "discord" ? "selected" : ""}>${t("settings.scopeDiscord")}</option>
          <option value="browsers" ${settings.scope === "browsers" ? "selected" : ""}>${t("settings.scopeBrowsers")}</option>
          <option value="all_browsers" ${settings.scope === "all_browsers" ? "selected" : ""}>${t("settings.scopeAllBrowsers")}</option>
          <option value="system" ${settings.scope === "system" ? "selected" : ""}>${t("settings.scopeSystem")}</option>
        </select>
      </div>
      <p class="muted">${t("settings.scopeLinux")}</p>
      <div class="row">
        <button class="btn" id="save">${t("settings.save")}</button>
        <button class="btn ghost" id="reset">${t("settings.reset")}</button>
      </div>
    </div>`);
  v.appendChild(card);

  (card.querySelector("#save") as HTMLButtonElement).onclick = async () => {
    settings = {
      ...settings,
      language: (card.querySelector("#lang") as HTMLSelectElement).value,
      theme: (card.querySelector("#theme") as HTMLSelectElement).value,
      scope: (card.querySelector("#scope") as HTMLSelectElement).value,
      auto_test_interval_min:
        parseInt((card.querySelector("#interval") as HTMLInputElement).value, 10) || 30,
      autostart: (card.querySelector("#autostart") as HTMLInputElement).checked,
    };
    try {
      await api.setSettings(settings);
      lang = settings.language;
      applyTheme();
      renderNav();
      toast(t("settings.saved"));
    } catch (e) {
      toast(`${t("common.error")}: ${e}`, true);
    }
  };
  (card.querySelector("#reset") as HTMLButtonElement).onclick = async () => {
    try {
      await api.engineRevert();
      await api.setAlwaysOn(false).catch(() => {});
      toast(t("settings.saved"));
    } catch (e) {
      toast(`${t("common.error")}: ${e}`, true);
    }
  };
}

// ---------- About ----------
function renderAbout() {
  view().innerHTML = `
    <h1>${t("nav.about")}</h1>
    <div class="card">
      <div class="brand" style="font-size:22px;font-weight:700">DPI<span style="color:var(--accent)">-Bypass</span> <span class="muted">v0.1.0</span></div>
      <p>${t("about.tagline")}</p>
      <p class="muted">${t("about.privacy")}</p>
      <p class="muted">${t("about.responsible")}</p>
    </div>`;
}

function applyTheme() {
  document.body.setAttribute("data-theme", settings.theme);
}

function render() {
  switch (current) {
    case "home": renderHome(); break;
    case "add": renderAdd(); break;
    case "profiles": renderProfiles(); break;
    case "settings": renderSettings(); break;
    case "about": renderAbout(); break;
  }
}

async function boot() {
  try {
    settings = await api.getSettings();
  } catch {
    settings = { language: "tr", theme: "dark", scope: "discord", auto_test_interval_min: 30, autostart: false };
  }
  lang = settings.language;
  applyTheme();
  renderNav();
  render();

  // Background-monitor events (spec §10.3 / §10.4). Fired from the Rust side.
  await listen("monitor://network-changed", () => {
    banner(t("monitor.networkChanged"), t("monitor.findNew"), () => go("add"));
  });
  await listen("monitor://retest", () => {
    if (current === "home") renderHome();
  });
}

boot();
