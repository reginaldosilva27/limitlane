//! Menu bar (macOS) and notification-area (Windows) app. The same binary also
//! answers CLI commands (`statusline`, `status`, `export`, ...) so it can serve
//! as Claude Code's statusline hook without shipping a second executable.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use paceline_core::alerts::{self, AlertState};
use paceline_core::config::Config;
use paceline_core::hook::{self, HookStatus, InstallOutcome};
use paceline_core::placement::{self, Screen};
use paceline_core::{icon, Engine, Paths, Summary};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, PhysicalPosition, Position, WebviewWindow, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
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
    engine.summary(days, paceline_core::now(), paceline_core::local_offset())
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

#[derive(serde::Serialize)]
struct AppInfo {
    version: &'static str,
    /// True for `cargo tauri dev` builds, which should not register
    /// themselves at login or be the long-term statusline hook.
    dev_build: bool,
    launch_at_login: bool,
    /// `macos`, `windows`, ...: the popover hides options a platform lacks.
    platform: &'static str,
}

#[tauri::command]
fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        dev_build: cfg!(debug_assertions),
        launch_at_login: app.autolaunch().is_enabled().unwrap_or(false),
        platform: std::env::consts::OS,
    }
}

#[tauri::command]
fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<bool, String> {
    if enabled && cfg!(debug_assertions) {
        return Err("This is a development build. Install the app and turn this on there.".into());
    }
    let auto = app.autolaunch();
    if enabled { auto.enable() } else { auto.disable() }.map_err(err_string)?;
    auto.is_enabled().map_err(err_string)
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

/// A tinted template on macOS; a coloured square on Windows, which neither
/// tints tray icons nor shows text next to them.
fn tray_image(five_hour: Option<f64>, seven_day: Option<f64>) -> Image<'static> {
    if cfg!(target_os = "macos") {
        Image::new_owned(icon::render_meter(five_hour, seven_day), icon::WIDTH, icon::HEIGHT)
    } else {
        Image::new_owned(icon::render_meter_color(five_hour, seven_day), icon::SQUARE, icon::SQUARE)
    }
}

fn update_tray(app: &AppHandle, summary: &Summary, config: &Config) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        eprintln!("[paceline] menu bar item not found");
        return;
    };
    if let Err(e) = tray.set_icon(Some(tray_image(summary.five_hour_pct(), summary.seven_day_pct()))) {
        eprintln!("[paceline] could not set the menu bar icon: {e}");
    }
    if cfg!(target_os = "macos") {
        let _ = tray.set_icon_as_template(true);
        let title = summary.menu_bar_title(config.menu_bar);
        if let Err(e) = tray.set_title(if title.is_empty() { None } else { Some(title) }) {
            eprintln!("[paceline] could not set the menu bar title: {e}");
        }
    }

    let line = |label: &str, v: Option<f64>| match v {
        Some(p) => format!("{label}: {p:.0}%"),
        None => format!("{label}: no data"),
    };
    // On Windows the tooltip is the only place the numbers appear as text.
    let tip = format!("Paceline\n{}\n{}", line("5-hour", summary.five_hour_pct()), line("Weekly", summary.seven_day_pct()));
    let _ = tray.set_tooltip(Some(tip));
}

/// Show the popover next to the tray icon (under it on macOS, above the
/// taskbar on Windows), or hide it if it is open. `icon_pos` and `icon_size`
/// are the tray item's rectangle in physical pixels.
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

    let screens: Vec<Screen> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Screen { x: m.position().x as f64, y: m.position().y as f64, width: m.size().width as f64, height: m.size().height as f64, scale: m.scale_factor() })
        .collect();
    let (win_w, win_h) = window
        .outer_size()
        .ok()
        .zip(window.scale_factor().ok())
        .map(|(size, scale)| (size.width as f64 / scale, size.height as f64 / scale))
        .unwrap_or((384.0, 640.0));
    let (place, position): (_, Position) = if cfg!(target_os = "macos") {
        let p = placement::popover_origin(icon_pos.x, icon_pos.y, icon_size.width, icon_size.height, &screens, win_w, win_h);
        // Points, not pixels: the only unit that is the same on every display.
        (p, LogicalPosition::new(p.x, p.y).into())
    } else {
        // Windows positions windows in physical pixels shared by all displays.
        let p = placement::tray_popover_origin(icon_pos.x, icon_pos.y, icon_size.width, icon_size.height, &screens, win_w, win_h);
        (p, PhysicalPosition::new(p.x.round() as i32, p.y.round() as i32).into())
    };
    if cfg!(debug_assertions) {
        eprintln!("[paceline] icon at {icon_pos:?} size {icon_size:?}; screens {screens:?}; popover {place:?}");
    }

    // Follow the user to whichever Space is active, including full-screen apps.
    let _ = window.set_visible_on_all_workspaces(true);
    let _ = window.set_always_on_top(true);
    let _ = window.set_position(position);
    let _ = window.show();
    // Moving to a display with another pixel density can shift the window;
    // setting the position again once it is on that display settles it.
    let _ = window.set_position(position);
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
    let started = Instant::now();
    let summary = compute(&state);
    update_tray(app, &summary, &config);
    if cfg!(debug_assertions) {
        eprintln!("[paceline] refresh in {:?}: title {:?}, {} responses in range", started.elapsed(), summary.menu_bar_title(config.menu_bar), summary.report.totals.messages);
    }

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

/// The Windows build is a GUI program, so typing `paceline status` in a
/// terminal would print nothing. Borrow the terminal's console in that case.
/// When Claude Code runs the hook it passes pipes, which are left alone.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
    // SAFETY: plain Win32 calls with constant arguments.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() {
    // CLI mode: any argument that is not a macOS launch artefact.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("-psn_")).collect();
    if !args.is_empty() {
        #[cfg(windows)]
        attach_parent_console();
        std::process::exit(paceline_cli::run(args));
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
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(state)
        .invoke_handler(tauri::generate_handler![get_summary, get_config, set_config, hook_install, hook_uninstall, hook_status, app_info, set_launch_at_login, hide_window, quit])
        .setup(|app| {
            // Menu bar only: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
            let refresh_item = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Paceline", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &refresh_item, &quit_item])?;

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(tray_image(None, None))
                .icon_as_template(cfg!(target_os = "macos"))
                // Placeholder until the first refresh, so the item is easy to spot.
                .title("Paceline")
                .tooltip("Paceline")
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
                        // The rectangle arrives in physical pixels; the factor is
                        // only used if a platform ever reports points instead.
                        toggle_popover(tray.app_handle(), rect.position.to_physical(1.0), rect.size.to_physical(1.0));
                    }
                })
                .build(app)?;
            eprintln!("[paceline] menu bar item created");

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
        .expect("failed to start Paceline");
}
