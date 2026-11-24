use serde_json::{json, Value};
use std::{fs, path::PathBuf, time::{Duration, SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};

fn event(day: u16, hour: u16, minute: u16) -> Option<&'static str> {
    match (day, hour, minute) {
        (_, 17, 0) => Some("workday-end"),
        (5, 11, 30) | (1..=4 | 6, 12, 0) => Some("break-start"),
        (5, 13, 30) | (1..=4 | 6, 13, 0) => Some("break-end"),
        _ => None,
    }
}

pub(crate) fn start(app: AppHandle, directory: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let path = directory.join("work-schedule-events.json");
        let mut fired = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|data| data["keys"].as_array().cloned()).unwrap_or_default();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let mut local = SYSTEMTIME::default();
            unsafe { GetLocalTime(&mut local); }
            let Some(kind) = event(local.wDayOfWeek, local.wHour, local.wMinute) else { continue; };
            let key = json!(format!("{:04}-{:02}-{:02}:{kind}", local.wYear, local.wMonth, local.wDay));
            if fired.contains(&key) { continue; }
            fired.push(key);
            if fired.len() > 60 { fired.drain(..fired.len()-60); }
            let value = json!({"keys":fired,"updatedAt":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()});
            let _ = fs::create_dir_all(&directory).and_then(|_| crate::host::atomic_write(&path,
                |file| serde_json::to_writer(file, &value).map_err(std::io::Error::other)));
            let _ = show(&app, kind, local.wDayOfWeek).await;
        }
    });
}

async fn show(app: &AppHandle, kind: &str, weekday: u16) -> tauri::Result<()> {
    if let Some(previous) = app.get_webview_window("work-schedule") {
        previous.destroy()?;
        for _ in 0..20 {
            if app.get_webview_window("work-schedule").is_none() { break; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    let back = if weekday == 5 { "13.30" } else { "13.00" };
    let (width,height) = if kind == "workday-end" { (1000.0,690.0) } else { (560.0,300.0) };
    WebviewWindowBuilder::new(app, "work-schedule", WebviewUrl::App(
        format!("v814/renderer/work-schedule-popup.html?kind={kind}&back={back}").into()))
        .title("ChatX · Jadwal Kerja").inner_size(width,height).center()
        .decorations(false).shadow(false).transparent(true).focused(false)
        .always_on_top(true).resizable(false)
        .build()?;
    Ok(())
}

#[tauri::command]
pub fn close_work_schedule(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "work-schedule" { return Err("Jendela jadwal tidak valid".into()); }
    window.close().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::event;

    #[test]
    fn matches_legacy_schedule_for_each_weekday() {
        for day in 0..=6 {
            for hour in 0..24 {
                for minute in 0..60 {
                    let expected = if (hour, minute) == (17, 0) { Some("workday-end") }
                        else if day == 0 { None }
                        else if (hour, minute) == if day == 5 { (11, 30) } else { (12, 0) } { Some("break-start") }
                        else if (hour, minute) == if day == 5 { (13, 30) } else { (13, 0) } { Some("break-end") }
                        else { None };
                    assert_eq!(event(day,hour,minute),expected);
                }
            }
        }
    }
}
