// Popover UI. All dynamic text goes through textContent; nothing from the
// transcripts is ever inserted as HTML.
(() => {
  const tauri = window.__TAURI__;
  const invoke = tauri.core.invoke;
  const $ = (id) => document.getElementById(id);

  const state = { days: 1, tab: "projects", summary: null, config: null };

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
    if (n < 1e9) return (n / 1e6).toFixed(2) + "M";
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
    el.className = "toast" + (ok ? " ok" : "");
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => (el.textContent = ""), 6000);
  }

  // ---------- limits ----------
  const WINDOW_SECS = { five_hour: 5 * 3600, seven_day: 7 * 86400 };
  const WINDOW_NAME = { five_hour: "5-hour", seven_day: "Weekly" };

  function forecastText(f, now) {
    if (f.used_pct >= 100) return { text: "Limit reached", cls: "crit" };
    if (f.hits_limit_at != null) {
      const early = fmtDur(f.resets_at - f.hits_limit_at);
      return { text: `At this pace: limit at ${clock(f.hits_limit_at, true)}, ${early} before reset`, cls: f.hits_limit_at - now < 3600 ? "crit" : "warn" };
    }
    if (f.basis === "none") return { text: "Not enough data for a forecast yet", cls: "" };
    return { text: `At this pace: ${Math.round(f.projected_pct_at_reset)}% at reset`, cls: "" };
  }

  function limitCard(f, now) {
    const elapsed = 1 - (f.resets_at - now) / WINDOW_SECS[f.window];
    const fc = forecastText(f, now);
    return h("div", { class: "limit" },
      h("div", { class: "limit-head" },
        h("span", { class: "limit-name", text: WINDOW_NAME[f.window] }),
        h("span", { class: "limit-reset", text: `resets in ${fmtDur(f.resets_at - now)} · ${clock(f.resets_at, true)}` }),
        h("span", { class: "limit-pct", text: `${Math.round(f.used_pct)}%` })),
      h("div", { class: `meter ${level(f.used_pct)}`, role: "progressbar", "aria-valuenow": Math.round(f.used_pct), "aria-valuemin": 0, "aria-valuemax": 100 },
        h("i", { style: pctWidth(f.used_pct) }),
        h("b", { style: `left:${Math.max(0, Math.min(100, elapsed * 100)).toFixed(1)}%`, title: "Time elapsed in this window. A bar past this mark is ahead of an even pace." })),
      h("div", { class: `forecast ${fc.cls}`, text: fc.text }));
  }

  function renderLimits(s) {
    const box = $("limits");
    box.replaceChildren();
    const now = s.generated_at;
    if (s.forecasts.length) {
      box.append(...s.forecasts.map((f) => limitCard(f, now)));
      return;
    }
    const installed = s.hook.state === "installed";
    const hint = h("div", { class: "hint" });
    if (installed) {
      hint.append(h("p", { text: "Waiting for Claude Code. Limits appear after the next response in any session (Pro and Max plans)." }));
    } else {
      hint.append(
        h("p", { text: "Real 5-hour and weekly limits come from Claude Code's statusline. No credentials, no network calls." }),
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

  // ---------- strip + totals ----------
  function renderBurn(s) {
    const b = s.burn;
    $("burn").replaceChildren(
      h("span", {}, "Last 30 min ", h("strong", { text: `${fmtTokens(b.tokens_per_min)}/min` })),
      h("span", {}, h("strong", { text: `${fmtUsd(b.cost_per_hour)}/h` }), " API-equivalent"));
  }

  function renderRangeTotal(s) {
    const t = s.report.totals;
    $("range-total").replaceChildren(
      h("strong", { text: fmtUsd(t.cost_usd) }),
      h("span", { class: "muted", text: ` · ${fmtTokens(t.total_tokens)} tok` }));
    $("range-total").title = t.unpriced_tokens > 0 ? `${fmtTokens(t.unpriced_tokens)} tokens from models without a known price are not included in the cost.` : "What this usage would cost at API list prices.";
  }

  // ---------- lists ----------
  function costRows(rows, nameOf, subOf, total) {
    if (!rows.length) return [h("div", { class: "empty", text: "No usage in this period." })];
    const max = Math.max(...rows.map((r) => r.totals.cost_usd), 0) || Math.max(...rows.map((r) => r.totals.total_tokens), 1);
    const useCost = total.cost_usd > 0;
    return rows.map((r) => {
      const value = useCost ? r.totals.cost_usd : r.totals.total_tokens;
      return h("div", { class: "row", title: r.project || r.model || "" },
        h("div", { class: "row-main" },
          h("span", { class: "row-name", text: nameOf(r) }),
          h("span", { class: "row-value", text: fmtUsd(r.totals.cost_usd) })),
        h("div", { class: "row-sub" },
          h("span", { class: "grow", text: subOf(r) }),
          h("span", { text: `${fmtTokens(r.totals.total_tokens)} tok` })),
        h("div", { class: "share" }, h("i", { style: pctWidth((value / max) * 100) })));
    });
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
          h("span", { text: `${fmtDur(s.generated_at - v.last_seen)} ago` })),
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
    const chart = h("div", { class: "chart", role: "img", "aria-label": `Daily ${useCost ? "API-equivalent cost" : "tokens"}` },
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

  // ---------- footer ----------
  function renderHook(s) {
    const el = $("hook-state");
    const age = s.limits_age_secs;
    let dot = "", text = "Live limits off";
    if (s.hook.state === "installed") {
      if (age == null) { dot = "stale"; text = "Hook on, waiting for Claude Code"; }
      else if (age > 900) { dot = "stale"; text = `Limits from ${fmtDur(age)} ago`; }
      else { dot = "on"; text = "Live limits"; }
    } else if (age != null) {
      dot = "stale"; text = `Limits from ${fmtDur(age)} ago`;
    }
    el.replaceChildren(h("span", { class: `dot ${dot}` }), h("span", { text }));
  }

  function render() {
    const s = state.summary;
    if (!s) return;
    renderLimits(s);
    renderBurn(s);
    renderRangeTotal(s);
    $("list").replaceChildren(...TABS[state.tab](s));
    renderHook(s);
    $("updated").textContent = clock(s.generated_at);
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
    $("hook-toggle").textContent = installed ? "Remove hook" : "Enable live limits";
    $("hook-toggle").className = installed ? "btn" : "btn primary";
    $("hook-detail").textContent = installed
      ? `Installed${s.hook.chained ? `, runs your statusline afterwards (${s.hook.chained})` : ""}.`
      : s.hook.state === "other"
        ? `Another statusline is configured (${s.hook.command}). Enabling keeps it working.`
        : "Adds one entry to ~/.claude/settings.json. A backup is kept next to it.";
    $("pricing-note").textContent = `Costs use API list prices as of ${s.pricing_as_of}. Subscriptions are not billed per token.`;
  }

  async function openSettings() {
    try {
      state.config = await invoke("get_config");
    } catch (e) {
      return toast(String(e));
    }
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
  load();
})();
