// Popover UI. All dynamic text goes through textContent; nothing from the
// transcripts is ever inserted as HTML.
(() => {
  const tauri = window.__TAURI__;
  const invoke = tauri.core.invoke;
  const $ = (id) => document.getElementById(id);

  const state = { days: 1, tab: "projects", summary: null, config: null, info: null };

  // ---------- helpers ----------
  function h(tag, attrs, ...children) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v == null || v === false) continue;
      if (k === "class") el.className = v;
      else if (k === "text") el.textContent = v;
      else if (k === "style") el.style.cssText = v;
      else el.setAttribute(k, v);
    }
    for (const c of children.flat()) if (c) el.append(c);
    return el;
  }

  function fmtTokens(n) {
    if (n == null) return "–";
    if (n < 1e3) return String(Math.round(n));
    if (n < 1e6) return (n / 1e3).toFixed(1) + "K";
    if (n < 1e9) return (n / 1e6).toFixed(n < 1e8 ? 2 : 1) + "M";
    return (n / 1e9).toFixed(2) + "B";
  }

  function fmtUsd(v) {
    if (v == null) return "–";
    if (v >= 1000) return "$" + Math.round(v).toLocaleString("en-US");
    return "$" + v.toFixed(2);
  }

  function fmtDur(secs) {
    const s = Math.max(0, Math.floor(secs));
    const d = Math.floor(s / 86400), hh = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
    if (d > 0) return `${d}d ${hh}h`;
    if (hh > 0) return `${hh}h ${String(m).padStart(2, "0")}m`;
    return `${m}m`;
  }

  function clock(ts, withDay) {
    const d = new Date(ts * 1000);
    const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const sameDay = new Date().toDateString() === d.toDateString();
    return withDay && !sameDay ? `${d.toLocaleDateString([], { weekday: "short" })} ${time}` : time;
  }

  const level = (pct) => (pct >= 85 ? "crit" : pct >= 60 ? "warn" : "");
  const pctWidth = (pct) => `width:${Math.max(0, Math.min(100, pct)).toFixed(1)}%`;

  function toast(message, ok) {
    const el = $("toast");
    el.textContent = message;
    el.title = message;
    el.className = "toast" + (ok ? " ok" : "");
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => (el.textContent = ""), 7000);
  }

  // ---------- limits ----------
  const WINDOW_SECS = { five_hour: 5 * 3600, seven_day: 7 * 86400 };
  const WINDOW_NAME = { five_hour: "5-hour", seven_day: "Weekly" };
  const FAMILIES = ["fable", "mythos", "opus", "sonnet", "haiku"];
  const famClass = (name) => `fam-${FAMILIES.includes(name.toLowerCase()) ? name.toLowerCase() : "other"}`;

  function forecastChip(f, now) {
    if (f.used_pct >= 100) return { text: "Limit reached", cls: "crit" };
    if (f.hits_limit_at != null) {
      const early = fmtDur(f.resets_at - f.hits_limit_at);
      return { text: `Limit at ${clock(f.hits_limit_at, true)} · ${early} before reset`, cls: f.hits_limit_at - now < 3600 ? "crit" : "warn" };
    }
    if (f.basis === "none") return { text: "Forecast needs a few more minutes", cls: "" };
    return { text: `On pace for ${Math.round(f.projected_pct_at_reset)}% at reset`, cls: "ok" };
  }

  function meter(pct, elapsedPct) {
    return h("div", { class: `meter ${level(pct)}`, role: "progressbar", "aria-valuenow": Math.round(pct), "aria-valuemin": 0, "aria-valuemax": 100 },
      h("i", { class: pct <= 0 ? "zero" : "", style: pctWidth(pct) }),
      elapsedPct == null ? null : h("b", { style: `left:${Math.max(0, Math.min(100, elapsedPct)).toFixed(1)}%`, title: "Time elapsed in this window. A bar past this mark is ahead of an even pace." }));
  }

  function modelMix(s) {
    const rows = s.week_models.filter((m) => m.share_pct >= 0.5);
    if (!rows.length) return null;
    const tip = "Share of this week's usage by model, measured in API-equivalent cost. Claude Code reports one weekly percentage, not one per model.";
    return h("div", { class: "mix", title: tip },
      h("div", { class: "mix-bar" }, rows.map((m) => h("i", { class: famClass(m.family), style: `width:${m.share_pct.toFixed(1)}%` }))),
      h("div", { class: "mix-legend" }, rows.slice(0, 4).map((m) =>
        h("span", {}, h("i", { class: `sw ${famClass(m.family)}` }), m.family + " ", h("strong", { text: `${Math.round(m.share_pct)}%` })))));
  }

  function limitCard(f, s) {
    const now = s.generated_at;
    const elapsed = (1 - (f.resets_at - now) / WINDOW_SECS[f.window]) * 100;
    const chip = forecastChip(f, now);
    return h("div", { class: "limit" },
      h("div", { class: "limit-head" },
        h("div", { class: "limit-id" },
          h("span", { class: "limit-name", text: WINDOW_NAME[f.window] }),
          h("span", { class: "limit-reset", text: `Resets in ${fmtDur(f.resets_at - now)} · ${clock(f.resets_at, true)}` })),
        h("span", { class: "limit-pct" }, String(Math.round(f.used_pct)), h("small", { text: "%" }))),
      meter(f.used_pct, elapsed),
      h("span", { class: `chip ${chip.cls}`, text: chip.text }),
      f.window === "seven_day" ? modelMix(s) : null);
  }

  function extraRow(w, now) {
    return h("div", { class: "extra", title: `Resets in ${fmtDur(w.resets_at - now)} · ${clock(w.resets_at, true)}` },
      h("span", { class: "limit-name", text: w.label }),
      meter(w.used_pct, null),
      h("span", { class: "num", text: `${Math.round(w.used_pct)}%` }));
  }

  function renderLimits(s) {
    const box = $("limits");
    box.replaceChildren();
    const now = s.generated_at;
    if (s.forecasts.length || s.extra_windows.length) {
      box.append(...s.forecasts.map((f) => limitCard(f, s)), ...s.extra_windows.map((w) => extraRow(w, now)));
      return;
    }
    const installed = s.hook.state === "installed";
    const hint = h("div", { class: "hint" });
    if (installed) {
      hint.append(h("p", { text: "Waiting for Claude Code. Limits appear after the next response in any session (Pro and Max plans)." }));
    } else {
      hint.append(
        h("p", { text: "See your real 5-hour and weekly limits. They come from Claude Code's statusline: no credentials, no network calls." }),
        h("button", { class: "btn primary", id: "hint-install", text: "Enable live limits" }));
      if (s.hook.state === "other") hint.append(h("p", { class: "muted small", text: "Your current statusline keeps rendering as it does today." }));
    }
    if (s.block_estimate) {
      const b = s.block_estimate;
      hint.append(h("p", { class: "muted small", text: `Estimated 5-hour window: resets about ${clock(b.end, true)} · ${fmtTokens(b.totals.total_tokens)} tokens so far` }));
    }
    box.append(hint);
    const btn = $("hint-install");
    if (btn) btn.addEventListener("click", installHook);
  }

  // ---------- readouts ----------
  const RANGE_NAME = { 1: "Today", 7: "7 days", 30: "30 days" };
  const ago = (secs) => (secs < 60 ? "now" : `${fmtDur(secs)} ago`);

  function stat(label, value, unit, title) {
    return h("div", { class: "stat", title },
      h("div", { class: "stat-label", text: label }),
      h("div", { class: "stat-value" }, value, unit ? h("small", { text: unit }) : null));
  }

  function renderStats(s) {
    const t = s.report.totals;
    const unpriced = t.unpriced_tokens > 0 ? ` ${fmtTokens(t.unpriced_tokens)} tokens from models without a known price are not included.` : "";
    $("stats").replaceChildren(
      stat("Burn (30 min)", fmtTokens(s.burn.tokens_per_min), " /min", "Tokens per minute over the last 30 minutes, cache reads included."),
      stat("API-equivalent", fmtUsd(s.burn.cost_per_hour), " /h", "What the last 30 minutes would cost per hour at API list prices."),
      stat(`${RANGE_NAME[s.report.days] || `${s.report.days} days`} · ${fmtTokens(t.total_tokens)}`, fmtUsd(t.cost_usd), null, `API-equivalent cost and tokens for the selected period.${unpriced}`));
  }

  // ---------- lists ----------
  function costRows(rows, nameOf, subOf, total) {
    if (!rows.length) return [h("div", { class: "empty", text: "No usage in this period." })];
    const useCost = total.cost_usd > 0;
    const val = (r) => (useCost ? r.totals.cost_usd : r.totals.total_tokens);
    const max = Math.max(...rows.map(val), 0) || 1;
    return rows.map((r) =>
      h("div", { class: "row", title: r.project || r.model || "" },
        h("div", { class: "row-main" },
          h("span", { class: "row-name", text: nameOf(r) }),
          h("span", { class: "row-value", text: fmtUsd(r.totals.cost_usd) })),
        h("div", { class: "row-sub" },
          h("span", { class: "grow", text: subOf(r) }),
          h("span", { text: `${fmtTokens(r.totals.total_tokens)} tok` })),
        h("div", { class: "share" }, h("i", { style: pctWidth((val(r) / max) * 100) }))));
  }

  function renderProjects(s) {
    return costRows(s.report.by_project, (r) => r.name || r.project,
      (r) => `${r.sessions} session${r.sessions === 1 ? "" : "s"} · ${fmtTokens(r.totals.output)} out`, s.report.totals);
  }

  function renderModels(s) {
    return costRows(s.report.by_model, (r) => r.model.replace(/^claude-/, ""),
      (r) => `${r.totals.messages} responses · ${fmtTokens(r.totals.cache_read)} cache read`, s.report.totals);
  }

  function renderSessions(s) {
    if (!s.sessions.length) return [h("div", { class: "empty", text: "No Claude Code sessions in the last 6 hours." })];
    return s.sessions.map((v) => {
      const pct = v.context_used_pct;
      const size = v.context_size ? ` / ${fmtTokens(v.context_size)}` : "";
      return h("div", { class: "row", title: v.project },
        h("div", { class: "row-main" },
          h("span", { class: "row-name", text: v.name || v.session_id.slice(0, 8) }),
          h("span", { class: "row-value", text: pct != null ? `${Math.round(pct)}%` : fmtTokens(v.context_tokens) })),
        h("div", { class: "row-sub" },
          h("span", { class: "grow", text: [v.model, `${fmtTokens(v.context_tokens)}${size} in context`].filter(Boolean).join(" · ") }),
          h("span", { text: ago(s.generated_at - v.last_seen) })),
        pct != null ? h("div", { class: `share ctx ${level(pct)}` }, h("i", { style: pctWidth(pct) })) : null);
    });
  }

  function renderDays(s) {
    const days = s.report.by_day;
    const useCost = s.report.totals.cost_usd > 0;
    const val = (d) => (useCost ? d.totals.cost_usd : d.totals.total_tokens);
    const max = Math.max(...days.map(val), 0);
    if (max <= 0) return [h("div", { class: "empty", text: "No usage in this period." })];
    const show = (d) => (useCost ? fmtUsd(d.totals.cost_usd) : fmtTokens(d.totals.total_tokens));
    const step = Math.ceil(days.length / 7);
    const chart = h("div", { class: "chart", role: "img", "aria-label": `Daily ${useCost ? "API-equivalent cost" : "tokens"}, peak ${show(days.reduce((a, b) => (val(b) > val(a) ? b : a)))}` },
      days.map((d, i) => h("div", { class: `col${i === days.length - 1 ? " today" : ""}${val(d) <= 0 ? " zero" : ""}`, title: `${d.date}: ${show(d)} · ${fmtTokens(d.totals.total_tokens)} tokens` },
        h("i", { style: `height:${Math.max(1.5, (val(d) / max) * 100).toFixed(1)}%` }))));
    const axis = h("div", { class: "chart-axis" },
      days.map((d, i) => h("span", { text: i % step !== 0 ? "" : days.length > 10 ? d.date.slice(8) : d.date.slice(8) + "/" + d.date.slice(5, 7) })));
    const rows = [...days].reverse().filter((d) => val(d) > 0).map((d) =>
      h("div", { class: "row" }, h("div", { class: "row-main" },
        h("span", { class: "row-name", text: new Date(d.date + "T12:00:00").toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" }) }),
        h("span", { class: "muted num", text: `${fmtTokens(d.totals.total_tokens)} tok` }),
        h("span", { class: "row-value", text: fmtUsd(d.totals.cost_usd) }))));
    return [chart, axis, ...rows];
  }

  const TABS = { projects: renderProjects, sessions: renderSessions, days: renderDays, models: renderModels };

  // ---------- status ----------
  function renderStatus(s) {
    const age = s.limits_age_secs;
    let dot = "", text = "Live limits off";
    if (s.hook.state === "installed") {
      if (age == null) { dot = "stale"; text = "Waiting for Claude Code"; }
      else if (age > 900) { dot = "stale"; text = `Limits from ${fmtDur(age)} ago`; }
      else { dot = "on"; text = "Live limits"; }
    } else if (age != null) {
      dot = "stale"; text = `Limits from ${fmtDur(age)} ago`;
    }
    $("hook-state").replaceChildren(h("span", { class: `dot ${dot}` }), h("span", { text }));
    $("live").textContent = `Updated ${clock(s.generated_at)}`;
  }

  function render() {
    const s = state.summary;
    if (!s) return;
    renderLimits(s);
    renderStats(s);
    $("list").replaceChildren(...TABS[state.tab](s));
    renderStatus(s);
    renderSettingsHook(s);
  }

  async function load() {
    try {
      state.summary = await invoke("get_summary", { rangeDays: state.days });
      render();
    } catch (e) {
      toast(String(e));
    }
  }

  // ---------- hook + settings ----------
  async function installHook() {
    try {
      const out = await invoke("hook_install");
      toast(out.wrapped_existing ? "Enabled. Your statusline was kept." : "Enabled. Send a message in Claude Code.", true);
      await load();
    } catch (e) {
      toast(String(e));
    }
  }

  async function removeHook() {
    try {
      await invoke("hook_uninstall");
      toast("Hook removed.", true);
      await load();
    } catch (e) {
      toast(String(e));
    }
  }

  function renderSettingsHook(s) {
    const installed = s.hook.state === "installed";
    const dev = state.info && state.info.dev_build;
    $("hook-toggle").textContent = installed ? "Remove hook" : "Enable live limits";
    $("hook-toggle").className = installed ? "btn" : "btn primary";
    let detail = installed
      ? `Installed${s.hook.chained ? `, runs your statusline afterwards (${s.hook.chained})` : ""}.`
      : s.hook.state === "other"
        ? `Another statusline is configured (${s.hook.command}). Enabling keeps it working.`
        : "Adds one entry to ~/.claude/settings.json. A backup is kept next to it.";
    if (installed && /--ctc-hook/.test(s.hook.command || "")) {
      detail = "Installed by an older build. Remove it and enable live limits again to keep the numbers updating.";
    } else if (installed && /[\\/]target[\\/](debug|release)[\\/]/.test(s.hook.command || "")) {
      detail += " It points at a development build: open the installed app and enable it again there.";
    } else if (dev && !installed) {
      detail += " This is a development build; enable from the installed app instead.";
    }
    $("hook-detail").textContent = detail;
    $("pricing-note").textContent = `Costs use API list prices as of ${s.pricing_as_of}. Subscriptions are not billed per token.`;
  }

  async function loadInfo() {
    try {
      state.info = await invoke("app_info");
      $("version").textContent = `v${state.info.version}${state.info.dev_build ? " dev" : ""}`;
      $("cfg-login").checked = !!state.info.launch_at_login;
    } catch (e) {
      /* older shell without app_info: leave the footer empty */
    }
  }

  async function openSettings() {
    try {
      state.config = await invoke("get_config");
    } catch (e) {
      return toast(String(e));
    }
    await loadInfo();
    // Only the macOS menu bar can show text next to the icon.
    $("cfg-menu-bar").closest(".field").hidden = !!(state.info && state.info.platform && state.info.platform !== "macos");
    const c = state.config;
    $("cfg-menu-bar").value = c.menu_bar;
    $("cfg-notifications").checked = c.notifications;
    $("cfg-forecast").checked = c.forecast_alerts;
    $("cfg-thresholds").value = c.thresholds.join(", ");
    $("settings").hidden = false;
  }

  async function saveSettings() {
    if (!state.config) return;
    const thresholds = $("cfg-thresholds").value.split(/[^0-9]+/).filter(Boolean).map(Number).filter((n) => n >= 1 && n <= 100);
    const config = { ...state.config, menu_bar: $("cfg-menu-bar").value, notifications: $("cfg-notifications").checked, forecast_alerts: $("cfg-forecast").checked, thresholds };
    try {
      state.config = await invoke("set_config", { config });
      $("cfg-thresholds").value = state.config.thresholds.join(", ");
    } catch (e) {
      toast(String(e));
    }
  }

  async function toggleLogin() {
    const box = $("cfg-login");
    try {
      box.checked = await invoke("set_launch_at_login", { enabled: box.checked });
    } catch (e) {
      box.checked = false;
      $("settings").hidden = true;
      toast(String(e));
    }
  }

  // ---------- wiring ----------
  function segmented(containerId, attr, onPick) {
    $(containerId).addEventListener("click", (ev) => {
      const btn = ev.target.closest("button");
      if (!btn) return;
      for (const b of $(containerId).querySelectorAll("button")) b.classList.toggle("on", b === btn);
      onPick(btn.dataset[attr]);
    });
  }

  segmented("range", "days", (d) => { state.days = Number(d); load(); });
  segmented("tabs", "tab", (t) => { state.tab = t; render(); });
  $("refresh").addEventListener("click", load);
  $("quit").addEventListener("click", () => invoke("quit"));
  $("open-settings").addEventListener("click", openSettings);
  $("close-settings").addEventListener("click", () => ($("settings").hidden = true));
  for (const id of ["cfg-menu-bar", "cfg-notifications", "cfg-forecast", "cfg-thresholds"]) $(id).addEventListener("change", saveSettings);
  $("cfg-login").addEventListener("change", toggleLogin);
  $("hook-toggle").addEventListener("click", () => (state.summary && state.summary.hook.state === "installed" ? removeHook() : installHook()));
  document.addEventListener("keydown", (ev) => {
    if (ev.key !== "Escape") return;
    if (!$("settings").hidden) $("settings").hidden = true;
    else invoke("hide_window");
  });

  if (tauri.event) {
    tauri.event.listen("summary", (ev) => {
      // Background refreshes use the range the popover last asked for.
      if (ev.payload && ev.payload.report.days === state.days) {
        state.summary = ev.payload;
        render();
      }
    });
  }
  loadInfo().then(load);
})();
