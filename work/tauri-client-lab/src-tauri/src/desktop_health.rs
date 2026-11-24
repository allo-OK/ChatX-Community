use serde_json::{json, Value};
use std::{fs::{self, OpenOptions}, io::{self, Write}, path::PathBuf};

pub(crate) struct HealthStorage(pub PathBuf);

#[tauri::command]
pub async fn check_health_storage(window: tauri::WebviewWindow, storage: tauri::State<'_, HealthStorage>) -> Result<Value, String> {
    if !matches!(window.label(), "main" | "host") {
        return Err("Pemeriksaan sistem tersedia di jendela chat".into());
    }
    let directory = storage.0.clone();
    tokio::task::spawn_blocking(move || {
        let result = (|| -> io::Result<()> {
            fs::create_dir_all(&directory)?;
            let path = directory.join(format!(".kc-health-{}.tmp", uuid::Uuid::new_v4()));
            let mut file = OpenOptions::new().write(true).create_new(true).open(&path)?;
            let written = file.write_all(b"ok");
            drop(file);
            let removed = fs::remove_file(path);
            written.and(removed)
        })();
        match result {
            Ok(()) => json!({"ok":true,"detail":directory}),
            Err(error) => json!({"ok":false,"detail":error.to_string()}),
        }
    }).await.map_err(|error| error.to_string())
}
