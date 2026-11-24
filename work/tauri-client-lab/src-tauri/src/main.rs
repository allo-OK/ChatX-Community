#![windows_subsystem = "windows"]
mod network;
mod chat_wallpaper;
mod history;
mod host;
mod gallery;
mod updates;
mod uploads;
mod chat_tray;
mod upscale;
mod crop_faces;
mod desktop_design;
mod gallery_download;
mod gallery_local;
mod desktop_health;
mod windows_activity;
mod work_schedule;
mod bledar;
mod cohost;
mod profiles;
mod message_tags;
mod asset_library;
mod chess;
mod games;
mod game_window;
mod diagnostics;
mod client_updater;
mod client_profile;
mod windows_secret;
mod renderer_preferences;
mod desktop_clipboard;
mod image_files;
mod notification;

use std::sync::Arc;
use tauri::Manager;

#[tauri::command]
fn directed_broadcast(address: &str, netmask: &str) -> Option<String> {
    network::directed_broadcast(address, netmask).map(|ip| ip.to_string())
}
#[tauri::command]
async fn discover_primary_server() -> Result<Option<String>, String> {
    let port = if client_renderer_smoke() {
        std::env::var("KC_LAB_DISCOVERY_PORT").ok().and_then(|value| value.parse().ok()).unwrap_or(45821)
    } else { 45821 };
    let address = network::discover_primary(port).await.map_err(|error| error.to_string())?;
    Ok(address.map(|address| format!("http://{address}")))
}

#[tauri::command]
async fn probe_client_host(address: String) -> Option<String> {
    network::probe_primary(&address).await.map(|address| format!("http://{address}"))
}

#[tauri::command]
fn client_computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into())
}

#[tauri::command]
async fn client_copy_png(window: tauri::WebviewWindow, clipboard: tauri::State<'_, Arc<desktop_clipboard::Clipboard>>, png: String) -> Result<serde_json::Value, String> {
    if window.label() != "main" { return Err("Jendela clipboard tidak valid.".into()); }
    let hwnd = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    clipboard.inner().clone().copy_mixed(hwnd, serde_json::json!({"png":png})).await
}

#[tauri::command]
fn client_notification(window: tauri::WebviewWindow, notifications: tauri::State<'_, Arc<notification::Notifications>>,
    updater: tauri::State<'_, Arc<client_updater::ClientUpdater>>, mut payload: serde_json::Value, settings: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if window.label() != "main" || !payload.is_object() { return Err("Data notifikasi CLIENT tidak valid".into()); }
    payload["_mediaOrigin"] = serde_json::json!(updater.origin()?);
    Ok(notifications.inner().enqueue(window.app_handle(), window.label(), payload, settings))
}

fn client_renderer_smoke() -> bool { std::env::args().any(|arg| arg == "--smoke-client-renderer") }
#[tauri::command]
async fn chat_test_port(updater:tauri::State<'_,Arc<client_updater::ClientUpdater>>) -> Result<Option<u16>, String> {
    let Ok(port) = std::env::var("KC_LAB_DISCOVERY_PORT") else {
        return Ok(None);
    };
    let port = port.parse::<u16>().map_err(|e| e.to_string())?;
    let loopback = std::net::Ipv4Addr::LOCALHOST;
    let address=network::discover(loopback, &[loopback], port, std::time::Duration::from_millis(1100))
        .await.ok_or("No healthy lab HOST found")?;
    updater.set_discovered(address);
    Ok(Some(address.port()))
}

#[tauri::command]
fn finish_chat_test(app: tauri::AppHandle, error: Option<String>) {
    if let Some(error) = error {
        eprintln!("LAB_CHAT_FAILED: {error}");
        app.exit(1);
    } else {
        println!("LAB_CHAT_PASSED");
        app.exit(0);
    }
}

#[tauri::command(rename = "upscale_image")]
async fn guarded_upscale_image(updater: tauri::State<'_, Arc<client_updater::ClientUpdater>>,
    profile: tauri::State<'_, client_profile::ProfileStore>,
    job: tauri::State<'_, desktop_design::UpscaleJob>, input: serde_json::Value) -> Result<serde_json::Value, String> {
    updater.authorize_local(&profile, "designAI", "alat desain / AI").await?;
    desktop_design::upscale_image(job, input).await
}

#[tauri::command(rename = "write_vector_output")]
async fn guarded_write_vector_output(updater: tauri::State<'_, Arc<client_updater::ClientUpdater>>,
    profile: tauri::State<'_, client_profile::ProfileStore>,
    outputs: tauri::State<'_, Arc<image_files::PngOutputs>>, input: serde_json::Value) -> Result<serde_json::Value, String> {
    updater.authorize_local(&profile, "designAI", "alat desain / AI").await?;
    desktop_design::write_vector_output(outputs, input).await
}

#[tauri::command(rename = "gallery_download")]
async fn guarded_gallery_download(window: tauri::WebviewWindow,
    updater: tauri::State<'_, Arc<client_updater::ClientUpdater>>,
    profile: tauri::State<'_, client_profile::ProfileStore>,
    downloads: tauri::State<'_, Arc<gallery_download::Downloads>>,
    action: String, input: serde_json::Value) -> Result<serde_json::Value, String> {
    match action.as_str() {
        "start" | "finish" => updater.authorize_local(&profile, "downloadFiles", "unduh file").await?,
        "write" => updater.authorize_cached_local(&profile, "downloadFiles", "unduh file")?,
        _ => {}
    }
    gallery_download::gallery_download(window, downloads, action, input).await
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let migrate = args.get(1).is_some_and(|arg| arg == "--migrate-electron-client");
    let cleanup = args.get(1).is_some_and(|arg| arg == "--cleanup-electron-client");
    let check_cleanup = args.get(1).is_some_and(|arg| arg == "--check-electron-client-cleanup");
    let installer = migrate || cleanup || check_cleanup;
    if installer || args.get(1).is_some_and(|arg| arg == "--import-client-profile") {
        let diagnostic = installer.then(|| std::env::current_exe().ok().map(|path|
            path.with_file_name("migration-error.txt"))).flatten();
        let result = (|| -> Result<(), String> {
            let directory = if let Some(path) = args.get(3).filter(|_| !installer) {
                std::path::PathBuf::from(path)
            } else {
                let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).expect("Tauri config");
                let identifier = config["identifier"].as_str().expect("CLIENT identifier");
                std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).ok_or("Folder LOCALAPPDATA tidak tersedia")?.join(identifier)
            };
            let store = client_profile::ProfileStore::new(directory.clone());
            if installer {
                if args.len() != 2 {
                    return Err("Migrasi installer memakai profil Windows saat ini.".into());
                }
                let source = directory.join("electron-client-import");
                let saved = directory.join("electron-client-recovery");
                if cleanup {
                    if store.load()?.is_none() {
                        return Err("Profil ChatX belum siap.".into());
                    }
                    if saved.exists() {
                        store.cleanup_legacy(&saved)?;
                    }
                    if source.exists() {
                        store.cleanup_legacy(&source)?;
                    }
                    return Ok(());
                }
                if !source.exists() && store.load()?.is_some() {
                    return Ok(());
                }
                return if check_cleanup { store.check_legacy_cleanup(&source) } else { store.migrate_legacy(&source) };
            }
            let source = args.get(2).map(std::path::Path::new).ok_or("Isi lokasi config.json Electron CLIENT")?;
            store.import(source)
        })();
        match result {
            Ok(()) => {
                if let Some(path) = diagnostic { let _ = std::fs::remove_file(path); }
                println!("Identitas CLIENT disimpan");
            }
            Err(error) => {
                if let Some(path) = diagnostic {
                    let _ = std::fs::write(path, error.replace(['\r', '\n'], " "));
                }
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--discover-primary-host") {
        let result = (|| -> Result<_, Box<dyn std::error::Error>> {
            let port = args.get(2).map(|arg| arg.to_string_lossy().parse::<u16>()).transpose()?.unwrap_or(45821);
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
            let address = runtime.block_on(network::discover_primary(port))?;
            Ok(address.map(|address| format!("http://{address}")))
        })();
        match result {
            Ok(address) => println!("{}", serde_json::json!(address)),
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    if args.len() == 2 && args[1] == "--lab-host" {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("Lab runtime");
        if let Err(error) = runtime.block_on(host::serve()) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 4 && args[1] == "--verify-history-unread" {
        let request: serde_json::Value = serde_json::from_str(&args[3].to_string_lossy()).expect("Invalid test request");
        let count = history::unread(
            std::path::Path::new(&args[2]),
            request["conversationId"].as_str().expect("Missing conversation"),
            request["name"].as_str().expect("Missing reader name"),
            request["since"].as_f64().unwrap_or(0.0),
        ).expect("Count unread messages");
        println!("{count}");
        return;
    }
    if args.len() == 4 && args[1] == "--verify-history-page" {
        let request: serde_json::Value = serde_json::from_str(&args[3].to_string_lossy()).expect("Invalid test request");
        let page = history::page(
            std::path::Path::new(&args[2]),
            request["conversationId"].as_str().expect("Missing conversation"),
            request["limit"].as_i64().unwrap_or(120),
            request["beforeTs"].as_f64(),
            request["beforeId"].as_str().unwrap_or(""),
        ).expect("Read test page");
        println!("{page}");
        return;
    }
    if args.len() == 4 && args[1] == "--verify-history-write" {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let batch = std::fs::read(&args[3])?;
            let messages = serde_json::from_slice::<Vec<serde_json::Value>>(&batch)?;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
            history::write_batch(std::path::Path::new(&args[2]), &messages, now.as_millis() as i64)?;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 3 && args[1] == "--verify-history" {
        match history::read_snapshot(std::path::Path::new(&args[2]), 5000) {
            Ok(rows) => println!("{}", serde_json::Value::Array(rows)),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let use_renderer = client_renderer_smoke() || (std::env::var_os("KC_LAB_DISCOVERY_PORT").is_none()
        && !std::env::args().any(|arg| arg == "--smoke-test"));
    let mut context = tauri::generate_context!();
    if use_renderer {
        let config = context.config_mut();
        config.app.windows.clear();
        config.app.security.csp = Some(tauri::utils::config::Csp::Policy(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob: http:; media-src 'self' blob: http:; \
             connect-src 'self' data: ipc: http: ws:".into()));
    }
    tauri::Builder::default()
        .setup(move |app| {
            let fixture=std::env::var_os("KC_LAB_DISCOVERY_PORT").is_some();
            let override_root=fixture.then(||std::env::var_os("KC_LAB_UPDATER_DIR")).flatten().map(std::path::PathBuf::from);
            let profile_root = override_root.clone().unwrap_or(app.path().app_local_data_dir()?);
            if !fixture { work_schedule::start(app.handle().clone(), profile_root.clone()); }
            let profile = client_profile::ProfileStore::new(profile_root.clone());
            let gallery = gallery_local::LocalGallery::new(&profile_root);
            if let Ok(Some(identity)) = profile.load() {
                for folder in identity["galleryFolders"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str) {
                    gallery.allow(std::path::Path::new(folder));
                }
            }
            app.manage(profile);
            app.manage(chat_wallpaper::Wallpaper::new(&profile_root));
            let design_outputs = Arc::new(image_files::PngOutputs::default());
            if client_renderer_smoke() {
                if let Some(directory) = std::env::var_os("KC_TEST_CLIENT_DESIGN_DIRECTORY") {
                    design_outputs.allow(std::path::Path::new(&directory)).map_err(std::io::Error::other)?;
                }
            }
            app.manage(design_outputs);
            app.manage(desktop_design::UpscaleJob::default());
            app.manage(Arc::new(gallery_download::Downloads::new(&profile_root)));
            app.manage(Arc::new(gallery));
            app.manage(desktop_health::HealthStorage(profile_root.clone()));
            app.manage(windows_activity::LocalActivity::default());
            app.manage(Arc::new(desktop_clipboard::Clipboard::default()));
            app.manage(Arc::new(notification::Notifications::default()));
            app.manage(game_window::GameWindowState::new("main"));
            let (trust,downloads)=match override_root {
                Some(root)=>(root.join("trusted-hosts.json"),root.join("downloads")),
                None=>(app.path().app_local_data_dir()?.join("trusted-hosts.json"),
                    app.path().download_dir()?.join("ChatX Updates")),
            };
            let updater = Arc::new(client_updater::ClientUpdater::new(trust,downloads,fixture,app.package_info().version.to_string(),use_renderer));
            app.manage(updater);
            if use_renderer {
                let mut window = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("v814/renderer/index.html".into()))
                    .disable_drag_drop_handler()
                    .on_download(chat_tray::download_feedback)
                    .title("ChatX").inner_size(1220.0,800.0).min_inner_size(800.0,580.0)
                    .initialization_script(include_str!("../../desktop-design-bridge.js"))
                    .initialization_script(include_str!("../../client-renderer-bridge.js"));
                window = window.icon(chat_tray::window_icon()?)?;
                if let Some(script) = renderer_preferences::initialization_script(&profile_root)? {
                    window = window.initialization_script(script);
                }
                if client_renderer_smoke() {
                    if std::env::var_os("KC_TEST_CLIENT_BACKGROUND_SAMPLE").is_some() {
                        window = window.initialization_script("window.__kcClientBackgroundSmoke = true;");
                    }
                    if std::env::var_os("KC_TEST_UPSCALE_RESOURCES").is_some() {
                        window = window.initialization_script("window.__kcClientUpscaleSmoke = true;");
                    }
                    if std::env::var_os("KC_TEST_FRESH_CLIENT").is_some() {
                        window = window.initialization_script("window.__kcFreshClientSmoke = true;");
                    }
                    window = window.initialization_script(include_str!("../../client-renderer-smoke.js"));
                }
                let window = window.build()?;
                chat_tray::install(app.handle(), "main")?;
                network::monitor_recovery(window);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![image_files::open_vectorizer_web,chat_test_port, finish_chat_test, directed_broadcast, discover_primary_server,
            probe_client_host, client_computer_name, client_profile::client_device_identity, client_profile::create_client_identity,
            client_copy_png,
            chat_wallpaper::chat_wallpaper,
            desktop_health::check_health_storage,
            windows_activity::get_local_activity,
            work_schedule::close_work_schedule,
            desktop_design::set_native_theme,
            desktop_clipboard::read_clipboard_image,
            client_updater::refresh_client_feature_policy,
            desktop_design::upscale_status, desktop_design::upscale_progress, guarded_upscale_image,
            desktop_design::choose_image_files, desktop_design::choose_upscale_output, desktop_design::choose_vector_output,
            desktop_design::write_base64_file, desktop_design::save_png, desktop_design::detect_crop_faces,
            guarded_write_vector_output, desktop_design::read_local_image, desktop_design::open_image_path,
            client_notification, notification::notification_snapshot, notification::notification_ready, notification::notification_action,
            game_window::open_game, game_window::game_window_sync, game_window::game_window_action,
            client_updater::remember_client_session,
            client_updater::download_client_attachment,
            guarded_gallery_download, gallery_download::gallery_choose_download_folder,
            gallery_local::gallery_local,
            client_updater::remember_update_host, client_updater::prepare_client_update, client_updater::open_client_update,
            client_updater::client_required_update, client_updater::refresh_client_update])
        .on_window_event(|window, event| {
            if window.label() == "main" && window.app_handle().tray_by_id("chatx-tray").is_some() {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if window.hide().is_err() { let _ = window.minimize(); }
                }
            }
        })
        .on_page_load(|webview, event| {
            if std::env::args().any(|arg| arg == "--smoke-test")
                && event.event() == tauri::webview::PageLoadEvent::Finished
            {
                use tauri::Manager;
                println!("LAB_PAGE_LOADED");
                webview.app_handle().exit(0);
            }
        })
        .run(context)
        .expect("ChatX CLIENT failed to start");
}
