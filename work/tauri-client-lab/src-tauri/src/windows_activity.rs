use serde_json::{json, Value};
use std::{path::Path, time::{Duration, Instant}};
use windows::{
    Media::Control::{GlobalSystemMediaTransportControlsSessionManager as SessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Playback},
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId},
};

#[derive(Default)]
pub(crate) struct LocalActivity(tokio::sync::Mutex<Option<(Instant, Value)>>);

#[tauri::command]
pub async fn get_local_activity(window: tauri::WebviewWindow, activity: tauri::State<'_, LocalActivity>) -> Result<Value, String> {
    if !matches!(window.label(), "host" | "main") {
        return Err("Aktivitas Windows tersedia di jendela chat".into());
    }
    let mut cached = activity.0.lock().await;
    if let Some((time, value)) = cached.as_ref() {
        if time.elapsed() < Duration::from_secs(10) { return Ok(value.clone()); }
    }
    let runtime = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        let (app, title) = foreground();
        let mut data = json!({"app":app,"title":title,"mediaTitle":"","mediaArtist":"","mediaAlbum":"","mediaSource":"","playback":""});
        if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok() {
            let _apartment = Apartment;
            if let Ok(Ok(media)) = runtime.block_on(async { tokio::time::timeout(Duration::from_millis(1800), media()).await }) {
                data.as_object_mut().expect("activity object").extend(media);
            }
        }
        json!({"ok":true,"activity":data})
    }).await.map_err(|error| error.to_string())?;
    *cached = Some((Instant::now(), result.clone()));
    Ok(result)
}

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) { unsafe { RoUninitialize(); } }
}

fn clean(text: &str, length: usize) -> String {
    text.replace(['\r', '\n', '\t'], " ").trim().chars().take(length).collect()
}

fn foreground() -> (String, String) {
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() { return ("ChatX".into(), String::new()); }
    let mut title = [0; 261];
    let length = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
    let title = clean(&String::from_utf16_lossy(&title[..length.max(0) as usize]), 260);
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid); }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() { return (String::new(), title); }
    let mut path = [0; 32768];
    let mut length = path.len() as u32;
    let found = unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) };
    unsafe { CloseHandle(process); }
    if found == 0 { return (String::new(), title); }
    let app = String::from_utf16_lossy(&path[..length as usize]);
    let name = Path::new(&app).file_stem().and_then(|part| part.to_str()).unwrap_or_default();
    (clean(name, 80), title)
}

async fn media() -> windows::core::Result<serde_json::Map<String, Value>> {
    let manager = SessionManager::RequestAsync()?.await?;
    let session = manager.GetCurrentSession()?;
    let properties = session.TryGetMediaPropertiesAsync()?.await?;
    let playback = session.GetPlaybackInfo()?.PlaybackStatus()?;
    let playback = match playback {
        Playback::Closed => "Closed",
        Playback::Opened => "Opened",
        Playback::Changing => "Changing",
        Playback::Stopped => "Stopped",
        Playback::Playing => "Playing",
        Playback::Paused => "Paused",
        _ => "",
    };
    Ok([
        ("mediaTitle", clean(&properties.Title()?.to_string_lossy(), 260)),
        ("mediaArtist", clean(&properties.Artist()?.to_string_lossy(), 160)),
        ("mediaAlbum", clean(&properties.AlbumTitle()?.to_string_lossy(), 160)),
        ("mediaSource", clean(&session.SourceAppUserModelId()?.to_string_lossy(), 180)),
        ("playback", playback.into()),
    ].into_iter().map(|(key, value)| (key.into(), json!(value))).collect())
}
