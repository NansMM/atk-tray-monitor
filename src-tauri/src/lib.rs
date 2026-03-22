use arboard::Clipboard;
use hidapi::HidApi;
use libatk_rs::prelude::{Command, CommandDescriptor, CommandId, Device};
use serde::Serialize;
use std::env;
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use std::sync::mpsc;
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::PageLoadEvent,
    AppHandle, Emitter, Listener, LogicalSize, Manager, PhysicalPosition, Position, Size, State,
    Url, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_store::StoreExt;
use tauri_plugin_updater::UpdaterExt;

const DEVICE_KEYWORDS: [&str; 4] = ["atk", "vxe", "leviatan", "mad r"];
const DEFAULT_DEVICE_LABEL: &str = "ATK device";
const GENERIC_MOUSE_LABEL: &str = "ATK mouse";
const KNOWN_DEVICE_IDS: [(u16, u16); 2] = [(0x373B, 0x1031), (0x373B, 0x105B)];
const KNOWN_VENDOR_IDS: [u16; 1] = [0x373B];
const ATK_USAGE_PAGE: u16 = 0xFF00;
const ATK_USAGE: u16 = 0x0001;
const AUTOSTART_FLAG: &str = "--autostart";
const SETTINGS_FILE: &str = "settings.json";
const START_MINIMIZED_KEY: &str = "startMinimizedOnAutostart";
const LOW_BATTERY_NOTIFICATIONS_KEY: &str = "lowBatteryNotifications";
const LOW_BATTERY_THRESHOLD_KEY: &str = "lowBatteryThreshold";
const LANGUAGE_KEY: &str = "language";
const SETTINGS_UPDATED_EVENT: &str = "settings-updated";
const BATTERY_REFRESH_INTERVAL_SECONDS: u64 = 20;
const SUPPORTED_LANGUAGES: [&str; 5] = ["de", "en", "es", "fr", "it"];
const TRAY_ICON_SIZE: u32 = 32;
const MAIN_WINDOW_WIDTH: f64 = 420.0;
const EMBEDDED_UPDATER_PUBKEY: &str = include_str!("../updater-public.key");
const DEFAULT_GITHUB_RELEASES_UPDATE_ENDPOINT: &str =
    "https://github.com/NansMM/atk-tray-monitor/releases/latest/download/latest.json";

struct TrayMenuLabels {
    open: &'static str,
    hide: &'static str,
    refresh: &'static str,
    copy_diagnostics: &'static str,
    version: &'static str,
    launch_on_startup: &'static str,
    start_minimized: &'static str,
    low_battery_notifications: &'static str,
    language: &'static str,
    threshold: &'static str,
    settings: &'static str,
    quit: &'static str,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatterySnapshot {
    level: u8,
    charge: u8,
    voltage: f32,
    is_charging: bool,
    connected: bool,
    status: String,
    device_label: String,
    updated_at: String,
    source: String,
    diagnostics: BatteryDiagnostics,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceCandidate {
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    label: String,
    score: u8,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatteryDiagnostics {
    selected_candidate: Option<String>,
    candidate_count: usize,
    candidates: Vec<DeviceCandidate>,
    last_error: Option<String>,
    backend: String,
}

struct AppState {
    latest_snapshot: Mutex<BatterySnapshot>,
    scan_in_progress: AtomicBool,
}

struct GetBatteryStatus;

impl CommandDescriptor for GetBatteryStatus {}

#[tauri::command]
fn refresh_battery_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BatterySnapshot, String> {
    refresh_snapshot(&app, &state)
}

#[tauri::command]
fn hide_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Fenetre principale introuvable.".to_string())?;

    window.hide().map_err(|error| error.to_string())
}

#[tauri::command]
fn show_window(app: AppHandle) -> Result<(), String> {
    show_main_window(&app)
}

#[tauri::command]
fn fit_window_to_content(app: AppHandle, content_height: f64) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Fenetre principale introuvable.".to_string())?;
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let target_height = content_height.max(1.0);
    let target_physical_size = (MAIN_WINDOW_WIDTH * scale_factor, target_height * scale_factor);

    window
        .set_size(Size::Logical(LogicalSize::new(MAIN_WINDOW_WIDTH, target_height)))
        .map_err(|error| error.to_string())?;

    position_window_near_tray(&app, &window, None, Some(target_physical_size))
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

#[tauri::command]
async fn install_available_update(app: AppHandle) -> Result<Option<String>, String> {
    let config = load_updater_config().ok_or_else(|| "Updater disabled".to_string())?;
    let update = app
        .updater_builder()
        .pubkey(config.pubkey)
        .endpoints(config.endpoints)
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;

    let Some(update) = update else {
        return Ok(None);
    };

    let version = update.version.clone();

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;

    Ok(Some(version))
}

pub fn run() {
    let launch_args: Vec<String> = std::env::args().collect();
    let launched_from_autostart = launch_args.iter().any(|arg| arg == AUTOSTART_FLAG);
    let show_after_first_page_load = Arc::new(AtomicBool::new(false));

    tauri::Builder::default()
        .on_page_load({
            let show_after_first_page_load = Arc::clone(&show_after_first_page_load);

            move |webview, payload| {
                if payload.event() == PageLoadEvent::Finished
                    && show_after_first_page_load.swap(false, Ordering::SeqCst)
                {
                    let _ = show_main_window(&webview.app_handle());
                }
            }
        })
        .plugin(
            tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                let _ = show_main_window(app);
            }),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args([AUTOSTART_FLAG])
                .app_name("ATK Tray Monitor")
                .build(),
        )
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(AppState {
            latest_snapshot: Mutex::new(disconnected_snapshot(
                "Initialisation de la detection ATK...",
                DEFAULT_DEVICE_LABEL,
                "bootstrap",
                Vec::new(),
            )),
            scan_in_progress: AtomicBool::new(false),
        })
        .setup(move |app| {
            if let Some(config) = load_updater_config() {
                app.handle().plugin(
                    tauri_plugin_updater::Builder::new()
                        .pubkey(config.pubkey)
                        .build(),
                )?;
            }

            build_tray(app)?;
            ensure_default_settings(app)?;
            let start_minimized_on_autostart =
                load_bool_setting(app.handle(), START_MINIMIZED_KEY, true);
            let should_show_on_launch = !(launched_from_autostart && start_minimized_on_autostart);

            show_after_first_page_load.store(should_show_on_launch, Ordering::SeqCst);

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_icon(render_tray_battery_icon(None));
                let handle = app.handle().clone();
                window.on_window_event(move |event| {
                    match event {
                        WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            if let Some(main_window) = handle.get_webview_window("main") {
                                let _ = main_window.hide();
                            }
                        }
                        WindowEvent::Focused(false) => {
                            if let Some(main_window) = handle.get_webview_window("main") {
                                let _ = main_window.hide();
                            }
                        }
                        _ => {}
                    }
                });
            }

            let app_handle = app.handle().clone();
            thread::spawn(move || loop {
                let state = app_handle.state::<AppState>();
                let _ = refresh_snapshot(&app_handle, &state);
                thread::sleep(Duration::from_secs(BATTERY_REFRESH_INTERVAL_SECONDS));
            });

            let tray_update_handle = app.handle().clone();
            app.listen("snapshot-refreshed", move |_| {
                let state = tray_update_handle.state::<AppState>();
                let snapshot = state.latest_snapshot.lock().ok().map(|s| s.clone());
                if let Some(snapshot) = snapshot {
                    update_tray_visuals(&tray_update_handle, &snapshot);
                }
            });

            let state = app.handle().state::<AppState>();
            let _ = refresh_snapshot(&app.handle().clone(), &state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            refresh_battery_status,
            hide_window,
            show_window,
            fit_window_to_content,
            restart_app,
            install_available_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

struct UpdaterConfig {
    pubkey: String,
    endpoints: Vec<Url>,
}

fn load_updater_config() -> Option<UpdaterConfig> {
    let pubkey = option_env!("TAURI_UPDATER_PUBKEY")
        .map(str::to_owned)
        .or_else(|| env::var("TAURI_UPDATER_PUBKEY").ok())
        .or_else(|| {
            let embedded_pubkey = EMBEDDED_UPDATER_PUBKEY.trim();
            (!embedded_pubkey.is_empty()).then(|| embedded_pubkey.to_owned())
        })?;
    let endpoints_source = option_env!("TAURI_UPDATER_ENDPOINTS")
        .map(str::to_owned)
        .or_else(|| env::var("TAURI_UPDATER_ENDPOINTS").ok())
        .unwrap_or_else(|| DEFAULT_GITHUB_RELEASES_UPDATE_ENDPOINT.to_string());
    let endpoints = endpoints_source
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Url::parse)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;

    if endpoints.is_empty() {
        return None;
    }

    Some(UpdaterConfig { pubkey, endpoints })
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let current_language = load_string_setting(app.handle(), LANGUAGE_KEY)
        .filter(|value| SUPPORTED_LANGUAGES.contains(&value.as_str()))
        .unwrap_or_else(|| "en".to_string());
    let labels = tray_menu_labels(&current_language);
    let app_version = app.package_info().version.to_string();
    let open = MenuItem::with_id(app, "open", labels.open, true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", labels.hide, true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", labels.refresh, true, None::<&str>)?;
    let copy_diagnostics = MenuItem::with_id(
        app,
        "copy_diagnostics",
        labels.copy_diagnostics,
        true,
        None::<&str>,
    )?;
    let version = MenuItem::with_id(
        app,
        "version",
        format_version_menu_text(labels.version, &app_version),
        false,
        None::<&str>,
    )?;
    let launch_on_startup = CheckMenuItem::with_id(
        app,
        "settings_launch_on_startup",
        labels.launch_on_startup,
        true,
        autostart_enabled(app.handle()),
        None::<&str>,
    )?;
    let start_minimized = CheckMenuItem::with_id(
        app,
        "settings_start_minimized",
        labels.start_minimized,
        true,
        load_bool_setting(app.handle(), START_MINIMIZED_KEY, true),
        None::<&str>,
    )?;
    let low_battery_notifications = CheckMenuItem::with_id(
        app,
        "settings_low_battery_notifications",
        labels.low_battery_notifications,
        true,
        load_bool_setting(app.handle(), LOW_BATTERY_NOTIFICATIONS_KEY, true),
        None::<&str>,
    )?;
    let threshold_values = [5_u8, 10, 15, 20, 25, 30, 40, 50];
    let current_threshold = load_u8_setting(app.handle(), LOW_BATTERY_THRESHOLD_KEY, 20);
    let language_de = CheckMenuItem::with_id(
        app,
        "settings_language_de",
        "Deutsch",
        true,
        current_language == "de",
        None::<&str>,
    )?;
    let language_en = CheckMenuItem::with_id(
        app,
        "settings_language_en",
        "English",
        true,
        current_language == "en",
        None::<&str>,
    )?;
    let language_es = CheckMenuItem::with_id(
        app,
        "settings_language_es",
        "Espanol",
        true,
        current_language == "es",
        None::<&str>,
    )?;
    let language_fr = CheckMenuItem::with_id(
        app,
        "settings_language_fr",
        "Francais",
        true,
        current_language == "fr",
        None::<&str>,
    )?;
    let language_it = CheckMenuItem::with_id(
        app,
        "settings_language_it",
        "Italiano",
        true,
        current_language == "it",
        None::<&str>,
    )?;
    let language_submenu = Submenu::with_items(
        app,
        labels.language,
        true,
        &[&language_de, &language_en, &language_es, &language_fr, &language_it],
    )?;
    let threshold_items = threshold_values
        .iter()
        .map(|value| {
            CheckMenuItem::with_id(
                app,
                format!("settings_threshold_{value}"),
                format!("{value}%"),
                true,
                *value == current_threshold,
                None::<&str>,
            )
            .map(|item| (*value, item))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let threshold_submenu_items = threshold_items
        .iter()
        .map(|(_, item)| item as &dyn tauri::menu::IsMenuItem<_>)
        .collect::<Vec<_>>();
    let threshold_submenu = Submenu::with_items(app, labels.threshold, true, &threshold_submenu_items)?;
    let settings_submenu = Submenu::with_items(
        app,
        labels.settings,
        true,
        &[
            &launch_on_startup,
            &start_minimized,
            &low_battery_notifications,
            &PredefinedMenuItem::separator(app)?,
            &language_submenu,
            &PredefinedMenuItem::separator(app)?,
            &threshold_submenu,
        ],
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &hide, &refresh, &copy_diagnostics, &settings_submenu, &separator, &version, &quit],
    )?;

    let tray_builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip("ATK Tray Monitor")
        .show_menu_on_left_click(false)
        .icon(render_tray_battery_icon(None));

    tray_builder
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let _ = toggle_window(&app, Some(rect));
            }
        })
        .on_menu_event({
            let open = open.clone();
            let hide = hide.clone();
            let refresh = refresh.clone();
            let copy_diagnostics = copy_diagnostics.clone();
            let version = version.clone();
            let launch_on_startup = launch_on_startup.clone();
            let start_minimized = start_minimized.clone();
            let low_battery_notifications = low_battery_notifications.clone();
            let language_submenu = language_submenu.clone();
            let threshold_submenu = threshold_submenu.clone();
            let settings_submenu = settings_submenu.clone();
            let quit = quit.clone();
            let app_version = app_version.clone();
            let language_de = language_de.clone();
            let language_en = language_en.clone();
            let language_es = language_es.clone();
            let language_fr = language_fr.clone();
            let language_it = language_it.clone();
            let threshold_items = threshold_items.clone();

            move |app, event| match event.id().as_ref() {
                "open" => {
                    let _ = show_main_window(app);
                }
                "hide" => {
                    let _ = hide_main_window(app);
                }
                "refresh" => {
                    let handle = app.clone();
                    thread::spawn(move || {
                        let state = handle.state::<AppState>();
                        let _ = refresh_snapshot(&handle, &state);
                    });
                }
                "copy_diagnostics" => {
                    let state = app.state::<AppState>();
                    let snapshot = state
                        .latest_snapshot
                        .lock()
                        .map(|snapshot| snapshot.clone())
                        .ok();

                    if let Some(snapshot) = snapshot {
                        let _ = copy_text_to_clipboard(&build_diagnostics_report(&snapshot));
                    }
                }
                "settings_launch_on_startup" => {
                    let next_enabled = !autostart_enabled(app);
                    if set_autostart_enabled(app, next_enabled).is_ok() {
                        let _ = launch_on_startup.set_checked(next_enabled);
                        emit_settings_updated(app);
                    }
                }
                "settings_start_minimized" => {
                    let next_enabled = !load_bool_setting(app, START_MINIMIZED_KEY, true);
                    if save_bool_setting(app, START_MINIMIZED_KEY, next_enabled).is_ok() {
                        let _ = start_minimized.set_checked(next_enabled);
                        emit_settings_updated(app);
                    }
                }
                "settings_low_battery_notifications" => {
                    let next_enabled = !load_bool_setting(app, LOW_BATTERY_NOTIFICATIONS_KEY, true);
                    if save_bool_setting(app, LOW_BATTERY_NOTIFICATIONS_KEY, next_enabled).is_ok() {
                        let _ = low_battery_notifications.set_checked(next_enabled);
                        emit_settings_updated(app);
                    }
                }
                "settings_language_de" => {
                    if save_string_setting(app, LANGUAGE_KEY, "de").is_ok() {
                        set_language_menu_checked(
                            &language_de,
                            &language_en,
                            &language_es,
                            &language_fr,
                            &language_it,
                            "de",
                        );
                        apply_tray_menu_language(
                            &open,
                            &hide,
                            &refresh,
                            &copy_diagnostics,
                            &version,
                            &launch_on_startup,
                            &start_minimized,
                            &low_battery_notifications,
                            &language_submenu,
                            &threshold_submenu,
                            &settings_submenu,
                            &quit,
                            &app_version,
                            "de",
                        );
                        emit_settings_updated(app);
                    }
                }
                "settings_language_en" => {
                    if save_string_setting(app, LANGUAGE_KEY, "en").is_ok() {
                        set_language_menu_checked(
                            &language_de,
                            &language_en,
                            &language_es,
                            &language_fr,
                            &language_it,
                            "en",
                        );
                        apply_tray_menu_language(
                            &open,
                            &hide,
                            &refresh,
                            &copy_diagnostics,
                            &version,
                            &launch_on_startup,
                            &start_minimized,
                            &low_battery_notifications,
                            &language_submenu,
                            &threshold_submenu,
                            &settings_submenu,
                            &quit,
                            &app_version,
                            "en",
                        );
                        emit_settings_updated(app);
                    }
                }
                "settings_language_es" => {
                    if save_string_setting(app, LANGUAGE_KEY, "es").is_ok() {
                        set_language_menu_checked(
                            &language_de,
                            &language_en,
                            &language_es,
                            &language_fr,
                            &language_it,
                            "es",
                        );
                        apply_tray_menu_language(
                            &open,
                            &hide,
                            &refresh,
                            &copy_diagnostics,
                            &version,
                            &launch_on_startup,
                            &start_minimized,
                            &low_battery_notifications,
                            &language_submenu,
                            &threshold_submenu,
                            &settings_submenu,
                            &quit,
                            &app_version,
                            "es",
                        );
                        emit_settings_updated(app);
                    }
                }
                "settings_language_fr" => {
                    if save_string_setting(app, LANGUAGE_KEY, "fr").is_ok() {
                        set_language_menu_checked(
                            &language_de,
                            &language_en,
                            &language_es,
                            &language_fr,
                            &language_it,
                            "fr",
                        );
                        apply_tray_menu_language(
                            &open,
                            &hide,
                            &refresh,
                            &copy_diagnostics,
                            &version,
                            &launch_on_startup,
                            &start_minimized,
                            &low_battery_notifications,
                            &language_submenu,
                            &threshold_submenu,
                            &settings_submenu,
                            &quit,
                            &app_version,
                            "fr",
                        );
                        emit_settings_updated(app);
                    }
                }
                "settings_language_it" => {
                    if save_string_setting(app, LANGUAGE_KEY, "it").is_ok() {
                        set_language_menu_checked(
                            &language_de,
                            &language_en,
                            &language_es,
                            &language_fr,
                            &language_it,
                            "it",
                        );
                        apply_tray_menu_language(
                            &open,
                            &hide,
                            &refresh,
                            &copy_diagnostics,
                            &version,
                            &launch_on_startup,
                            &start_minimized,
                            &low_battery_notifications,
                            &language_submenu,
                            &threshold_submenu,
                            &settings_submenu,
                            &quit,
                            &app_version,
                            "it",
                        );
                        emit_settings_updated(app);
                    }
                }
                menu_id if menu_id.starts_with("settings_threshold_") => {
                    if let Some((selected_value, _)) = threshold_items
                        .iter()
                        .find(|(_, item)| item.id().as_ref() == menu_id)
                    {
                        if save_u8_setting(app, LOW_BATTERY_THRESHOLD_KEY, *selected_value).is_ok() {
                            for (value, item) in &threshold_items {
                                let _ = item.set_checked(*value == *selected_value);
                            }
                            emit_settings_updated(app);
                        }
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

fn toggle_window(app: &AppHandle, tray_rect: Option<tauri::Rect>) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Fenetre principale introuvable.".to_string())?;

    if window.is_visible().map_err(|error| error.to_string())? {
        window.hide().map_err(|error| error.to_string())
    } else {
        position_window_near_tray(app, &window, tray_rect, None)?;
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())
    }
}

fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Fenetre principale introuvable.".to_string())?;

    position_window_near_tray(app, &window, None, None)?;
    window.show().map_err(|error| error.to_string())?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

fn hide_main_window(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Fenetre principale introuvable.".to_string())?;

    window.hide().map_err(|error| error.to_string())
}

fn position_window_near_tray(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    tray_rect: Option<tauri::Rect>,
    window_size_override: Option<(f64, f64)>,
) -> Result<(), String> {
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let tray_rect = match tray_rect {
        Some(rect) => Some(rect),
        None => app
            .tray_by_id("main")
            .and_then(|tray| tray.rect().ok().flatten()),
    };

    let Some(tray_rect) = tray_rect else {
        return Ok(());
    };

    let tray_x = position_to_physical_x(tray_rect.position, scale_factor);
    let tray_y = position_to_physical_y(tray_rect.position, scale_factor);
    let tray_width = size_to_physical_width(tray_rect.size, scale_factor);
    let tray_height = size_to_physical_height(tray_rect.size, scale_factor);
    let (window_width, window_height) = match window_size_override {
        Some(size) => size,
        None => {
            let window_size = window.outer_size().map_err(|error| error.to_string())?;
            (window_size.width as f64, window_size.height as f64)
        }
    };
    let margin = 12.0;
    let mut target_x = tray_x + (tray_width / 2.0) - (window_width / 2.0);
    let mut target_y = if tray_y > window_height + margin {
        tray_y - window_height - margin
    } else {
        tray_y + tray_height + margin
    };

    if let Ok(Some(monitor)) = window.current_monitor() {
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        let min_x = monitor_position.x as f64;
        let min_y = monitor_position.y as f64;
        let max_x = (monitor_position.x + monitor_size.width as i32) as f64 - window_width;
        let max_y = (monitor_position.y + monitor_size.height as i32) as f64 - window_height;

        target_x = target_x.clamp(min_x, max_x.max(min_x));
        target_y = target_y.clamp(min_y, max_y.max(min_y));
    }

    window
        .set_position(Position::Physical(PhysicalPosition::new(
            target_x.round() as i32,
            target_y.round() as i32,
        )))
        .map_err(|error| error.to_string())
}

fn position_to_physical_x(position: Position, scale_factor: f64) -> f64 {
    match position {
        Position::Physical(position) => position.x as f64,
        Position::Logical(position) => position.x * scale_factor,
    }
}

fn position_to_physical_y(position: Position, scale_factor: f64) -> f64 {
    match position {
        Position::Physical(position) => position.y as f64,
        Position::Logical(position) => position.y * scale_factor,
    }
}

fn size_to_physical_width(size: tauri::Size, scale_factor: f64) -> f64 {
    match size {
        tauri::Size::Physical(size) => size.width as f64,
        tauri::Size::Logical(size) => size.width * scale_factor,
    }
}

fn size_to_physical_height(size: tauri::Size, scale_factor: f64) -> f64 {
    match size {
        tauri::Size::Physical(size) => size.height as f64,
        tauri::Size::Logical(size) => size.height * scale_factor,
    }
}

const HID_SCAN_TIMEOUT_SECONDS: u64 = 10;

fn refresh_snapshot(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<BatterySnapshot, String> {
    if state.scan_in_progress.swap(true, Ordering::SeqCst) {
        let latest = state
            .latest_snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| "Scan deja en cours.".to_string())?;
        return Ok(latest);
    }

    let scan_result = run_hid_scan_with_timeout();

    let mut snapshot = match scan_result {
        Ok(inner) => inner,
        Err(error) => disconnected_snapshot(&error, DEFAULT_DEVICE_LABEL, "hid-scan", Vec::new()),
    };

    {
        let mut latest_snapshot = state.latest_snapshot.lock().map_err(|_| {
            state.scan_in_progress.store(false, Ordering::SeqCst);
            "Impossible de verrouiller l'etat de batterie pour la mise a jour.".to_string()
        })?;
        normalize_snapshot_level(&mut snapshot, &latest_snapshot);
        *latest_snapshot = snapshot.clone();
    }

    state.scan_in_progress.store(false, Ordering::SeqCst);

    let _ = app.emit("battery-updated", &snapshot);
    let _ = app.emit("snapshot-refreshed", ());

    Ok(snapshot)
}

fn run_hid_scan_with_timeout() -> Result<BatterySnapshot, String> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = run_hid_scan();
        let _ = tx.send(result);
    });

    rx.recv_timeout(Duration::from_secs(HID_SCAN_TIMEOUT_SECONDS))
        .map_err(|_| "Le scan HID a depasse le delai d'attente.".to_string())?
}

fn run_hid_scan() -> Result<BatterySnapshot, String> {
    match locate_candidates() {
        Ok(candidates) => {
            let diagnostics_candidates = candidates.clone();

            match query_first_working_candidate(candidates) {
                Ok(snapshot) => Ok(enrich_snapshot_with_diagnostics(
                    snapshot,
                    diagnostics_candidates,
                    None,
                )),
                Err(error) => Ok(disconnected_snapshot(
                    &error,
                    DEFAULT_DEVICE_LABEL,
                    "hid-scan",
                    diagnostics_candidates,
                )),
            }
        }
        Err(error) => Ok(disconnected_snapshot(&error, DEFAULT_DEVICE_LABEL, "hid-scan", Vec::new())),
    }
}

fn normalize_snapshot_level(snapshot: &mut BatterySnapshot, previous_snapshot: &BatterySnapshot) {
    if !snapshot.connected || !snapshot.is_charging || !previous_snapshot.connected {
        return;
    }

    if snapshot.device_label != previous_snapshot.device_label {
        return;
    }

    if !snapshots_are_recent(previous_snapshot, snapshot, 15 * 60) {
        return;
    }

    let halved_level = snapshot.level / 2;
    let jump = snapshot.level.saturating_sub(previous_snapshot.level);
    let half_matches_previous = previous_snapshot.level.abs_diff(halved_level) <= 10;

    if jump >= 30 && half_matches_previous {
        snapshot.level = halved_level;
        snapshot.status = format!(
            "{} Niveau normalise pendant la charge.",
            snapshot.status
        );
    }
}

fn snapshots_are_recent(
    previous_snapshot: &BatterySnapshot,
    snapshot: &BatterySnapshot,
    max_gap_seconds: u64,
) -> bool {
    let previous_time = previous_snapshot.updated_at.parse::<u64>().ok();
    let current_time = snapshot.updated_at.parse::<u64>().ok();

    match (previous_time, current_time) {
        (Some(previous_time), Some(current_time)) => current_time.saturating_sub(previous_time) <= max_gap_seconds,
        _ => false,
    }
}

fn locate_candidates() -> Result<Vec<DeviceCandidate>, String> {
    let hid_api = HidApi::new().map_err(|error| error.to_string())?;
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for device in hid_api.device_list() {
        let manufacturer = device.manufacturer_string().unwrap_or_default().to_lowercase();
        let product = device.product_string().unwrap_or_default().to_lowercase();
        let combined = format!("{manufacturer} {product}");
        let vendor_id = device.vendor_id();
        let product_id = device.product_id();
        let usage_page = device.usage_page();
        let usage = device.usage();
        let known_id = KNOWN_DEVICE_IDS.contains(&(vendor_id, product_id));
        let known_vendor = KNOWN_VENDOR_IDS.contains(&vendor_id);
        let keyword_match = DEVICE_KEYWORDS.iter().any(|keyword| combined.contains(keyword));
        let protocol_shape_match = usage_page == ATK_USAGE_PAGE && usage == ATK_USAGE;

        if !(known_id || known_vendor || keyword_match || protocol_shape_match) {
            continue;
        }

        if !seen.insert((vendor_id, product_id, usage_page, usage)) {
            continue;
        }

        let score = score_candidate(known_id, known_vendor, usage_page, usage, &combined);

        let label = format!(
            "{} {} [{:04X}:{:04X} u{:04X}:{:04X}]",
            device
                .manufacturer_string()
                .unwrap_or(if known_id || known_vendor { "ATK" } else { "HID" }),
            device
                .product_string()
                .unwrap_or(if known_id || known_vendor { "Device" } else { "Device" }),
            vendor_id,
            product_id,
            usage_page,
            usage
        )
        .trim()
        .to_string();

        candidates.push(DeviceCandidate {
            vendor_id,
            product_id,
            usage_page,
            usage,
            label,
            score,
        });
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then(left.vendor_id.cmp(&right.vendor_id))
            .then(left.product_id.cmp(&right.product_id))
            .then(left.usage_page.cmp(&right.usage_page))
            .then(left.usage.cmp(&right.usage))
    });

    if candidates.is_empty() {
        Err("Aucun peripherique ATK compatible detecte sur le bus HID. Branchez le dongle ou le cable, puis relancez le diagnostic pour inspecter les interfaces exposees.".to_string())
    } else {
        Ok(candidates)
    }
}

fn query_first_working_candidate(
    candidates: Vec<DeviceCandidate>,
) -> Result<BatterySnapshot, String> {
    let mut errors = Vec::new();

    for candidate in candidates {
        match query_device(&candidate) {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => errors.push(error),
        }
    }

    if errors.is_empty() {
        Err("Aucun candidat compatible n'a repondu.".to_string())
    } else {
        Err(errors.join(" | "))
    }
}

fn query_device(candidate: &DeviceCandidate) -> Result<BatterySnapshot, String> {
    let device = Device::new(
        candidate.vendor_id,
        candidate.product_id,
        candidate.usage_page,
        candidate.usage,
    )
    .map_err(|error| format!("{}: {}", candidate.label, error))?;

    let response = device
        .execute(build_battery_query())
        .map_err(|error| format!("{}: {}", candidate.label, error))?;

    let (level, charge, voltage) = decode_battery_response(&response);

    Ok(BatterySnapshot {
        level: level.min(100),
        charge,
        voltage,
        is_charging: charge > 0,
        connected: true,
        status: format!(
            "Lecture reussie via libatk-rs sur {:04x}:{:04x} (usage {:04x}:{:04x}).",
            candidate.vendor_id, candidate.product_id, candidate.usage_page, candidate.usage
        ),
        device_label: friendly_device_label(candidate),
        updated_at: iso_now(),
        source: "libatk-rs".to_string(),
        diagnostics: empty_diagnostics("libatk-rs"),
    })
}

fn friendly_device_label(candidate: &DeviceCandidate) -> String {
    sanitize_device_label(&candidate.label)
}

fn sanitize_device_label(label: &str) -> String {
    let cleaned = label
        .split('[')
        .next()
        .unwrap_or(label)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    if cleaned.is_empty() {
        return DEFAULT_DEVICE_LABEL.to_string();
    }

    let normalized = cleaned.to_ascii_lowercase();

    if normalized.contains("dongle")
        || normalized.contains("receiver")
        || (normalized.contains("f1") && normalized.contains("leviatan"))
    {
        return GENERIC_MOUSE_LABEL.to_string();
    }

    cleaned
}

fn score_candidate(
    known_id: bool,
    known_vendor: bool,
    usage_page: u16,
    usage: u16,
    combined: &str,
) -> u8 {
    let mut score: u8 = 0;

    if known_id {
        score += 100;
    }

    if known_vendor {
        score += 60;
    }

    if usage_page == ATK_USAGE_PAGE {
        score += 30;
    }

    if usage == ATK_USAGE {
        score += 20;
    }

    if usage_page == ATK_USAGE_PAGE && usage == ATK_USAGE {
        score += 20;
    }

    if DEVICE_KEYWORDS
        .iter()
        .any(|keyword| combined.contains(keyword))
    {
        score += 15;
    }

    if combined.contains("mouse") {
        score += 10;
    }

    if combined.contains("dongle")
        || combined.contains("receiver")
        || combined.contains("light version")
    {
        score = score.saturating_sub(25);
    }

    score
}

fn build_battery_query() -> Command<GetBatteryStatus> {
    let mut command = Command::default();
    command.set_id(CommandId::GetBatteryLevel);
    command
}

fn decode_battery_response(response: &Command<GetBatteryStatus>) -> (u8, u8, f32) {
    let data = response.data();
    let level = data.first().copied().unwrap_or_default();
    let charge = data.get(1).copied().unwrap_or_default();
    let voltage = data.get(2).copied().unwrap_or_default() as f32 / 10.0;

    (level, charge, voltage)
}

fn update_tray_visuals(app: &AppHandle, snapshot: &BatterySnapshot) {
    let icon = render_tray_battery_icon(Some(snapshot));

    if let Some(tray) = app.tray_by_id("main") {
        let tooltip = if snapshot.connected {
            format!("ATK Tray Monitor {}%", snapshot.level)
        } else {
            "ATK Tray Monitor indisponible".to_string()
        };

        let _ = tray.set_tooltip(Some(tooltip));
        let _ = tray.set_icon(Some(icon.clone()));
    }

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_icon(icon);
    }
}

fn render_tray_battery_icon(snapshot: Option<&BatterySnapshot>) -> Image<'static> {
    let mut rgba = vec![0_u8; (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize];
    let connected = snapshot.map(|value| value.connected).unwrap_or(false);
    let level = snapshot.map(|value| value.level.min(100)).unwrap_or(0);
    let is_charging = snapshot.map(|value| value.is_charging).unwrap_or(false);
    let border = if connected {
        [236, 240, 241, 255]
    } else {
        [148, 163, 184, 255]
    };
    let track = if connected {
        [255, 255, 255, 28]
    } else {
        [148, 163, 184, 20]
    };
    let fill = if !connected {
        [100, 116, 139, 180]
    } else if is_charging {
        [56, 189, 248, 255]
    } else if level <= 20 {
        [248, 113, 113, 255]
    } else if level <= 50 {
        [250, 204, 21, 255]
    } else {
        [74, 222, 128, 255]
    };

    stroke_rect(&mut rgba, TRAY_ICON_SIZE, 6, 8, 18, 14, border);
    fill_rect(&mut rgba, TRAY_ICON_SIZE, 24, 12, 2, 6, border);
    fill_rect(&mut rgba, TRAY_ICON_SIZE, 8, 10, 14, 10, track);

    let fill_width = if connected {
        ((level as u32 * 14) + 99) / 100
    } else {
        0
    };

    if fill_width > 0 {
        fill_rect(&mut rgba, TRAY_ICON_SIZE, 8, 10, fill_width, 10, fill);
    }

    if is_charging && connected {
        let bolt = [255, 255, 255, 220];
        fill_rect(&mut rgba, TRAY_ICON_SIZE, 13, 10, 3, 4, bolt);
        fill_rect(&mut rgba, TRAY_ICON_SIZE, 12, 14, 3, 3, bolt);
        fill_rect(&mut rgba, TRAY_ICON_SIZE, 15, 14, 2, 6, bolt);
    }

    Image::new_owned(rgba, TRAY_ICON_SIZE, TRAY_ICON_SIZE)
}

fn stroke_rect(
    rgba: &mut [u8],
    canvas_size: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: [u8; 4],
) {
    if width == 0 || height == 0 {
        return;
    }

    fill_rect(rgba, canvas_size, x, y, width, 1, color);
    fill_rect(rgba, canvas_size, x, y + height.saturating_sub(1), width, 1, color);
    fill_rect(rgba, canvas_size, x, y, 1, height, color);
    fill_rect(rgba, canvas_size, x + width.saturating_sub(1), y, 1, height, color);
}

fn fill_rect(
    rgba: &mut [u8],
    canvas_size: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: [u8; 4],
) {
    let max_x = (x + width).min(canvas_size);
    let max_y = (y + height).min(canvas_size);

    for current_y in y..max_y {
        for current_x in x..max_x {
            let offset = ((current_y * canvas_size + current_x) * 4) as usize;
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
}

fn enrich_snapshot_with_diagnostics(
    mut snapshot: BatterySnapshot,
    candidates: Vec<DeviceCandidate>,
    last_error: Option<String>,
) -> BatterySnapshot {
    snapshot.diagnostics = BatteryDiagnostics {
        selected_candidate: Some(snapshot.device_label.clone()),
        candidate_count: candidates.len(),
        candidates,
        last_error,
        backend: snapshot.source.clone(),
    };
    snapshot
}

fn empty_diagnostics(backend: &str) -> BatteryDiagnostics {
    BatteryDiagnostics {
        selected_candidate: None,
        candidate_count: 0,
        candidates: Vec::new(),
        last_error: None,
        backend: backend.to_string(),
    }
}

fn disconnected_snapshot(
    message: &str,
    device_label: &str,
    source: &str,
    candidates: Vec<DeviceCandidate>,
) -> BatterySnapshot {
    BatterySnapshot {
        level: 0,
        charge: 0,
        voltage: 0.0,
        is_charging: false,
        connected: false,
        status: message.to_string(),
        device_label: device_label.to_string(),
        updated_at: iso_now(),
        source: source.to_string(),
        diagnostics: BatteryDiagnostics {
            selected_candidate: None,
            candidate_count: candidates.len(),
            candidates,
            last_error: Some(message.to_string()),
            backend: source.to_string(),
        },
    }
}

fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

fn ensure_default_settings(app: &mut tauri::App) -> tauri::Result<()> {
    let store = app
        .store(SETTINGS_FILE)
        .map_err(|error| std::io::Error::other(error.to_string()))?;

    if store.get(START_MINIMIZED_KEY).is_none() {
        store.set(START_MINIMIZED_KEY.to_string(), serde_json::json!(true));
    }

    if store.get(LOW_BATTERY_NOTIFICATIONS_KEY).is_none() {
        store.set(
            LOW_BATTERY_NOTIFICATIONS_KEY.to_string(),
            serde_json::json!(true),
        );
    }

    if store.get(LOW_BATTERY_THRESHOLD_KEY).is_none() {
        store.set(LOW_BATTERY_THRESHOLD_KEY.to_string(), serde_json::json!(20));
    }

    if store.get(LANGUAGE_KEY).is_none() {
        store.set(LANGUAGE_KEY.to_string(), serde_json::json!("en"));
    }

    store
        .save()
        .map_err(|error| std::io::Error::other(error.to_string()))?;

    Ok(())
}

fn emit_settings_updated(app: &AppHandle) {
    let _ = app.emit(SETTINGS_UPDATED_EVENT, ());
}

fn tray_menu_labels(language: &str) -> TrayMenuLabels {
    match language {
        "de" => TrayMenuLabels {
            open: "Offnen",
            hide: "Ausblenden",
            refresh: "Aktualisieren",
            copy_diagnostics: "Diagnose kopieren",
            version: "Version",
            launch_on_startup: "Mit Windows starten",
            start_minimized: "Minimiert starten",
            low_battery_notifications: "Benachrichtigung bei niedrigem Akkustand",
            language: "Sprache",
            threshold: "Warnschwelle",
            settings: "Einstellungen",
            quit: "Beenden",
        },
        "es" => TrayMenuLabels {
            open: "Abrir",
            hide: "Ocultar",
            refresh: "Actualizar",
            copy_diagnostics: "Copiar diagnostico",
            version: "Version",
            launch_on_startup: "Iniciar con Windows",
            start_minimized: "Iniciar minimizado",
            low_battery_notifications: "Notificaciones de bateria baja",
            language: "Idioma",
            threshold: "Umbral de alerta",
            settings: "Ajustes",
            quit: "Salir",
        },
        "fr" => TrayMenuLabels {
            open: "Ouvrir",
            hide: "Masquer",
            refresh: "Actualiser",
            copy_diagnostics: "Copier le diagnostic",
            version: "Version",
            launch_on_startup: "Lancer avec Windows",
            start_minimized: "Demarrage discret",
            low_battery_notifications: "Notifications batterie faible",
            language: "Langue",
            threshold: "Seuil d'alerte",
            settings: "Reglages",
            quit: "Quitter",
        },
        "it" => TrayMenuLabels {
            open: "Apri",
            hide: "Nascondi",
            refresh: "Aggiorna",
            copy_diagnostics: "Copia diagnostica",
            version: "Versione",
            launch_on_startup: "Avvia con Windows",
            start_minimized: "Avvio ridotto",
            low_battery_notifications: "Notifiche batteria scarica",
            language: "Lingua",
            threshold: "Soglia di avviso",
            settings: "Impostazioni",
            quit: "Esci",
        },
        _ => TrayMenuLabels {
            open: "Open",
            hide: "Hide",
            refresh: "Refresh",
            copy_diagnostics: "Copy diagnostics",
            version: "Version",
            launch_on_startup: "Launch with Windows",
            start_minimized: "Start minimized",
            low_battery_notifications: "Low battery notifications",
            language: "Language",
            threshold: "Alert threshold",
            settings: "Settings",
            quit: "Quit",
        },
    }
}

fn format_version_menu_text(label: &str, version: &str) -> String {
    format!("{label} {version}")
}

fn apply_tray_menu_language<R: tauri::Runtime>(
    open: &MenuItem<R>,
    hide: &MenuItem<R>,
    refresh: &MenuItem<R>,
    copy_diagnostics: &MenuItem<R>,
    version: &MenuItem<R>,
    launch_on_startup: &CheckMenuItem<R>,
    start_minimized: &CheckMenuItem<R>,
    low_battery_notifications: &CheckMenuItem<R>,
    language_submenu: &Submenu<R>,
    threshold_submenu: &Submenu<R>,
    settings_submenu: &Submenu<R>,
    quit: &MenuItem<R>,
    app_version: &str,
    language: &str,
) {
    let labels = tray_menu_labels(language);

    let _ = open.set_text(labels.open);
    let _ = hide.set_text(labels.hide);
    let _ = refresh.set_text(labels.refresh);
    let _ = copy_diagnostics.set_text(labels.copy_diagnostics);
    let _ = version.set_text(format_version_menu_text(labels.version, app_version));
    let _ = launch_on_startup.set_text(labels.launch_on_startup);
    let _ = start_minimized.set_text(labels.start_minimized);
    let _ = low_battery_notifications.set_text(labels.low_battery_notifications);
    let _ = language_submenu.set_text(labels.language);
    let _ = threshold_submenu.set_text(labels.threshold);
    let _ = settings_submenu.set_text(labels.settings);
    let _ = quit.set_text(labels.quit);
}

fn set_language_menu_checked<R: tauri::Runtime>(
    language_de: &CheckMenuItem<R>,
    language_en: &CheckMenuItem<R>,
    language_es: &CheckMenuItem<R>,
    language_fr: &CheckMenuItem<R>,
    language_it: &CheckMenuItem<R>,
    selected_language: &str,
) {
    let _ = language_de.set_checked(selected_language == "de");
    let _ = language_en.set_checked(selected_language == "en");
    let _ = language_es.set_checked(selected_language == "es");
    let _ = language_fr.set_checked(selected_language == "fr");
    let _ = language_it.set_checked(selected_language == "it");
}

fn build_diagnostics_report(snapshot: &BatterySnapshot) -> String {
    let candidates = if snapshot.diagnostics.candidates.is_empty() {
        "Aucun candidat HID compatible detecte".to_string()
    } else {
        snapshot
            .diagnostics
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                format!(
                    "{}. {} | score={} | VID={:04X} PID={:04X} U={:04X}:{:04X}",
                    index + 1,
                    candidate.label,
                    candidate.score,
                    candidate.vendor_id,
                    candidate.product_id,
                    candidate.usage_page,
                    candidate.usage
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    [
        "ATK Tray Monitor diagnostic",
        &format!("Horodatage: {}", snapshot.updated_at),
        &format!("Source: {}", snapshot.source),
        &format!("Backend: {}", snapshot.diagnostics.backend),
        &format!("Connecte: {}", if snapshot.connected { "oui" } else { "non" }),
        &format!("Niveau: {}%", snapshot.level),
        &format!("Charge: {}", snapshot.charge),
        &format!("Tension: {:.1}V", snapshot.voltage),
        &format!(
            "Cible retenue: {}",
            snapshot
                .diagnostics
                .selected_candidate
                .clone()
                .unwrap_or_else(|| "aucune".to_string())
        ),
        &format!(
            "Erreur: {}",
            snapshot
                .diagnostics
                .last_error
                .clone()
                .unwrap_or_else(|| "aucune".to_string())
        ),
        &format!("Candidats: {}", snapshot.diagnostics.candidate_count),
        "",
        &candidates,
    ]
    .join("\n")
}

fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| error.to_string())
}

fn autostart_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

fn set_autostart_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        app.autolaunch().enable().map_err(|error| error.to_string())
    } else {
        app.autolaunch().disable().map_err(|error| error.to_string())
    }
}

fn load_bool_setting(app: &AppHandle, key: &str, default: bool) -> bool {
    match app.store(SETTINGS_FILE) {
        Ok(store) => store.get(key).and_then(|value| value.as_bool()).unwrap_or(default),
        Err(_) => default,
    }
}

fn load_u8_setting(app: &AppHandle, key: &str, default: u8) -> u8 {
    match app.store(SETTINGS_FILE) {
        Ok(store) => store
            .get(key)
            .and_then(|value| value.as_u64())
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(default),
        Err(_) => default,
    }
}

fn load_string_setting(app: &AppHandle, key: &str) -> Option<String> {
    match app.store(SETTINGS_FILE) {
        Ok(store) => store
            .get(key)
            .and_then(|value| value.as_str().map(ToString::to_string)),
        Err(_) => None,
    }
}

fn save_bool_setting(app: &AppHandle, key: &str, value: bool) -> Result<(), String> {
    let store = app.store(SETTINGS_FILE).map_err(|error| error.to_string())?;
    store.set(key.to_string(), serde_json::json!(value));
    store.save().map_err(|error| error.to_string())
}

fn save_u8_setting(app: &AppHandle, key: &str, value: u8) -> Result<(), String> {
    let store = app.store(SETTINGS_FILE).map_err(|error| error.to_string())?;
    store.set(key.to_string(), serde_json::json!(value));
    store.save().map_err(|error| error.to_string())
}

fn save_string_setting(app: &AppHandle, key: &str, value: &str) -> Result<(), String> {
    let store = app.store(SETTINGS_FILE).map_err(|error| error.to_string())?;
    store.set(key.to_string(), serde_json::json!(value));
    store.save().map_err(|error| error.to_string())
}