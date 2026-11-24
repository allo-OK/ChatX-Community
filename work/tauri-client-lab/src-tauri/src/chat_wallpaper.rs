use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{fs, io::{self, Read, Write}, path::{Path, PathBuf}};

const LIMIT: u64 = 8 * 1024 * 1024;
const PREFIX: &str = "data:image/jpeg;base64,";

pub(crate) struct Wallpaper(PathBuf);

impl Wallpaper {
    pub fn new(profile: &Path) -> Self {
        Self(profile.join("appearance/chat-wallpaper.jpg"))
    }
}

fn valid_jpeg(bytes: &[u8]) -> bool {
    bytes.len() as u64 <= LIMIT && bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.ends_with(&[0xff, 0xd9])
}

pub(crate) fn import_legacy(config: &Path, profile: &Path) -> io::Result<()> {
    let Some(legacy_profile) = config.parent() else { return Ok(()); };
    let source = Wallpaper::new(legacy_profile).0;
    let target = Wallpaper::new(profile).0;
    if target.try_exists()? { return Ok(()); }
    let file = match fs::File::open(source) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if !valid_jpeg(&bytes) { return Ok(()); }
    let directory = target.parent().expect("wallpaper directory");
    fs::create_dir_all(directory)?;
    let staged = directory.join(format!(".wallpaper-import-{}.tmp", uuid::Uuid::new_v4()));
    crate::host::atomic_write(&staged, |file| file.write_all(&bytes))?;
    match crate::image_files::move_no_replace(&staged, &target) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(staged);
            if error.kind() == io::ErrorKind::AlreadyExists { Ok(()) } else { Err(error) }
        }
    }
}

fn perform(path: &Path, action: &str, data_url: Option<String>) -> Result<Value, String> {
    match action {
        "save" => {
            let encoded = data_url.as_deref().and_then(|value| value.strip_prefix(PREFIX))
                .filter(|value| value.len() <= (LIMIT as usize).div_ceil(3) * 4)
                .ok_or("Data wallpaper JPEG tidak valid atau terlalu besar")?;
            let bytes = STANDARD.decode(encoded).map_err(|_| "Data wallpaper tidak valid")?;
            if !valid_jpeg(&bytes) {
                return Err("Data wallpaper JPEG tidak valid atau terlalu besar".into());
            }
            fs::create_dir_all(path.parent().expect("wallpaper directory")).map_err(|e| e.to_string())?;
            crate::host::atomic_write(path, |file| file.write_all(&bytes)).map_err(|e| e.to_string())?;
            Ok(json!({"ok":true,"size":bytes.len(),"dataUrl":data_url}))
        }
        "load" => {
            let file = match fs::File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(json!({"ok":true,"exists":false})),
                Err(error) => return Err(error.to_string()),
            };
            let mut bytes = Vec::new();
            file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if !valid_jpeg(&bytes) {
                return Err("File wallpaper JPEG tidak valid atau terlalu besar".into());
            }
            Ok(json!({"ok":true,"exists":true,"dataUrl":format!("{PREFIX}{}",STANDARD.encode(bytes))}))
        }
        "clear" => {
            match fs::remove_file(path) {
                Ok(()) => {},
                Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                Err(error) => return Err(error.to_string()),
            }
            Ok(json!({"ok":true}))
        }
        _ => Err("Aksi wallpaper tidak valid".into()),
    }
}

#[tauri::command]
pub async fn chat_wallpaper(window: tauri::WebviewWindow, state: tauri::State<'_, Wallpaper>,
    action: String, data_url: Option<String>) -> Result<Value, String> {
    if !matches!(window.label(), "host" | "main") {
        return Err("Wallpaper hanya tersedia di jendela chat".into());
    }
    let path = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || perform(&path, &action, data_url)
        .unwrap_or_else(|error| json!({"ok":false,"error":error})))
        .await.map_err(|error| error.to_string())
}
