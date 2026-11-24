use crate::{crop_faces, image_files, upscale};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tauri::Manager;

#[tauri::command]
pub fn set_native_theme(window: tauri::WebviewWindow, theme: &str) -> Result<Value, String> {
    if !matches!(window.label(), "main" | "host") {
        return Err("Tema native diatur dari jendela chat".into());
    }
    let theme = match theme {
        "dark" => tauri::Theme::Dark,
        "light" => tauri::Theme::Light,
        _ => return Ok(json!({"ok":false,"error":"Tema tidak valid"})),
    };
    window.app_handle().set_theme(Some(theme));
    Ok(json!({"ok":true}))
}

pub struct UpscaleJob {
    active: tokio::sync::Mutex<()>,
    progress: Arc<Mutex<Value>>,
}

impl Default for UpscaleJob {
    fn default() -> Self {
        Self { active: tokio::sync::Mutex::new(()), progress: Arc::new(Mutex::new(json!({"pct":0,"text":""}))) }
    }
}

#[tauri::command]
pub fn upscale_status() -> Value {
    upscale::detect().map_or_else(
        || json!({"ok":false,"error":"Upscayl belum ditemukan di Windows."}), |engine| engine.status(),
    )
}

#[tauri::command]
pub fn upscale_progress(job: tauri::State<'_, UpscaleJob>) -> Value {
    job.progress.lock().expect("upscale progress").clone()
}

#[tauri::command]
pub async fn upscale_image(job: tauri::State<'_, UpscaleJob>, input: Value) -> Result<Value, String> {
    let _active = job.active.lock().await;
    let Some(engine) = upscale::detect() else { return Ok(upscale_status()) };
    let progress = job.progress.clone();
    let result = tokio::task::spawn_blocking(move || {
        engine.run(input, |pct, text| {
            *progress.lock().expect("upscale progress") = json!({"pct":pct,"text":text});
        })
    }).await.unwrap_or_else(|error| Err(error.to_string()));
    Ok(result.unwrap_or_else(|error| {
        *job.progress.lock().expect("upscale progress") = json!({"pct":0,"text":"Gagal"});
        json!({"ok":false,"error":error})
    }))
}

#[tauri::command]
pub async fn choose_image_files(window: tauri::WebviewWindow, multiple: bool) -> Result<Vec<std::path::PathBuf>, String> {
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    tokio::task::spawn_blocking(move || image_files::choose_images(owner, multiple))
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn choose_upscale_output(window: tauri::WebviewWindow) -> Result<Option<std::path::PathBuf>, String> {
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    tokio::task::spawn_blocking(move || image_files::choose_folder(owner)).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn choose_vector_output(
    window: tauri::WebviewWindow, outputs: tauri::State<'_, Arc<image_files::PngOutputs>>,
) -> Result<Option<std::path::PathBuf>, String> {
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    let outputs = Arc::clone(outputs.inner());
    tokio::task::spawn_blocking(move || {
        let folder = image_files::choose_folder(owner)?;
        if let Some(path) = &folder { outputs.allow(path)?; }
        Ok(folder)
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn write_base64_file(outputs: tauri::State<'_, Arc<image_files::PngOutputs>>, input: Value) -> Result<Value, String> {
    let outputs = Arc::clone(outputs.inner());
    tokio::task::spawn_blocking(move || {
        let path = std::path::Path::new(input["path"].as_str().unwrap_or_default());
        outputs.write(path, input["base64"].as_str().unwrap_or_default())
            .unwrap_or_else(|error| json!({"ok":false,"error":error}))
    }).await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_png(window: tauri::WebviewWindow, input: Value) -> Result<Value, String> {
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    tokio::task::spawn_blocking(move || image_files::save_png(owner, &input)
        .unwrap_or_else(|error| json!({"ok":false,"error":error})))
        .await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn detect_crop_faces(png: String) -> Result<Vec<Value>, String> {
    tokio::task::spawn_blocking(move || crop_faces::detect(&png))
        .await.map_err(|_| "Deteksi wajah Windows berhenti.".to_owned())?
}

#[tauri::command]
pub async fn write_vector_output(outputs: tauri::State<'_, Arc<image_files::PngOutputs>>, input: Value) -> Result<Value, String> {
    let outputs = Arc::clone(outputs.inner());
    tokio::task::spawn_blocking(move || outputs.write_vector(&input)
        .unwrap_or_else(|error| json!({"ok":false,"error":error})))
        .await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn read_local_image(path: std::path::PathBuf, preview: bool) -> Value {
    match tokio::task::spawn_blocking(move || image_files::read_image(&path, preview)).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => json!({"ok":false,"error":error}),
        Err(_) => json!({"ok":false,"error":"Gambar gagal dimuat"}),
    }
}

#[tauri::command]
pub async fn open_image_path(window: tauri::WebviewWindow, path: std::path::PathBuf) -> Result<Value, String> {
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    tokio::task::spawn_blocking(move || image_files::open_path(owner, &path)).await.map_err(|error| error.to_string())?
}
