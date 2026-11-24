#![windows_subsystem = "windows"]

#[path = "../history.rs"]
mod history;
#[path = "../chat_wallpaper.rs"]
mod chat_wallpaper;
#[path = "../host.rs"]
mod host;
#[path = "../network.rs"]
mod network;
#[path = "../gallery.rs"]
mod gallery;
#[path = "../gallery_download.rs"]
mod gallery_download;
#[path = "../gallery_local.rs"]
mod gallery_local;
#[path = "../backup_space.rs"]
mod backup_space;
#[path = "../renderer_preferences.rs"]
mod renderer_preferences;
#[path = "../desktop_profile_import.rs"]
mod desktop_profile_import;
#[path = "../chat_tray.rs"]
mod chat_tray;
#[path = "../updates.rs"]
mod updates;
#[path = "../uploads.rs"]
mod uploads;
#[path = "../desktop_clipboard.rs"]
mod desktop_clipboard;
#[path = "../image_files.rs"]
mod image_files;

#[tauri::command]
async fn choose_storage_folder(window: tauri::WebviewWindow) -> Result<Option<std::path::PathBuf>, String> {
    if window.label() != "host" {
        return Err("Pengaturan penyimpanan hanya tersedia di HOST".into());
    }
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--smoke-gui") {
        if let Some(path) = std::env::var_os("KC_TEST_STORAGE_DESTINATION") {
            return Ok(Some(path.into()));
        }
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    tokio::task::spawn_blocking(move || image_files::choose_folder(owner))
        .await.map_err(|error| error.to_string())?
}
#[path = "../crop_faces.rs"]
mod crop_faces;
#[path = "../desktop_design.rs"]
mod desktop_design;
#[path = "../desktop_health.rs"]
mod desktop_health;
#[path = "../windows_activity.rs"]
mod windows_activity;
#[path = "../work_schedule.rs"]
mod work_schedule;
#[path = "../upscale.rs"]
mod upscale;
#[path = "../bledar.rs"]
mod bledar;
#[path = "../cohost.rs"]
mod cohost;
#[path = "../profiles.rs"]
mod profiles;
#[path = "../message_tags.rs"]
mod message_tags;
#[path = "../asset_library.rs"]
mod asset_library;
#[path = "../chess.rs"]
mod chess;
#[path = "../games.rs"]
mod games;
#[path = "../diagnostics.rs"]
mod diagnostics;
#[path = "../game_window.rs"]
mod game_window;
#[path = "../notification.rs"]
mod notification;
#[path = "../support_log.rs"]
mod support_log;
#[path = "../bug_queue.rs"]
mod bug_queue;

use serde_json::{json, Value};
use std::{io::Write, sync::{Arc, Mutex}};
use tauri::Manager;

type HostStatus = Arc<Mutex<Value>>;
const RENDERER_BRIDGE: &str = concat!(
    include_str!("../../../desktop-design-bridge.js"), "\n",
    include_str!("../../../host-renderer-bridge.js"),
);

#[tauri::command]
async fn copy_local_text(
    window: tauri::WebviewWindow, clipboard: tauri::State<'_, Arc<desktop_clipboard::Clipboard>>, text: String,
) -> Result<Value, String> {
    let hwnd = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    Arc::clone(clipboard.inner()).copy_text(hwnd, text).await
}

#[tauri::command]
async fn copy_chat_image(
    window: tauri::WebviewWindow, clipboard: tauri::State<'_, Arc<desktop_clipboard::Clipboard>>, input: Value,
) -> Result<Value, String> {
    let hwnd = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    Arc::clone(clipboard.inner()).copy_mixed(hwnd, input).await
}

#[tauri::command]
fn host_status(status: tauri::State<'_, HostStatus>) -> Value {
    status.lock().expect("status HOST").clone()
}

#[tauri::command]
async fn host_moderation_snapshot(window: tauri::WebviewWindow, status: tauri::State<'_, HostStatus>) -> Result<Value, String> {
    if window.label() != "host" { return Err("Moderation hanya tersedia di jendela HOST.".into()); }
    let (address, token) = {
        let state = status.lock().map_err(|_| "Status HOST gagal dibaca.")?;
        let address = state["address"].as_str().ok_or("Listener HOST belum siap.")?;
        let token = state["adminToken"].as_str().ok_or("Sesi HOST belum siap.")?;
        (address.to_owned(), token.to_owned())
    };
    let response = reqwest::Client::new().get(format!("{address}/admin/moderation"))
        .bearer_auth(token).timeout(std::time::Duration::from_secs(8)).send().await
        .map_err(|error| if error.is_timeout() { "HOST terlalu lama memuat Moderation.".to_owned() }
            else { "Koneksi lokal ke Moderation gagal.".to_owned() })?;
    let status = response.status();
    let body = response.bytes().await.map_err(|_| "Respons Moderation terputus.")?;
    let data: Value = serde_json::from_slice(&body).map_err(|_| "Respons Moderation tidak valid.")?;
    if !status.is_success() || data["ok"] != true {
        return Err(data["error"].as_str().unwrap_or("Moderation gagal dimuat.").to_owned());
    }
    Ok(data)
}

#[tauri::command]
async fn download_host_attachment(window: tauri::WebviewWindow, status: tauri::State<'_, HostStatus>,
    url: String, name: String) -> Result<Value, String> {
    use tokio::io::AsyncWriteExt;
    if window.label() != "host" { return Err("Unduhan hanya tersedia di jendela HOST.".into()); }
    let origin = status.lock().expect("status HOST")["address"].as_str()
        .ok_or("HOST belum aktif.")?.to_owned();
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Alamat lampiran tidak valid.")?;
    if parsed.origin().ascii_serialization() != origin || !parsed.username().is_empty()
        || parsed.password().is_some() || parsed.fragment().is_some()
        || !parsed.path().starts_with("/download/")
        || parsed.query_pairs().any(|(key, _)| key != "kc_token") {
        return Err("Alamat lampiran tidak valid.".into());
    }
    let clean: String = name.chars().take(180).map(|ch|
        if ch.is_control() || r#"<>:"/\|?*"#.contains(ch) { '_' } else { ch }).collect();
    let clean = clean.trim_matches(['.', ' ']);
    let name = if clean.is_empty() { "lampiran" } else { clean }.to_owned();
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    let destination = tokio::task::spawn_blocking(move || image_files::choose_save_path(
        owner, &name, "Semua file\0*.*\0\0", "", "Simpan lampiran"
    )).await.map_err(|error| error.to_string())??;
    let Some(destination) = destination else { return Ok(json!({"ok":false,"canceled":true})); };
    let temporary = destination.with_file_name(format!(".kc-download-{}.part", uuid::Uuid::new_v4()));
    let result = async {
        let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(60)).build().map_err(|_| "Unduhan gagal disiapkan.")?;
        let mut response = client.get(parsed).send().await.map_err(|_| "Koneksi unduhan gagal.")?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!("Unduh lampiran HTTP {}", response.status().as_u16()));
        }
        let expected = response.content_length();
        let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)
            .await.map_err(|error| error.to_string())?;
        let mut size = 0;
        while let Some(bytes) = response.chunk().await.map_err(|_| "Unduhan terputus.")? {
            file.write_all(&bytes).await.map_err(|error| error.to_string())?;
            size += bytes.len() as u64;
        }
        if expected.is_some_and(|length| length != size) { return Err("Unduhan belum lengkap.".into()); }
        file.sync_all().await.map_err(|error| error.to_string())?;
        drop(file);
        tokio::fs::rename(&temporary, &destination).await.map_err(|error| error.to_string())?;
        Ok(json!({"ok":true,"path":destination,"size":size}))
    }.await;
    if result.is_err() { let _ = tokio::fs::remove_file(&temporary).await; }
    result
}

#[tauri::command]
async fn open_server_data(window: tauri::WebviewWindow, status: tauri::State<'_, HostStatus>) -> Result<Value, String> {
    if window.label() != "host" {
        return Err("Folder data tersedia di jendela HOST".into());
    }
    let path = status.lock().expect("status HOST")["dataPath"].as_str().map(std::path::PathBuf::from);
    let Some(path) = path else { return Ok(json!({"ok":false,"error":"Server belum aktif"})); };
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    tokio::task::spawn_blocking(move || image_files::open_path(owner, &path)
        .unwrap_or_else(|error| json!({"ok":false,"error":error})))
        .await.map_err(|error| error.to_string())
}

#[tauri::command]
fn send_notification(window: tauri::WebviewWindow, state: tauri::State<'_, Arc<notification::Notifications>>,
    status: tauri::State<'_, HostStatus>, mut payload: Value, settings: Value,
) -> Value {
    if !(window.label() == "host") || !payload.is_object() {
        return json!({"ok":false,"error":"Data notifikasi tidak valid."});
    }
    payload["_mediaOrigin"] = status.lock().expect("status HOST")["address"].clone();
    state.inner().enqueue(window.app_handle(), window.label(), payload, settings)
}

#[tauri::command]
fn host_ui_ready(app: tauri::AppHandle, status: tauri::State<'_, HostStatus>, error: Option<String>) {
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--smoke-gui")
        && status.lock().expect("status HOST")["state"] == "running"
        && app.get_webview_window("host").is_some()

    {
        if let Some(error) = error {
            println!("HOST_GUI_SMOKE_FAILED: {error}");
            app.exit(1);
        } else {
            println!("HOST_GUI_READY");
            if let Some(signal) = std::env::var_os("KC_TEST_WAIT_FOR_CLIENT") {
                tauri::async_runtime::spawn(async move {
                    let signal = std::path::PathBuf::from(signal);
                    let long_test = std::env::var_os("KC_TEST_CLIENT_BACKGROUND_SAMPLE").is_some()
                        || std::env::var_os("KC_TEST_UPSCALE_RESOURCES").is_some();
                    let seconds = if long_test { 145 } else { 50 };
                    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds);
                    while !signal.is_file() && tokio::time::Instant::now() < deadline {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    app.exit(if signal.is_file() { 0 } else { 1 });
                });
            } else {
                app.exit(0);
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args_os().nth(1);
    if mode.as_deref() == Some(std::ffi::OsStr::new("--detect-host-lan-ip")) {
        writeln!(std::io::stdout().lock(), "{}", network::primary_lan_address()?)?;
        return Ok(());
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new("--inspect-electron-host")) {
        let root = std::env::args_os().nth(2).map(std::path::PathBuf::from)
            .ok_or("Lokasi profil Electron HOST tidak tersedia")?;
        let profile = desktop_profile_import::ElectronHostProfile::open(&root)?;
        writeln!(std::io::stdout().lock(), "{}", profile.inventory()?)?;
        return Ok(());
    }
    let cutover_smoke = mode.as_deref() == Some(std::ffi::OsStr::new("--smoke-replace-electron-host"));
    let electron_profile = if cutover_smoke || mode.as_deref() == Some(std::ffi::OsStr::new("--replace-electron-host")) {
        if cutover_smoke && std::env::args_os().nth(2).is_none() {
            return Err("Tes penggantian memerlukan direktori profil fixture".into());
        }
        let root = std::env::args_os().nth(2).map(std::path::PathBuf::from)
            .ok_or("Lokasi profil Electron HOST tidak tersedia")?;
        let profile = desktop_profile_import::ElectronHostProfile::open(&root)?;
        if cutover_smoke {
            let fixture_parent = std::env::current_exe()?.parent()
                .ok_or("Lokasi EXE tes tidak valid")?.canonicalize()?;
            if profile.root != fixture_parent
                || !profile.root.file_name().and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|name| name.starts_with("host-import-qa-")) {
                return Err("Tes penggantian hanya menerima profil fixture di samping EXE tes".into());
            }
        }
        if !profile.server_data.join(history::ELECTRON_HISTORY).is_file()
            || profile.server_data.join("chat.sqlite").try_exists()? {
            return Err("Database Electron tidak ditemukan atau folder berisi dua database chat".into());
        }
        Some(profile)
    } else {
        None
    };
    let data_dir = match &electron_profile {
        Some(profile) => profile.server_data.clone(),
        None => std::env::current_exe()?.with_file_name("host-rust-lab-data"),
    };
    let profile_dir = electron_profile.as_ref().map_or_else(|| data_dir.clone(), |profile| profile.root.clone());
    if mode.as_deref() == Some(std::ffi::OsStr::new("--import-host-backup")) {
        let source = std::env::args_os().nth(2).ok_or("Isi folder backup HOST terverifikasi")?;
        let imported = backup_space::import_host_backup(std::path::Path::new(&source), &data_dir)
            .map_err(std::io::Error::other)?;
        let _ = writeln!(std::io::stdout().lock(), "{imported}");
        return Ok(());
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new("--import-host-config")) {
        let source = std::env::args_os().nth(2).ok_or("Isi lokasi config.json Electron HOST")?;
        let imported = desktop_profile_import::import(std::path::Path::new(&source), &data_dir)?;
        let _ = writeln!(std::io::stdout().lock(), "{imported}");
        return Ok(());
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new("--lab-host")) {
        tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(host::serve())?;
        return Ok(());
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new("--lab-lan-host")) {
        let address = std::env::args().nth(2).ok_or("Isi IPv4 privat untuk LAN lab")?.parse::<std::net::Ipv4Addr>()?;
        if !address.is_private() {
            return Err("Alamat HOST lab harus IPv4 LAN privat".into());
        }
        let directory = std::env::current_exe()?.with_file_name("host-rust-lan-lab-data");
        std::fs::create_dir_all(&directory)?;
        std::env::set_current_dir(directory)?;
        tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(host::serve_lan_lab(address))?;
        return Ok(());
    }
    let auto_lan = mode.as_deref() == Some(std::ffi::OsStr::new("--lan-host-auto"));
    let lan = match mode.as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("--lan-host") => {
            let address = std::env::args().nth(2).ok_or("Isi IPv4 privat untuk --lan-host")?.parse()?;
            Some(host::LanBinding { address, ephemeral: false })
        }
        Some("--smoke-gui") => std::env::var("KC_TEST_LAN_ADDRESS").ok()
            .map(|raw| raw.parse().map(|address| host::LanBinding { address, ephemeral: true }))
            .transpose()?,
        _ => None,
    };
    let (lan, reserved) = if electron_profile.is_some() {
        let lan = Some(host::LanBinding { address: network::primary_lan_address()?, ephemeral: cutover_smoke });
        let listeners = tauri::async_runtime::block_on(host::HostListeners::bind(lan, true))
            .map_err(|error| std::io::Error::other(format!("Port HOST masih dipakai. Tutup HOST Electron sebelum beralih. {error}")))?;
        (lan, Some(listeners))
    } else {
        (lan, None)
    };
    let office_host = auto_lan || lan.as_ref().is_some_and(|binding| !binding.ephemeral);
    let admin = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let status: HostStatus = Arc::new(Mutex::new(json!({"state":"starting","adminToken":admin})));
    let support = Arc::new(support_log::SupportLog::new(data_dir.clone()));
    let backup_space = Arc::new(backup_space::BackupSpace::new(
        data_dir.clone(),
        electron_profile.as_ref().map(|profile| profile.root.clone()),
    ));
    let snapshot = backup_space.gate();
    let update_permit=Arc::new(updates::UpdatePermit::default());
    tauri::async_runtime::spawn(backup_space.clone().schedule(status.clone(),support.clone()));
    let _ = support.record("runtime","host-starting",&json!({"version":"1.1.43","pid":std::process::id()}));
    let bug_queue = Arc::new(bug_queue::BugQueue::new(&data_dir));
    tauri::async_runtime::spawn(bug_queue.clone().run(status.clone(), support.clone()));
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = "id.chatx.community.host".into();
    tauri::Builder::default()
        .manage(status.clone())
        .manage(support.clone())
        .manage(bug_queue)
        .manage(chat_wallpaper::Wallpaper::new(&data_dir))
        .manage(tokio::sync::Mutex::new(()))
        .manage(Arc::new(desktop_clipboard::Clipboard::default()))
        .manage(Arc::new(image_files::PngOutputs::default()))
        .manage(Arc::new(gallery_download::Downloads::new(&data_dir)))
        .manage(Arc::new(gallery_local::LocalGallery::new(&data_dir)))
        .manage(backup_space)
        .manage(update_permit.clone())
        .manage(Arc::new(notification::Notifications::default()))
        .manage(desktop_design::UpscaleJob::default())
        .manage(desktop_health::HealthStorage(data_dir.clone()))
        .manage(windows_activity::LocalActivity::default())
        .invoke_handler(tauri::generate_handler![image_files::open_vectorizer_web,
            support_log::support_event, support_log::open_support_folder, support_log::export_support_log,
            bug_queue::queue_bug_report,
            gallery_download::gallery_choose_download_folder, gallery_download::gallery_download,
            gallery_local::gallery_local,
            backup_space::backup_space, backup_space::host_storage_inventory, backup_space::backup_recovery, backup_space::backup_config, backup_space::backup_create, choose_storage_folder,
            game_window::open_game, game_window::game_window_sync, game_window::game_window_action,
            host_status, host_moderation_snapshot, download_host_attachment, host_ui_ready,
            chat_wallpaper::chat_wallpaper,
            desktop_health::check_health_storage,
            windows_activity::get_local_activity,
            work_schedule::close_work_schedule,
            desktop_design::set_native_theme, open_server_data,
            desktop_clipboard::read_clipboard_image,
            copy_local_text, copy_chat_image,
            desktop_design::upscale_status, desktop_design::upscale_progress, desktop_design::upscale_image, desktop_design::choose_image_files, desktop_design::choose_upscale_output, desktop_design::read_local_image, desktop_design::open_image_path,
            desktop_design::choose_vector_output, desktop_design::write_base64_file, desktop_design::save_png, desktop_design::detect_crop_faces, desktop_design::write_vector_output,
            send_notification, notification::notification_snapshot, notification::notification_ready, notification::notification_action
        ])
        .setup(move |app| {
            app.manage(game_window::GameWindowState::new("host"));
            let smoke = std::env::args_os().nth(1).is_some_and(|arg| arg == "--smoke-gui");
            let mut window = tauri::WebviewWindowBuilder::new(app, "host", tauri::WebviewUrl::App("v814/renderer/index.html".into()))
                .disable_drag_drop_handler()
                .on_download(chat_tray::download_feedback)
                .title("ChatX")
                .inner_size(1220.0, 800.0)
                .min_inner_size(800.0, 580.0)
                .initialization_script(format!("window.__kcOfficeHost = {office_host};"))
                .center();
            window = window.icon(chat_tray::window_icon()?)?;
            let profile = match &electron_profile {
                Some(profile) => profile.renderer_config().map(Some),
                None => desktop_profile_import::load(&data_dir),
            };
            match profile {
                Ok(Some(mut profile)) => {
                    let gallery = app.state::<Arc<gallery_local::LocalGallery>>();
                    for folder in profile["galleryFolders"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                        gallery.allow(std::path::Path::new(folder));
                    }
                    window = window.initialization_script(format!("window.__kcLegacyDesktopProfile = {profile};"));
                }
                Ok(None) => {},
                Err(error) => {
                    let _ = support.record("storage", "desktop-profile-import-failed", &json!({"error":error.to_string()}));
                }
            }
            match renderer_preferences::initialization_script(&data_dir) {
                Ok(Some(script)) => window = window.initialization_script(script),
                Ok(None) => {},
                Err(error) => {
                    let _ = support.record("storage", "renderer-preferences-import-failed", &json!({"error":error.to_string()}));
                }
            }
            if smoke {
                if std::env::var_os("KC_TEST_AUTO_MOCKUP_ONLY").is_none() {
                    window = window.initialization_script("window.__kcHostSmoke = true;");
                }
                if std::env::var_os("KC_TEST_RENDERER_PREFERENCES").is_some() {
                    window = window.initialization_script("window.__kcTestRendererPreferences = true;");
                }
                if let Ok(folder) = std::env::var("KC_TEST_GALLERY_DIRECTORY") {
                    app.state::<Arc<gallery_download::Downloads>>().allow(std::path::Path::new(&folder))?;
                    window = window.initialization_script(format!("window.__kcGalleryFixture = {};",json!(folder)));
                }
                if let Ok(folder) = std::env::var("KC_TEST_GALLERY_SOURCE") {
                    window = window.initialization_script(format!("window.__kcGallerySource = {};",json!(folder)));
                }
            }
            let window = window.initialization_script(RENDERER_BRIDGE).build()?;
            chat_tray::install(app.handle(), "host")?;
            network::monitor_recovery(window);
            if !smoke && !cutover_smoke { work_schedule::start(app.handle().clone(), profile_dir.clone()); }
            let status = status.clone();
            let admin = admin.clone();
            let support = support.clone();
            let snapshot = snapshot.clone();
            let update_permit=update_permit.clone();
            tauri::async_runtime::spawn(async move {
                let launch = || -> std::io::Result<_> {
                    let lan = if auto_lan {
                        Some(host::LanBinding { address: network::primary_lan_address()?, ephemeral: false })
                    } else {
                        lan
                    };
                    let data = data_dir.clone();
                    std::fs::create_dir_all(&data)?;
                    std::env::set_current_dir(&data)?;
                    Ok((data, lan))
                };
                let result = match launch() {
                    Ok((data, lan)) => host::serve_with_provider(|port, discovery_port, lan_address| {
                        *status.lock().expect("status HOST") = json!({
                            "state":"running","address":format!("http://127.0.0.1:{port}"),
                            "discoveryPort":discovery_port,"dataPath":data,"adminToken":admin,
                            "lanAddress":lan_address.map(|address| format!("http://{address}")),
                        });
                        let details = json!({"port":port,"discoveryPort":discovery_port,"lanAddress":lan_address});
                        let _ = support.record("runtime","host-listening",&details);
                        if office_host && !cutover_smoke && !smoke {
                            let log = support.clone();
                            let root = data.clone();
                            tauri::async_runtime::spawn(async move {
                                loop {
                                    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                                    let root = root.clone();
                                    let result = tauri::async_runtime::spawn_blocking(move || {
                                        let uploads = host::storage_location::upload_dir(&root)?;
                                        host::host_compression::background_io(|| host::host_compression::compress_idle_upload(&uploads))
                                    }).await;
                                    if let Ok(Err(error)) = result {
                                        let _ = log.record("storage","upload-compression-failed",
                                            &json!({"error":error.to_string()}));
                                        if matches!(error.raw_os_error(), Some(1 | 50 | 87)) { break; }
                                    }
                                }
                            });
                        }
                        if (smoke || cutover_smoke) && lan_address.is_some() {
                            println!("HOST_LAN_SMOKE {details}");
                        }
                        Ok(())
                    }, admin.clone(), snapshot, update_permit, lan, reserved).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    let _ = support.record("runtime","host-listener-failed",&json!({"error":error.to_string()}));
                    *status.lock().expect("status HOST") = json!({"state":"failed","error":error.to_string()});
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "host" && window.app_handle().tray_by_id("chatx-tray").is_some() {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if window.hide().is_err() { let _ = window.minimize(); }
                }
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let log = window.app_handle().state::<Arc<support_log::SupportLog>>();
                let _ = log.record("runtime","window-destroyed",&json!({"window":window.label()}));
            }
            if (window.label() == "host") && matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().state::<Arc<notification::Notifications>>().close_all(window.app_handle(), window.label());
            }
            if window.label() == "host" && matches!(event, tauri::WindowEvent::Destroyed) {
            }
        })
        .run(context)?;
    Ok(())
}
