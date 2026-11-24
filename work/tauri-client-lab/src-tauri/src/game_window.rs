use serde_json::Value;
use std::sync::Mutex;
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub struct GameWindowState {
    owner: &'static str,
    snapshot: Mutex<Value>,
}

impl GameWindowState {
    pub fn new(owner: &'static str) -> Self {
        Self { owner, snapshot: Mutex::new(Value::Null) }
    }
}

const BRIDGE: &str = r#"
(() => {
  const send = (kind, payload = null) =>
    window.__TAURI__.core.invoke('game_window_action', {kind, payload});
  window.gameAPI = {
    onState(callback) {
      window.addEventListener('kc-game-state', event => callback(event.detail));
      void send('snapshot').then(state => { if (state) callback(state); });
    },
    onFeedback(callback) {
      window.addEventListener('kc-game-feedback', event => callback(event.detail));
    },
    action: payload => send('action', payload),
    minimize: () => send('minimize'),
    close: () => send('close'),
  };
})();
"#;

#[tauri::command]
pub async fn open_game(owner: WebviewWindow, state: tauri::State<'_, GameWindowState>) -> Result<bool, String> {
    if owner.label() != state.owner {
        return Err("Buka game dari jendela chat.".into());
    }
    let app = owner.app_handle();
    if let Some(window) = app.get_webview_window("game") {
        window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
            .map_err(|error| error.to_string())?;
        return Ok(true);
    }
    let mut builder = WebviewWindowBuilder::new(
        app,
        "game",
        WebviewUrl::App("v814/renderer/game-window.html".into()),
    )
    .title("ChatX Game")
    .inner_size(720.0, 820.0)
    .min_inner_size(560.0, 620.0)
    .parent(&owner)
    .map_err(|error| error.to_string())?
    .initialization_script(BRIDGE);
    if state.owner == "main" {
        builder = builder.initialization_script("window.__kcClientGameWindow = true;");
    }
    if is_smoke() { builder = builder.initialization_script(include_str!("../../game-window-smoke.js")); }
    builder.build().map_err(|error| error.to_string())?;
    Ok(true)
}

fn is_smoke() -> bool {
    std::env::args_os().nth(1).is_some_and(|arg| arg == "--smoke-gui" || arg == "--smoke-client-renderer")
}

fn dispatch(window: &WebviewWindow, event: &str, payload: &Value) -> Result<(), String> {
    let payload = serde_json::to_string(&payload.to_string()).map_err(|error| error.to_string())?;
    window
        .eval(format!(
            "window.dispatchEvent(new CustomEvent('{event}',{{detail:JSON.parse({payload})}}));"
        ))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn game_window_sync(
    owner: WebviewWindow,
    kind: String,
    payload: Value,
    state: tauri::State<'_, GameWindowState>,
) -> Result<(), String> {
    if owner.label() != state.owner {
        return Err("Sumber game tidak valid.".into());
    }
    let event = match kind.as_str() {
        "state" => {
            *state.snapshot.lock().map_err(|_| "Status game tidak tersedia.")? = payload.clone();
            "kc-game-state"
        }
        "feedback" => {
            if payload["closed"] == true {
                *state.snapshot.lock().map_err(|_| "Status game tidak tersedia.")? = Value::Null;
            }
            "kc-game-feedback"
        }
        _ => return Err("Jenis pembaruan game tidak valid.".into()),
    };
    if let Some(window) = owner.app_handle().get_webview_window("game") {
        dispatch(&window, event, &payload)?;
    }
    Ok(())
}

#[tauri::command]
pub fn game_window_action(
    window: WebviewWindow,
    kind: String,
    payload: Value,
    state: tauri::State<'_, GameWindowState>,
) -> Result<Value, String> {
    if window.label() != "game" {
        return Err("Sumber aksi game tidak valid.".into());
    }
    match kind.as_str() {
        "snapshot" => {
            return Ok(state
                .snapshot
                .lock()
                .map_err(|_| "Status game tidak tersedia.")?
                .clone())
        }
        "minimize" => window.minimize().map_err(|error| error.to_string())?,
        "close" => window.close().map_err(|error| error.to_string())?,
        "smoke-client-mode" if is_smoke() => return Ok(Value::Bool(state.owner == "main")),
        "smoke-complete" if is_smoke() => {
            let owner = window.app_handle().get_webview_window(state.owner).ok_or("Jendela chat tidak tersedia.")?;
            dispatch(&owner, "kc-game-smoke-complete", &payload)?;
        }
        "action" => {
            let owner = window
                .app_handle()
                .get_webview_window(state.owner)
                .ok_or("Jendela chat tidak tersedia.")?;
            dispatch(&owner, "kc-game-action", &payload)?;
        }
        _ => return Err("Aksi game tidak valid.".into()),
    }
    Ok(Value::Null)
}
