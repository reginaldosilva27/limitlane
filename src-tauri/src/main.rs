//! Menu bar app. The same binary also answers CLI commands (`statusline`,
//! `status`, `export`, ...) so it can serve as Claude Code's statusline hook
//! without shipping a second executable.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ctc_core::alerts::{self, AlertState};
use ctc_core::config::Config;
use ctc_core::hook::{self, HookStatus, InstallOutcome};
use ctc_core::{icon, Engine, Paths, Summary};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow, WindowEvent};
use tauri_plugin_notification::NotificationExt;

const TRAY_ID: &str = "main";
const WINDOW: &str = "main";
/// Refresh interval while the popover is open.
const VISIBLE_REFRESH: Duration = Duration::from_secs(5);

struct AppState {
    paths: Paths,
    engine: Mutex<Engine>,
    alerts: Mutex<AlertState>,
    /// Report range the popover is showing.
    range_days: AtomicU32,
    refresh_now: AtomicBool,
    /// When the popover was last hidden, to tell "click to close" from "click to open".
    hidden_at: Mutex<Option<Instant>>,
}

fn compute(state: &AppState) -> Summary {
    let days = state.range_days.load(Ordering::Relaxed).max(1);
    let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    engine.summary(days, ctc_core::now(), ctc_core::local_offset())
}

fn err_string(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

// ---------- commands called from the popover ----------

#[tauri::command]
fn get_summary(state: tauri::State<AppState>, range_days: u32) -> Summary {
    state.range_days.store(range_days.clamp(1, 365), Ordering::Relaxed);
    compute(&state)
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Config {
    Config::load(&state.paths)
}

#[tauri::command]
fn set_config(state: tauri::State<AppState>, config: Config) -> Result<Config, String> {
    let config = config.sanitized();
    config.save(&state.paths).map_err(err_string)?;
    state.refresh_now.store(true, Ordering::Relaxed);
    Ok(config)
}

#[tauri::command]
fn hook_install(state: tauri::State<AppState>) -> Result<InstallOutcome, String> {
    let exe = std::env::current_exe().map_err(err_string)?;
    let out = hook::install(&state.paths, &exe).map_err(err_string)?;
    state.refresh_now.store(true, Ordering::Relaxed);
    Ok(out)
}

#[tauri::command]
fn hook_uninstall(state: tauri::State<AppState>) -> Result<bool, String> {
    let removed = hook::uninstall(&state.paths).map_err(err_string)?;
    state.refresh_now.store(true, Ordering::Relaxed);
    Ok(removed)
}

#[tauri::command]
fn hook_status(state: tauri::State<AppState>) -> HookStatus {
    hook::status(&state.paths)
}

#[tauri::command]
fn hide_window(window: WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

// ---------- tray ----------

fn update_tray(app: &AppHandle, summary: &Summary, config: &Config) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let pixels = icon::render_meter(summary.five_hour_pct(), summary.seven_day_pct());
    let _ = tray.set_icon(Some(Image::new_owned(pixels, icon::WIDTH, icon::HEIGHT)));
    let _ = tray.set_icon_as_template(true);
    let title = summary.menu_bar_title(config.menu_bar);
    let _ = tray.set_title(if title.is_empty() { None } else { Some(title) });

    let line = |label: &str, v: Option<f64>| match v {
        Some(p) => format!("{label}: {p:.0}%"),
        None => format!("{label}: no data"),
    };
    let tip = format!("{}\n{}", line("5-hour", summary.five_hour_pct()), line("Weekly", summary.seven_day_pct()));
    let _ = tray.set_tooltip(Some(tip));
}

/// Show the popover centred under the tray icon, or hide it if it is open.
fn toggle_popover(app: &AppHandle, icon_pos: PhysicalPosition<f64>, icon_size: tauri::PhysicalSize<f64>) {
    let Some(window) = app.get_webview_window(WINDOW) else { return };
    let state = app.state::<AppState>();
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    // Clicking the icon while the popover is open first blurs (and hides) it;
    // do not reopen it on that same click.
    let just_hidden = state.hidden_at.lock().ok().and_then(|h| *h).is_some_and(|t| t.elapsed() < Duration::from_millis(250));
    if just_hidden {
        return;
    }
    if let Ok(size) = window.outer_size() {
        let mut x = icon_pos.x + icon_size.width / 2.0 - size.width as f64 / 2.0;
        let y = icon_pos.y + icon_size.height + 4.0;
        // Keep the popover on the screen that holds the icon.
        if let Ok(Some(monitor)) = app.monitor_from_point(icon_pos.x, icon_pos.y) {
            let left = monitor.position().x as f64 + 8.0;
            let right = monitor.position().x as f64 + monitor.size().width as f64 - size.width as f64 - 8.0;
            x = x.clamp(left, right.max(left));
        }
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
    let _ = window.set_focus();
    state.refresh_now.store(true, Ordering::Relaxed);
}

fn show_centered(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.center();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// ---------- background refresh ----------

fn refresh(app: &AppHandle) {
    let state = app.state::<AppState>();
    let config = Config::load(&state.paths);
    let summary = compute(&state);
    update_tray(app, &summary, &config);

    let fired = {
        let mut st = state.alerts.lock().unwrap_or_else(|e| e.into_inner());
        let before = st.clone();
        let fired = alerts::evaluate(&mut st, &summary.forecasts, &config, summary.generated_at);
        if *st != before {
            let _ = st.save(&state.paths);
        }
        fired
    };
    for alert in fired {
        let _ = app.notification().builder().title(alert.title).body(alert.body).show();
    }
    let _ = app.emit("summary", &summary);
}

fn spawn_refresh_loop(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<Instant> = None;
        loop {
            let state = app.state::<AppState>();
            let visible = app.get_webview_window(WINDOW).and_then(|w| w.is_visible().ok()).unwrap_or(false);
            let every = if visible { VISIBLE_REFRESH } else { Duration::from_secs(Config::load(&state.paths).refresh_secs) };
            let forced = state.refresh_now.swap(false, Ordering::Relaxed);
            if forced || last.is_none_or(|t| t.elapsed() >= every) {
                refresh(&app);
                last = Some(Instant::now());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

fn main() {
    // CLI mode: any argument that is not a macOS launch artefact.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn_")).collect();
    if !args.is_empty() {
        std::process::exit(ctc_cli::run(args));
    }

    let paths = Paths::from_env();
    let state = AppState {
        engine: Mutex::new(Engine::new(paths.clone())),
        alerts: Mutex::new(AlertState::load(&paths)),
        paths,
        range_days: AtomicU32::new(1),
        refresh_now: AtomicBool::new(false),
        hidden_at: Mutex::new(None),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![get_summary, get_config, set_config, hook_install, hook_uninstall, hook_status, hide_window, quit])
        .setup(|app| {
            // Menu bar only: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
            let refresh_item = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Claude Tokens Counter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &refresh_item, &quit_item])?;

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(Image::new_owned(icon::render_meter(None, None), icon::WIDTH, icon::HEIGHT))
                .icon_as_template(true)
                .tooltip("Claude Tokens Counter")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_centered(app),
                    "refresh" => app.state::<AppState>().refresh_now.store(true, Ordering::Relaxed),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                        let app = tray.app_handle();
                        let scale = app.get_webview_window(WINDOW).and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
                        toggle_popover(app, rect.position.to_physical(scale), rect.size.to_physical(scale));
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window(WINDOW) {
                let handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        if let Some(w) = handle.get_webview_window(WINDOW) {
                            let _ = w.hide();
                        }
                        if let Ok(mut hidden) = handle.state::<AppState>().hidden_at.lock() {
                            *hidden = Some(Instant::now());
                        }
                    }
                });
            }

            spawn_refresh_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start Claude Tokens Counter");
}
