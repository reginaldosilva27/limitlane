// Generated demo data in the shape of the app's `Summary` (crates/core/src/summary.rs).
// Nothing here comes from real usage.
(() => {
  // Fri 27 Nov 2026, 09:12 local time (the recorder pins the clock to this).
  const NOW = Math.floor(new Date(2026, 10, 27, 9, 12, 0).getTime() / 1000);
  const H = 3600, D = 86400;

  function rng(seed) {
    return () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  // Tokens per dollar of API-equivalent cost; cache reads make this large.
  const TOK_PER_USD = 870000;

  function totals(cost, messages) {
    const total = Math.round(cost * TOK_PER_USD);
    return {
      input: Math.round(total * 0.004),
      output: Math.round(total * 0.012),
      cache_read: Math.round(total * 0.91),
      cache_write: Math.round(total * 0.074),
      total_tokens: total,
      cost_usd: cost,
      unpriced_tokens: 0,
      messages: messages ?? Math.round(cost * 9),
    };
  }

  const PROJECTS = [
    ["acme-api", 0.34, 9],
    ["web-dashboard", 0.24, 7],
    ["data-pipeline", 0.17, 5],
    ["mobile-app", 0.12, 4],
    ["infra-terraform", 0.08, 3],
    ["docs-site", 0.05, 2],
  ];
  const MODELS = [
    ["claude-fable-5-2", 0.83],
    ["claude-sonnet-5-5", 0.09],
    ["claude-opus-5-5", 0.06],
    ["claude-haiku-5-5", 0.02],
  ];

  function dateStr(ts) {
    const d = new Date(ts * 1000);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }

  function report(days) {
    const midnight = Math.floor(new Date(2026, 10, 27).getTime() / 1000);
    const byDay = [];
    for (let i = days - 1; i >= 0; i--) {
      const ts = midnight - i * D;
      const dow = new Date(ts * 1000).getDay();
      const weekend = dow === 0 || dow === 6;
      // Seeded by date, so a day costs the same in the 7- and 30-day views.
      const r = rng(1000 + i);
      let cost = i === 0 ? 48.2 : (weekend ? 6 + r() * 14 : 42 + r() * 96);
      cost = Math.round(cost * 100) / 100;
      byDay.push({ date: dateStr(ts), totals: totals(cost) });
    }
    const sum = byDay.reduce((a, d) => a + d.totals.cost_usd, 0);
    const t = totals(Math.round(sum * 100) / 100);
    const scale = days === 1 ? 1 : Math.sqrt(days);
    return {
      since: midnight - (days - 1) * D,
      until: NOW,
      days,
      totals: t,
      by_project: PROJECTS.map(([name, w, sess]) => ({
        project: `/Users/demo/code/${name}`,
        name,
        totals: totals(Math.round(sum * w * 100) / 100),
        sessions: Math.max(1, Math.round(sess * scale)),
        last_active: NOW - 60,
      })),
      by_model: MODELS.map(([model, w]) => ({ model, totals: totals(Math.round(sum * w * 100) / 100) })),
      by_day: byDay,
      by_session: [],
    };
  }

  const REPORTS = { 1: report(1), 7: report(7), 30: report(30) };

  function forecasts(fiveHour, sevenDay) {
    const fiveReset = NOW + 2 * H + 10 * 60;
    const weekReset = NOW + 3 * D + 1 * H;
    // At the final 64% the pace reaches 100% at 10:02, 1h 20m before the reset.
    const rate = (100 - 64) / (50 / 60);
    const hits = fiveHour >= 60 ? NOW + Math.round(((100 - fiveHour) / rate) * H) : null;
    return [
      {
        window: "five_hour",
        used_pct: fiveHour,
        resets_at: fiveReset,
        seconds_to_reset: fiveReset - NOW,
        basis: "recent",
        rate_pct_per_hour: rate,
        projected_pct_at_reset: fiveHour + rate * ((fiveReset - NOW) / H),
        hits_limit_at: hits,
      },
      {
        window: "seven_day",
        used_pct: sevenDay,
        resets_at: weekReset,
        seconds_to_reset: weekReset - NOW,
        basis: "recent",
        rate_pct_per_hour: 0.41,
        projected_pct_at_reset: 68,
        hits_limit_at: null,
      },
    ];
  }

  const SESSIONS = [
    ["acme-api", "Fable 5.2", 31, 62000, 0],
    ["web-dashboard", "Opus 5.5", 72, 144000, 12 * 60],
    ["data-pipeline", "Sonnet 5.5", 88, 176000, 48 * 60],
    ["docs-site", "Haiku 5.5", 18, 36000, 2 * H + 5 * 60],
  ];

  function summary(days, fiveHour, sevenDay) {
    const rep = REPORTS[days] || REPORTS[1];
    return {
      generated_at: NOW,
      hook: { state: "installed", command: "~/Applications/Limitlane.app/Contents/MacOS/limitlane statusline --limitlane-hook", chained: null },
      limits: { captured_at: NOW - 12 },
      limits_age_secs: 12,
      forecasts: forecasts(fiveHour, sevenDay),
      extra_windows: [],
      week_since: NOW - 4 * D,
      week_models: [
        { family: "Fable", totals: totals(410), share_pct: 83 },
        { family: "Sonnet", totals: totals(44), share_pct: 9 },
        { family: "Opus", totals: totals(40), share_pct: 8 },
      ],
      burn: { window_minutes: 30, tokens_per_min: 182400, cost_per_hour: 9.84, tokens: 5472000 },
      block_estimate: null,
      today: REPORTS[1].totals,
      report: rep,
      sessions: SESSIONS.map(([name, model, pct, tokens, ago], i) => ({
        session_id: `demo-${i}`,
        name,
        project: `/Users/demo/code/${name}`,
        model,
        context_used_pct: pct,
        context_tokens: tokens,
        context_size: 200000,
        cost_usd: null,
        last_seen: NOW - ago,
        source: "hook",
      })),
      pricing_as_of: "2026-11-01",
    };
  }

  window.LimitlaneDemoData = { NOW, summary };
})();
