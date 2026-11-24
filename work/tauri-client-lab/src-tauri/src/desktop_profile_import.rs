use serde_json::{json, Map, Value};
use std::{fs::{self, File}, io::{self, Read}, path::{Path, PathBuf}};

const FILE: &str = "desktop-profile-import.json";
const MAX_CONFIG: u64 = 1024 * 1024;

pub(crate) fn import(source: &Path, directory: &Path) -> io::Result<Value> {
    let config = read(source)?;
    if config["role"] != "server" {
        return Err(invalid("Pilih config.json dari profil Electron HOST"));
    }
    let profile = supported_fields(&config);
    fs::create_dir_all(directory)?;
    let target = directory.join(FILE);
    if target.try_exists()? {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Profil desktop sudah diimpor"));
    }
    crate::chat_wallpaper::import_legacy(source, directory)?;
    let staged = directory.join(format!(".profile-import-{}.tmp", uuid::Uuid::new_v4()));
    crate::host::atomic_write(&staged, |file| serde_json::to_writer(file, &profile).map_err(io::Error::other))?;
    if let Err(error) = crate::backup_space::move_no_replace(&staged, &target) {
        let _ = fs::remove_file(staged);
        return Err(error);
    }
    Ok(json!({"ok":true,"fieldCount":profile.as_object().expect("profile object").len()}))
}

pub(crate) fn load(directory: &Path) -> io::Result<Option<Value>> {
    match read(&directory.join(FILE)) {
        Ok(profile) => Ok(Some(supported_fields(&profile))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read(path: &Path) -> io::Result<Value> {
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_CONFIG + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG {
        return Err(invalid("File pengaturan desktop terlalu besar"));
    }
    let data: Value = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))?;
    if !data.is_object() {
        return Err(invalid("Format pengaturan desktop tidak valid"));
    }
    Ok(data)
}

fn supported_fields(config: &Value) -> Value {
    let mut profile = Map::new();
    for key in ["name", "teamRole"] {
        if let Some(value) = config[key].as_str().filter(|value| !value.trim().is_empty()) {
            profile.insert(key.into(), json!(value));
        }
    }
    if let Some(folders) = config["galleryFolders"].as_array() {
        let paths: Vec<_> = folders.iter().filter_map(Value::as_str)
            .filter(|path| Path::new(path).is_absolute()).collect();
        profile.insert("galleryFolders".into(), json!(paths));
    }
    if let Some(settings) = config["notificationSettings"].as_object() {
        let mut supported = Map::new();
        for (key, choices) in [
            ("position", &["top-right", "top-left", "bottom-right", "bottom-left", "center"][..]),
            ("normalSize", &["small", "medium", "large"][..]),
            ("mentionSize", &["medium", "large", "fullscreen"][..]),
            ("normalSound", &["none", "classic", "iphone", "bell", "pop"][..]),
            ("style", &["card", "floating-center"][..]),
        ] {
            if let Some(value) = settings.get(key).and_then(Value::as_str).filter(|value| choices.contains(value)) {
                supported.insert(key.into(), json!(value));
            }
        }
        profile.insert("notificationSettings".into(), Value::Object(supported));
    }
    if let Some(settings) = config["workTagPrefs"].as_object() {
        let mut supported = Map::new();
        for key in ["print-test", "design-revision", "design-request"] {
            if let Some(value) = settings.get(key).and_then(Value::as_bool) {
                supported.insert(key.into(), json!(value));
            }
        }
        profile.insert("workTagPrefs".into(), Value::Object(supported));
    }
    Value::Object(profile)
}

pub(crate) struct ElectronHostProfile {
    pub(crate) root: PathBuf,
    pub(crate) server_data: PathBuf,
}

impl ElectronHostProfile {
    pub(crate) fn open(root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let config = read(&root.join("config.json"))?;
        if config["role"] != "server" {
            return Err(invalid("Folder ini bukan profil Electron HOST"));
        }
        let server_data = root.join("server-data").canonicalize()?;
        if !server_data.is_dir() {
            return Err(invalid("Direktori server-data Electron tidak tersedia"));
        }
        Ok(Self { root, server_data })
    }

    pub(crate) fn renderer_config(&self) -> io::Result<Value> {
        let config = read(&self.root.join("config.json"))?;
        let mut profile = supported_fields(&config);
        let device = config["deviceId"].as_str().filter(|value| !value.trim().is_empty())
            .ok_or_else(|| invalid("ID perangkat Electron HOST tidak tersedia"))?;
        profile["electronHostProfileId"] = json!(device);
        Ok(profile)
    }

    pub(crate) fn inventory(&self) -> io::Result<Value> {
        fn present(directory: &Path, name: &str) -> io::Result<bool> {
            match fs::metadata(directory.join(name)) {
                Ok(metadata) => Ok(metadata.is_file()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error),
            }
        }

        Ok(json!({
            "profile": self.root,
            "serverData": self.server_data,
            "sqliteChat": present(&self.server_data, "chat-messages-v1.sqlite")?,
            "sqliteChatWal": present(&self.server_data, "chat-messages-v1.sqlite-wal")?,
            "sqliteGallery": present(&self.server_data, "gallery-index-v2.sqlite")?,
            "sqliteGalleryWal": present(&self.server_data, "gallery-index-v2.sqlite-wal")?,
            "externalStorageConfig": present(&self.server_data, "storage-location.json")?,
            "updateIdentity": present(&self.server_data, "update-signing-key.pem")?,
            "workScheduleState": present(&self.root, "work-schedule-events.json")?,
        }))
    }
}