use ed25519_dalek::{pkcs8::DecodePublicKey, VerifyingKey};
use serde_json::{json, Value};
use std::{fs, io::{self, Read, Write}, os::windows::ffi::OsStrExt, path::{Path, PathBuf}};
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

const MAX_CONFIG: u64 = 1024 * 1024;

pub(crate) struct ProfileStore(PathBuf);

impl ProfileStore {
    pub fn new(directory: PathBuf) -> Self { Self(directory.join("client-identity.dpapi")) }

    pub fn import(&self, source: &Path) -> Result<(), String> {
        self.import_electron(source, false)
    }

    pub fn migrate(&self, source: &Path) -> Result<(), String> {
        self.import_electron(source, true)
    }

    pub fn migrate_legacy(&self, root: &Path) -> Result<(), String> {
        check_path(root)?;
        let config = root.join("config.json");
        check_path(&config)?;
        self.migrate(&config)
    }

    pub fn check_legacy_cleanup(&self, root: &Path) -> Result<(), String> {
        self.migrate_legacy(root)?;
        if root.join("server-data").try_exists().map_err(|error| error.to_string())? {
            return Err("Profil Electron juga menyimpan data server. Aplikasi dan data Electron dipertahankan.".into());
        }
        let outbox = root.join("outbox");
        if outbox.exists() {
            check_path(&outbox)?;
            for entry in fs::read_dir(outbox).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                check_path(&path)?;
                if path.extension().is_some_and(|ext| ext == "json") {
                    let bytes = read(&path).map_err(|error| error.to_string())?;
                    if !serde_json::from_slice::<Value>(&bytes).is_ok_and(|value|
                        value.as_array().is_some_and(Vec::is_empty)) {
                        return Err("Masih ada antrean pesan Electron. Kirim pesan yang tertunda sebelum membersihkan profil.".into());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn cleanup_legacy(&self, root: &Path) -> Result<(), String> {
        self.check_legacy_cleanup(root)?;
        fs::remove_dir_all(root).map_err(|error| error.to_string())
    }

    fn import_electron(&self, source: &Path, allow_matching_identity: bool) -> Result<(), String> {
        let mut bytes = read(source).map_err(|error| error.to_string())?;
        let parsed = serde_json::from_slice::<Value>(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes));
        bytes.fill(0);
        let parsed = parsed.map_err(|_| "Config Electron tidak valid")?;
        let root = source.parent().ok_or("Folder profil Electron tidak valid")?;
        let legacy_role_missing = (parsed["role"].is_null()
            || parsed["role"].as_str().is_some_and(|role| role.trim().is_empty()))
            && !root.join("server-data").try_exists().map_err(|error| error.to_string())?;
        let role = parsed["role"].as_str().unwrap_or("").trim();
        if matches!(role.to_ascii_lowercase().as_str(), "server" | "host") {
            return Err("Folder Electron berisi profil HOST. Jalankan installer HOST di PC server, atau jalankan installer CLIENT di akun Windows yang memakai Electron CLIENT.".into());
        }
        if !(role.eq_ignore_ascii_case("client") || allow_matching_identity && legacy_role_missing) {
            if root.join("server-data").try_exists().map_err(|error| error.to_string())? {
                return Err("Folder Electron menyimpan data server dan tidak memiliki role CLIENT. Profil perlu diperiksa sebelum dipindahkan.".into());
            }
            return Err("Role profil Electron belum dikenali sebagai CLIENT. Buka Electron CLIENT di akun Windows ini dan periksa profilnya.".into());
        }
        let mut parsed = parsed;
        if parsed["galleryFolders"].is_null() {
            let cache = root.join("gallery-local-index-v3.json");
            if let Ok(bytes) = read(&cache) {
                if let Ok(index) = serde_json::from_slice::<Value>(&bytes) {
                    parsed["galleryFolders"] = index["roots"].clone();
                }
            }
        }
        let identity = validate(&parsed)?;
        if allow_matching_identity {
            if let Some(existing) = self.load()? {
                if ["name", "deviceId", "deviceKey"].iter().all(|&field| existing[field] == identity[field]) {
                    return Ok(());
                }
                return Err("Profil Rust yang tersimpan berbeda dari akun Electron. Migrasi dihentikan agar akses chat lama tetap tersedia.".into());
            }
        }
        if self.0.try_exists().map_err(|error| error.to_string())? {
            return Err("Identitas CLIENT sudah tersimpan.".into());
        }
        let directory = self.0.parent().ok_or("Folder profil CLIENT tidak valid")?;
        import_trust(directory, &parsed["trustedHostKeys"]).map_err(|error| {
            format!("Kunci HOST dari profil Electron gagal diimpor: {error}")
        })?;
        crate::chat_wallpaper::import_legacy(source, directory)
            .map_err(|error| format!("Wallpaper Electron gagal diimpor: {error}"))?;
        self.save_new(&identity)
    }

    fn save_new(&self, identity: &Value) -> Result<(), String> {
        let mut plain = serde_json::to_vec(&identity).map_err(|error| error.to_string())?;
        let encrypted = crate::windows_secret::protect(&plain, true);
        plain.fill(0);
        let encrypted = encrypted.map_err(|error| format!("Windows gagal melindungi identitas CLIENT: {error}"))?;
        write_new(&self.0, &encrypted).map_err(|error| format!("Identitas CLIENT tidak dapat disimpan: {error}"))
    }

    pub fn load(&self) -> Result<Option<Value>, String> {
        let bytes = match read(&self.0) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let mut plain = crate::windows_secret::protect(&bytes, false)
            .map_err(|_| "Identitas CLIENT tidak bisa dibuka pada akun Windows ini")?;
        let parsed = serde_json::from_slice::<Value>(&plain);
        plain.fill(0);
        validate(&parsed.map_err(|_| "Identitas CLIENT rusak")?).map(Some)
    }
}

fn check_path(path: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("Lokasi data Electron memakai tautan Windows.".into());
    }
    Ok(())
}

fn import_trust(directory: &Path, source: &Value) -> io::Result<()> {
    let Some(keys) = source.as_object() else { return Ok(()); };
    let mut trusted = serde_json::Map::new();
    for (origin, key) in keys {
        let Ok(address) = reqwest::Url::parse(origin) else { continue; };
        if address.scheme() != "http" || address.origin().ascii_serialization() != *origin
            || !address.host_str().and_then(|host| host.parse::<std::net::Ipv4Addr>().ok())
                .is_some_and(|ip| ip.is_private() || ip.is_loopback())
        {
            continue;
        }
        if key.as_str().is_some_and(|pem| pem.len() <= 2048 && VerifyingKey::from_public_key_pem(pem).is_ok()) {
            trusted.insert(origin.clone(), key.clone());
        }
    }
    if trusted.is_empty() { return Ok(()); }
    match write_new(&directory.join("trusted-hosts.json"), &serde_json::to_vec(&trusted)?) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        result => result,
    }
}

fn write_new(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = destination.parent().ok_or_else(|| io::Error::other("Folder profil CLIENT tidak valid"))?;
    fs::create_dir_all(parent)?;
    let staged = parent.join(format!(".client-profile-{}.tmp", uuid::Uuid::new_v4()));
    crate::host::atomic_write(&staged, |file| file.write_all(bytes))?;
    let wide = |path: &Path| path.as_os_str().encode_wide().chain([0]).collect::<Vec<_>>();
    let source = wide(&staged);
    let destination = wide(destination);
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        let error = io::Error::last_os_error();
        let _ = fs::remove_file(staged);
        return Err(error);
    }
    Ok(())
}

fn read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(MAX_CONFIG + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG { return Err(io::Error::new(io::ErrorKind::InvalidData, "File profil terlalu besar")); }
    Ok(bytes)
}

fn validate(config: &Value) -> Result<Value, String> {
    let name = config["name"].as_str().filter(|name| !name.trim().is_empty() && name.chars().count() <= 40)
        .ok_or("Nama CLIENT lama tidak valid")?;
    let id = config["deviceId"].as_str().filter(|id| !id.trim().is_empty() && id.chars().count() <= 120 && !id.chars().any(char::is_control))
        .ok_or("ID perangkat CLIENT lama tidak valid")?;
    let key = config["deviceKey"].as_str().filter(|key| key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("Kunci perangkat CLIENT lama tidak valid")?;
    let mut identity = json!({"name":name,"deviceId":id,"deviceKey":key});
    if let Some(folders) = config["galleryFolders"].as_array() {
        let folders: Vec<_> = folders.iter().filter_map(Value::as_str)
            .filter(|folder| Path::new(folder).is_absolute()).take(128).collect();
        identity["galleryFolders"] = json!(folders);
    }
    Ok(identity)
}

#[tauri::command]
pub fn client_device_identity(window: tauri::WebviewWindow, state: tauri::State<'_, ProfileStore>) -> Result<Option<Value>, String> {
    match window.label() {
        "main" => state.load(),
        _ => Err("Jendela CLIENT tidak valid.".into()),
    }
}

#[tauri::command]
pub fn create_client_identity(window: tauri::WebviewWindow, state: tauri::State<'_, ProfileStore>, name: String) -> Result<(), String> {
    if window.label() != "main" { return Err("Identitas CLIENT hanya tersedia di jendela utama".into()); }
    let identity = validate(&json!({"name":name.trim(),"deviceId":uuid::Uuid::new_v4().to_string(),
        "deviceKey":format!("{}{}",uuid::Uuid::new_v4().simple(),uuid::Uuid::new_v4().simple())}))?;
    state.save_new(&identity)
}
