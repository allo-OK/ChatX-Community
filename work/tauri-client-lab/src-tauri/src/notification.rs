use serde_json::{json, Value};
use std::{collections::{HashMap, VecDeque}, sync::{Arc, Mutex}, time::{Duration, Instant}};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::{watch, Notify};

#[derive(Clone)]
struct Job {
    owner: String,
    payload: Value,
    settings: Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Loading, Visible, Closed }

struct Popup {
    job: Job,
    phase: watch::Sender<Phase>,
}

#[derive(Default)]
struct Lane {
    jobs: VecDeque<Job>,
    running: bool,
    changed: Arc<Notify>,
}

#[derive(Default)]
struct Inner {
    lane: Lane,
    popups: HashMap<String, Popup>,
    seen: VecDeque<(String, Instant)>,
}

#[derive(Default)]
pub struct Notifications(Mutex<Inner>);

impl Notifications {
    pub fn enqueue(self: &Arc<Self>, app: &AppHandle, owner: &str, payload: Value, settings: Value) -> Value {
        if !payload.is_object() || payload.to_string().len() > 256 * 1024 {
            return json!({"ok":false,"error":"Data notifikasi tidak valid."});
        }
        let id = payload["messageId"].as_str().unwrap_or_default();
        let job = Job { owner: owner.into(), payload: payload.clone(), settings };
        let start = {
            let mut state = self.0.lock().expect("notification state");
            let now = Instant::now();
            state.seen.retain(|(_, at)| now.duration_since(*at) < Duration::from_secs(120));
            if !id.is_empty() {
                if state.seen.iter().any(|(seen, _)| seen == id) { return json!({"ok":true,"duplicate":true}); }
                if state.seen.len() >= 2000 { state.seen.pop_front(); }
                state.seen.push_back((id.into(), now));
            }
            let lane = &mut state.lane;
            if let Some(pending) = lane.jobs.iter_mut().find(|pending| payload["mention"] != true
                && pending.payload["mention"] != true
                && payload["kind"] != "game-invite" && pending.payload["kind"] != "game-invite"
                && pending.owner == job.owner
                && pending.payload["conversationId"] == payload["conversationId"]) {
                *pending = job;
            } else if full_mention(&payload) {
                let next = lane.jobs.iter().position(|pending| !full_mention(&pending.payload)).unwrap_or(lane.jobs.len());
                lane.jobs.insert(next, job);
            } else if payload["mention"] == true {
                let next = lane.jobs.iter().position(|pending| pending.payload["mention"] != true).unwrap_or(lane.jobs.len());
                lane.jobs.insert(next, job);
            } else {
                lane.jobs.push_back(job);
            }
            if lane.jobs.len() > 100 { lane.jobs.pop_front(); }
            lane.changed.notify_one();
            !std::mem::replace(&mut lane.running, true)
        };
        if start {
            let state = Arc::clone(self);
            let app = app.clone();
            tauri::async_runtime::spawn(async move { state.run(app).await; });
        }
        json!({"ok":true,"accepted":true})
    }

    async fn run(self: Arc<Self>, app: AppHandle) {
        loop {
            let (job, changed) = {
                let mut state = self.0.lock().expect("notification state");
                let lane = &mut state.lane;
                let Some(job) = lane.jobs.pop_front() else { lane.running = false; return };
                (job, Arc::clone(&lane.changed))
            };
            if !self.present(&app, &job, &changed).await {
                let mut recovery = job.clone();
                recovery.payload["tagVideoEnabled"] = json!(false);
                recovery.payload["mentionLevel"] = json!("notice");
                recovery.payload["messages"] = json!([{"text":job.payload["body"]}]);
                recovery.payload["imageUrl"] = Value::Null;
                recovery.payload["_quiet"] = json!(true);
                let _ = self.present(&app, &recovery, &changed).await;
            }
        }
    }

    async fn present(&self, app: &AppHandle, job: &Job, changed: &Notify) -> bool {
        let index = usize::from(job.payload["mention"] == true);
        let label = format!("notification-{index}-{}", uuid::Uuid::new_v4().simple());
        let (sender, mut phase) = watch::channel(Phase::Loading);
        self.0.lock().expect("notification state").popups.insert(label.clone(), Popup { job: job.clone(), phase: sender.clone() });
        let popup = make_window(app, &label, job);
        let shown = if let Ok(window) = popup {
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) { sender.send_replace(Phase::Closed); }
            });
            let ready = tokio::time::timeout(Duration::from_secs(12), phase.wait_for(|phase| *phase != Phase::Loading))
                .await.is_ok_and(|result| result.is_ok_and(|phase| *phase == Phase::Visible));
            if ready {
                let shown_at = tokio::time::Instant::now();
                let normal = if job.payload["mention"] != true { 9000 } else if full_mention(&job.payload) { 15000 } else { 18000 };
                loop {
                    let pending = {
                        let state = self.0.lock().expect("notification state");
                        if index == 0 { !state.lane.jobs.is_empty() }
                        else { state.lane.jobs.front().is_some_and(|next| next.payload["mention"] == true) }
                    };
                    let duration = if pending {
                        if index == 0 && job.payload["kind"] != "game-invite" { 0 }
                        else if index == 0 { 1500 } else { 5000 }
                    } else { normal };
                    tokio::select! {
                        _ = phase.wait_for(|phase| *phase == Phase::Closed) => break,
                        _ = tokio::time::sleep_until(shown_at + Duration::from_millis(duration)) => break,
                        _ = changed.notified() => {},
                    }
                }
            }
            let _ = window.hide();
            let _ = window.destroy();
            ready
        } else { false };
        self.0.lock().expect("notification state").popups.remove(&label);
        shown
    }

    pub fn close_all(&self, app: &AppHandle, owner: &str) {
        let mut state = self.0.lock().expect("notification state");
        state.lane.jobs.retain(|job| job.owner != owner);
        for (label, popup) in &state.popups {
            if popup.job.owner == owner {
                popup.phase.send_replace(Phase::Closed);
                if let Some(window) = app.get_webview_window(label) { let _ = window.hide(); }
            }
        }
    }
}

fn full_mention(payload: &Value) -> bool {
    payload["mention"] == true && (payload["mentionLevel"] == "fullscreen" || payload["tagVideoEnabled"] == true)
}

fn make_window(app: &AppHandle, label: &str, job: &Job) -> tauri::Result<WebviewWindow> {
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App("v814/renderer/rust-notification.html".into()))
        .title("ChatX").visible(false).focused(false).decorations(false).shadow(false).transparent(true)
        .resizable(false).always_on_top(true).skip_taskbar(true);
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "--smoke-gui" || arg == "--smoke-client-renderer") {
        builder = builder.initialization_script("window.__kcNotificationSmoke = true;");
    }
    let popup = builder.build()?;
    if let Ok(Some(monitor)) = popup.primary_monitor() {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let bledar = full_mention(&job.payload) && job.payload["tagVideoEnabled"] != false;
        let size = if job.payload["mention"] == true {
            match job.payload["mentionLevel"].as_str() { Some("medium") => "large", Some("standard") => "small", _ => "medium" }
        } else { job.settings["normalSize"].as_str().unwrap_or("medium") };
        let (width, height) = match size { "small" => (340, 240), "large" => (440, 320), _ => (360, 260) };
        let mut width = (width as f64 * scale).round() as i32;
        let mut height = (height as f64 * scale).round() as i32;
        let gap = (20.0 * scale).round() as i32;
        let (aw, ah) = (area.size.width as i32, area.size.height as i32);
        if bledar {
            width = (480.0 * scale).round() as i32;
            height = (280.0 * scale).round() as i32;
        } else {
            let items = job.payload["messages"].as_array();
            let count = items.map_or(1, |rows| rows.len().clamp(1, 4));
            let images = items.map_or(0, |rows| rows.iter().rev().take(4).filter(|row| row["imageUrl"].is_string()).count());
            height = height.min(((190 + count * 60 + images * 96).max(248) as f64 * scale).round() as i32);
        }
        width = width.min((aw - gap).max(1));
        height = height.min((ah - gap).max(1));
        let pos = job.settings["position"].as_str().unwrap_or("top-right");
        let (mut x, mut y) = match pos {
            "top-left" => (gap, gap),
            "bottom-left" => (gap, ah - height - gap),
            "bottom-right" => (aw - width - gap, ah - height - gap),
            "center" => ((aw - width) / 2, (ah - height) / 2),
            _ => (aw - width - gap, gap),
        };
        let other = if job.payload["mention"] == true { "notification-0-" } else { "notification-1-" };
        for window in app.webview_windows().values().filter(|window| window.label().starts_with(other)) {
            if let (Ok(position), Ok(size)) = (window.outer_position(), window.outer_size()) {
                let other_y = position.y - area.position.y;
                let below = other_y + size.height as i32 + gap / 2;
                let above = other_y - height - gap / 2;
                if x < position.x - area.position.x + size.width as i32 && x + width > position.x - area.position.x
                    && y < other_y + size.height as i32 && y + height > other_y {
                    y = if below + height <= ah { below } else if above >= 0 { above } else { y };
                }
            }
        }
        x += area.position.x;
        y += area.position.y;
        let _ = popup.set_size(PhysicalSize::new(width as u32, height as u32));
        let _ = popup.set_position(PhysicalPosition::new(x, y));
    }
    Ok(popup)
}

fn notify_owner(app: &AppHandle, job: &Job, action: &str, text: Value) -> Result<(), String> {
    let owner = app.get_webview_window(&job.owner).ok_or("Jendela chat sudah ditutup.")?;
    if action == "focus" && job.payload["kind"] == "game-invite" {
        return owner.eval("window.dispatchEvent(new Event('kc-focus-game-invite'));")
            .map_err(|error| error.to_string());
    }
    let data = json!({"messageId":job.payload["messageId"],"conversationId":job.payload["conversationId"],"text":text});
    owner.eval(format!("window.dispatchEvent(new CustomEvent('kc-notification-{action}', {{detail:{data}}}));"))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn notification_snapshot(window: WebviewWindow, state: tauri::State<'_, Arc<Notifications>>) -> Result<Value, String> {
    let inner = state.0.lock().expect("notification state");
    let popup = inner.popups.get(window.label()).ok_or("Notifikasi sudah ditutup.")?;
    Ok(json!({"payload":popup.job.payload,"settings":popup.job.settings}))
}

#[tauri::command]
pub fn notification_ready(window: WebviewWindow, state: tauri::State<'_, Arc<Notifications>>) -> Result<(), String> {
    let inner = state.0.lock().expect("notification state");
    let popup = inner.popups.get(window.label()).ok_or("Notifikasi sudah ditutup.")?;
    if *popup.phase.borrow() != Phase::Loading { return Ok(()); }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        };
        let handle = window.hwnd().map_err(|error| error.to_string())?;
        if unsafe {
            SetWindowPos(handle.0 as _, HWND_TOPMOST, 0, 0, 0, 0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW)
        } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    #[cfg(not(target_os = "windows"))]
    window.show().map_err(|error| error.to_string())?;
    popup.phase.send_replace(Phase::Visible);
    Ok(())
}

#[tauri::command]
pub fn notification_action(window: WebviewWindow, state: tauri::State<'_, Arc<Notifications>>, action: String, text: Option<String>) -> Result<(), String> {
    let inner = state.0.lock().expect("notification state");
    let popup = inner.popups.get(window.label()).ok_or("Notifikasi sudah ditutup.")?;
    if *popup.phase.borrow() != Phase::Visible { return Err("Notifikasi sudah ditutup.".into()); }
    match action.as_str() {
        "close" => {},
        "focus" => {
            let job = popup.job.clone();
            let phase = popup.phase.clone();
            drop(inner);
            notify_owner(window.app_handle(), &job, "focus", Value::Null)?;
            phase.send_replace(Phase::Closed);
            let _ = window.hide();
            if let Some(owner) = window.app_handle().get_webview_window(&job.owner) {
                let _ = owner.unminimize();
                let _ = owner.show();
                let _ = owner.set_focus();
            }
            return Ok(());
        }
        "reply" => {
            if popup.job.payload["kind"] == "game-invite" { return Err("Buka undangan untuk menanggapi.".into()); }
            let text = text.as_deref().unwrap_or_default().trim();
            if text.is_empty() || text.chars().count() > 4000 { return Err("Balasan kosong atau terlalu panjang.".into()); }
            notify_owner(window.app_handle(), &popup.job, "reply", json!(text))?;
        }
        _ => return Err("Aksi notifikasi tidak dikenal.".into()),
    }
    popup.phase.send_replace(Phase::Closed);
    Ok(())
}
