# Claude Tokens Counter

A macOS menu bar app that shows how much of your Claude limits you have used, **when you will hit them at your current pace**, and which projects are spending the quota.

It reads only local files. No API keys, no OAuth token, no cookies, no network calls.

> Unofficial project, not affiliated with Anthropic. Status: v0.1, the engine and CLI are tested; the macOS app shell has been compile-checked on Linux but not yet run on a Mac (see [Status](#status)).

## What it shows

- **5-hour and weekly limits** as reported by Claude Code itself, with reset countdown and a pace mark on each bar.
- **Burn-rate forecast**: "At this pace: limit at 14:55, 45m before reset", or "68% at reset" when you are safe. The pace decays while you are idle.
- **Per-project breakdown**: tokens and API-equivalent cost by project, model, session and day, for today, 7 days or 30 days.
- **Context window meter** for every active Claude Code session.
- **Alerts** at 50 / 75 / 90% (configurable), plus one warning per window when the forecast says you will run out before the reset.
- **CLI and export**: the same numbers as tables, JSON or CSV.

## How it gets the data

| Source | What it provides | Notes |
|---|---|---|
| `~/.claude/projects/**/*.jsonl` | Tokens, models, projects, sessions | Claude Code's own session transcripts. Deduplicated by message and request id. |
| Claude Code statusline hook | Real 5-hour / weekly percentages, reset times, context window per session | Claude Code passes this as JSON to the statusline command. Pro and Max plans, after the first response in a session. |

The limits are account-wide, so usage from the desktop app and claude.ai is included in the percentages. They refresh whenever a Claude Code session is active. Without the hook the app still works and shows tokens, cost and an estimated 5-hour window.

The hook is one entry in `~/.claude/settings.json`. If you already have a statusline, it is kept: the hook records the data and then runs your original command, so your terminal looks the same. A backup of the settings file is written next to it, and uninstalling restores the original.

Costs are "API-equivalent": what the same tokens would cost at API list prices (table in `crates/core/src/pricing.json`, overridable in the config). A subscription is not billed per token.

## Install (from source)

Requirements: macOS 11+, Xcode Command Line Tools, Rust 1.82+.

```bash
cargo install tauri-cli --version "^2" --locked

cargo tauri dev      # run in development
cargo tauri build    # produces target/release/bundle/macos/Claude Tokens Counter.app
```

Move the `.app` to `/Applications` **before** enabling live limits, because the hook stores the app's path. If you move it later, open Settings and enable again.

Then click the menu bar icon and choose **Enable live limits**, or from a terminal:

```bash
"/Applications/Claude Tokens Counter.app/Contents/MacOS/claude-tokens-counter" hook install
```

## CLI

The app binary doubles as the CLI. For a standalone `ctc`:

```bash
cargo install --path crates/cli
```

```text
ctc status                      limits, forecast, burn rate, today
ctc projects --days 30          cost per project
ctc daily --days 7              cost per day
ctc models --days 7             cost per model
ctc sessions                    context window of recent sessions
ctc export --days 30            full JSON document
ctc export --format csv --by project|day|model|session
ctc hook install|uninstall|status
```

Add `--json` to `status`, `projects`, `daily`, `models` and `sessions`.

```text
5-hour  █████████████░░░░░░░  64.0%   resets in 2h 09m (Thu 22:00)
        at this pace: limit Thu 21:26 (34m before reset)
weekly  ████████░░░░░░░░░░░░  38.5%   resets in 3d 0h (Sun 20:50)
        at this pace: 68% at reset
```

## Configuration

`~/.claudetokenscounter/config.json` (also editable from the app's Settings):

```json
{
  "thresholds": [50, 75, 90],
  "forecast_alerts": true,
  "notifications": true,
  "refresh_secs": 30,
  "menu_bar": "both",
  "pricing_overrides": []
}
```

`menu_bar` is one of `both`, `five_hour`, `seven_day`, `icon_only`. Environment: `CLAUDE_CONFIG_DIR` (comma-separated) and `CTC_HOME` override the default directories.

## Project layout

```text
crates/core   engine: transcript parsing, pricing, limits store, forecast, alerts, hook installer
crates/cli    ctc command line (also embedded in the app)
src-tauri     menu bar shell: tray icon, popover window, notifications
ui            popover (plain HTML/CSS/JS, no build step)
```

```bash
cargo test                           # engine + CLI (no GUI dependencies)
cargo check -p claude-tokens-counter # the app
```

## How it compares

| Project | Limits source | Forecast | Per-project cost | Context | CLI |
|---|---|---|---|---|---|
| [CodexBar](https://github.com/steipete/CodexBar) | OAuth endpoint, cookies, CLI | no | cost summary | no | yes |
| [tokenmeter](https://github.com/Priyans-hu/tokenmeter) | OAuth token from Keychain | no | yes | no | no |
| [ClaudeBar](https://github.com/BBerthod/ClaudeBar) | local files | no | yes | no | no |
| [usage](https://github.com/aqua5230/usage) | local files | no | yes | no | no |
| [claude-meter](https://github.com/aki77/claude-meter) | OAuth endpoint | weekly only | no | no | no |
| **This project** | statusline hook (no credentials) | 5-hour and weekly | yes | yes | yes |

Based on each project's README in November 2026; check them for the current state.

## Status

Verified on Linux: 44 unit tests, an end-to-end run of the CLI and hook against generated transcripts, totals cross-checked with an independent script, and the popover rendered in a headless browser with real engine output.

Not yet verified, needs a Mac:

- Tray icon rendering and title in the real menu bar, popover position under the icon, hide on blur.
- `ActivationPolicy::Accessory` (no Dock icon). This line is macOS-only and was not compiled on Linux.
- Notifications. In `cargo tauri dev` macOS attributes them to the terminal; use a built `.app` to test.
- Field names in real transcripts and statusline payloads were taken from the Claude Code documentation and other tools, then tested against synthetic data, not against a live Claude Code session.

## Roadmap

- Launch at login, signed and notarized builds, Homebrew cask.
- Rounded, translucent popover (needs `macOSPrivateApi`).
- File watching instead of polling.
- Opt-in OAuth usage endpoint for people who want limits without a running Claude Code session.
- Per-project budget alerts; Codex and Gemini providers.

## License

MIT
