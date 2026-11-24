use ed25519_dalek::{pkcs8::DecodePublicKey, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Emitter, State, WebviewWindow};
use tokio::io::AsyncWriteExt;

type Result<T> = std::result::Result<T, String>;
const MAX_STATUS: usize = 128 * 1024;
const LOCAL_FEATURES: [&str; 3] = [ "designAI", "downloadFiles", "banned"];

fn local_permission_flags(source: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    LOCAL_FEATURES.into_iter().filter_map(|key|
        source.get(key).and_then(Value::as_bool).map(|allowed| (key.into(), json!(allowed))))
        .collect()
}

#[tauri::command]
pub async fn refresh_client_feature_policy(window: WebviewWindow, updater: State<'_, Arc<ClientUpdater>>,
    profile: State<'_, crate::client_profile::ProfileStore>) -> Result<Value> {
    if window.label() != "main" { return Err("Pemeriksaan izin tersedia di jendela CLIENT".into()); }
    let identity = profile.load()?.unwrap_or(Value::Null);
    Ok(updater.local_permissions(&identity).await)
}

pub struct ClientUpdater {
    trust: PathBuf,
    downloads: PathBuf,
    version: String,
    origin: Mutex<Option<String>>,
    token: Mutex<Option<String>>,
    ready: Mutex<Option<(PathBuf, u64, String)>>,
    transfer: tokio::sync::Mutex<()>,
    fixture: bool,
    rust_channel: bool,
}

impl ClientUpdater {
    pub(crate) async fn authorize_local(&self, profile: &crate::client_profile::ProfileStore, feature: &str, label: &str) -> Result<()> {
        let identity = profile.load()?.unwrap_or(Value::Null);
        let permissions = self.local_permissions(&identity).await;
        Self::check_permission(&permissions, feature, label)
    }

    pub(crate) fn authorize_cached_local(&self, profile: &crate::client_profile::ProfileStore, feature: &str, label: &str) -> Result<()> {
        let identity = profile.load()?.unwrap_or(Value::Null);
        let permissions = identity["deviceId"].as_str().map(|device| self.cached_local_permissions(device)).unwrap_or(Value::Null);
        Self::check_permission(&permissions, feature, label)
    }

    fn check_permission(permissions: &Value, feature: &str, label: &str) -> Result<()> {
        if permissions["banned"] == true || permissions[feature] == false {
            return Err(format!("Akses {label} dibatasi HOST"));
        }
        Ok(())
    }

    fn cached_local_permissions(&self, device: &str) -> Value {
        use std::io::Read;
        let path = self.trust.with_file_name("local-feature-policy.json");
        let cached = fs::File::open(path).ok().and_then(|file|
            serde_json::from_reader::<_, Value>(file.take(8192)).ok());
        cached.filter(|record| record["device"] == device)
            .and_then(|record| record["permissions"].as_object().map(local_permission_flags))
            .map(Value::Object).unwrap_or_else(|| json!({}))
    }

    pub(crate) async fn local_permissions(&self, identity: &Value) -> Value {
        let Some(device) = identity["deviceId"].as_str() else { return json!({}); };
        let path = self.trust.with_file_name("local-feature-policy.json");
        let session = {
            let _job = self.transfer.lock().await;
            self.origin().ok().zip(self.token.lock().expect("update session").clone())
        };
        if let Some((origin, token)) = session {
            let read = async {
                let client = reqwest::Client::builder().no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_millis(2500)).build().ok()?;
                let mut response = client.get(format!("{origin}/permissions")).bearer_auth(&token)
                    .send().await.ok()?.error_for_status().ok()?;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.ok()? {
                    if bytes.len() + chunk.len() > 64 * 1024 { return None; }
                    bytes.extend_from_slice(&chunk);
                }
                let value: Value = serde_json::from_slice(&bytes).ok()?;
                (value["ok"] == true).then(|| value["policy"]["permissions"].as_object().cloned()).flatten()
            };
            if let Some(permissions) = read.await {
                let _job = self.transfer.lock().await;
                if self.origin().ok().as_deref() == Some(origin.as_str())
                    && self.token.lock().expect("update session").as_deref() == Some(token.as_str()) {
                    let permissions = local_permission_flags(&permissions);
                    let record = json!({"device":device,"permissions":permissions});
                    let _ = crate::host::atomic_write(&path, |file| serde_json::to_writer(file, &record).map_err(io::Error::other));
                    return Value::Object(permissions);
                }
            }
        }
        self.cached_local_permissions(device)
    }

    pub fn new(trust: PathBuf, downloads: PathBuf, fixture: bool, version: String, rust_channel: bool) -> Self {
        Self {
            trust,
            downloads,
            version,
            origin: Mutex::default(),
            token: Mutex::default(),
            ready: Mutex::default(),
            transfer: tokio::sync::Mutex::new(()),
            fixture,
            rust_channel,
        }
    }

    fn policy_path(&self) -> PathBuf {
        self.trust.with_file_name(if self.rust_channel { "required-rust-client-update.json" } else { "required-client-update.json" })
    }

    fn endpoint(&self) -> &'static str {
        if self.rust_channel { "/rust-client-update" } else { "/client-update" }
    }

    fn verify(&self, manifest: &Value, key: &str) -> Result<()> {
        if self.rust_channel {
            crate::updates::verify_rust_manifest(manifest, key).map_err(reason)
        } else {
            crate::updates::verify_manifest(manifest, key).map_err(reason)
        }
    }

    fn policy(&self) -> Result<Value> {
        let raw = match fs::File::open(self.policy_path()) {
            Ok(file) => serde_json::from_reader::<_, Value>(file).map_err(reason)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Value::Null),
            Err(error) => return Err(reason(error)),
        };
        if raw.is_null() {
            return Ok(raw);
        }
        let trust = self.read_trust()?;
        let origin = raw["origin"]
            .as_str()
            .ok_or("Alamat kebijakan update tidak valid.")?;
        let key = trust
            .get(origin)
            .and_then(Value::as_str)
            .ok_or("Kunci update HOST belum dipercaya.")?;
        self.verify(&raw["manifest"], key)?;
        Ok(raw)
    }

    fn required(&self) -> Result<Value> {
        let policy = self.policy()?;
        let manifest = &policy["manifest"];
        if manifest["required"] != true
            || !newer(
                manifest["version"].as_str().unwrap_or_default(),
                &self.version,
            )
        {
            return Ok(json!({"ok":true,"locked":false}));
        }
        Ok(
            json!({"ok":true,"locked":true,"version":manifest["version"],"notes":policy["notes"],"changelog":policy["changelog"]}),
        )
    }

    pub fn set_discovered(&self, address: std::net::SocketAddrV4) {
        *self.origin.lock().expect("update origin") = Some(format!("http://{address}"));
        self.token.lock().expect("update session").take();
        self.ready.lock().expect("verified update").take();
    }

    pub(crate) fn origin(&self) -> Result<String> {
        self.origin
            .lock()
            .expect("update origin")
            .clone()
            .ok_or("HOST update belum terhubung.".into())
    }

    fn read_trust(&self) -> Result<serde_json::Map<String, Value>> {
        match fs::File::open(&self.trust) {
            Ok(file) => serde_json::from_reader(file).map_err(reason),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(serde_json::Map::new()),
            Err(error) => Err(reason(error)),
        }
    }

    fn remember(&self, key: &str, fingerprint: &str, token: &str) -> Result<Value> {
        if key.len() > 2048
            || format!("{:x}", Sha256::digest(key.as_bytes())) != fingerprint
            || VerifyingKey::from_public_key_pem(key).is_err()
            || token.len() != 64
            || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Identitas penandatangan HOST tidak valid.".into());
        }
        let origin = self.origin()?;
        let mut trust = self.read_trust()?;
        if let Some(previous) = trust.get(&origin) {
            if previous != key {
                return Err("Kunci update HOST berubah. Periksa kembali identitas HOST.".into());
            }
        } else {
            fs::create_dir_all(self.trust.parent().ok_or("Folder konfigurasi update tidak valid.")?).map_err(reason)?;
            trust.insert(origin, json!(key));
            crate::host::atomic_save(&self.trust, &trust).map_err(reason)?;
        }
        *self.token.lock().expect("update session") = Some(token.to_owned());
        Ok(json!({"ok":true}))
    }

    async fn fetch_verified(&self) -> Result<(reqwest::Client, String, String, Value)> {
        let origin = self.origin()?;
        let token = self
            .token
            .lock()
            .expect("update session")
            .clone()
            .ok_or("Sesi update HOST belum terhubung.")?;
        let trust = self.read_trust()?;
        let key = trust
            .get(&origin)
            .and_then(Value::as_str)
            .ok_or("Kunci update HOST belum dipercaya.")?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(180))
            .build()
            .map_err(reason)?;
        let mut response = client
            .get(format!("{origin}{}/status",self.endpoint()))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(reason)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!("Status update gagal ({})", response.status()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(reason)? {
            if bytes.len() + chunk.len() > MAX_STATUS {
                return Err("Status update terlalu besar.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let status: Value = serde_json::from_slice(&bytes).map_err(reason)?;
        if status["ok"] != true {
            return Err("Status update HOST tidak valid.".into());
        }
        if status["available"] != true {
            return Ok((client, origin, token, Value::Null));
        }
        let manifest = &status["manifest"];
        self.verify(manifest, key)?;
        if status["version"] != manifest["version"]
            || status["required"] != manifest["required"]
            || status["downloadUrl"] != format!("{}/download",self.endpoint())
        {
            return Err("Informasi update berbeda dengan manifest HOST.".into());
        }
        let previous = self.policy()?;
        if !previous.is_null() {
            let old = &previous["manifest"];
            if previous["origin"] != origin && self.required()?["locked"] == true {
                return Err("Update wajib harus diperiksa dari HOST yang menerbitkannya.".into());
            }
            if previous["origin"] == origin
                && (old["publishedAt"].as_u64() > manifest["publishedAt"].as_u64()
                    || (old["publishedAt"] == manifest["publishedAt"] && old != manifest))
            {
                return Err("HOST mengirim kebijakan update yang lebih lama.".into());
            }
        }
        if self.origin()? != origin {
            return Err("HOST berubah saat memeriksa update.".into());
        }
        if previous["manifest"] != *manifest || previous["origin"] != origin {
            let clip = |key: &str, limit: usize| {
                status[key]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(limit)
                    .collect::<String>()
            };
            let saved = json!({"origin":origin,"manifest":manifest,"notes":clip("notes",1000),"changelog":clip("changelog",16000)});
            crate::host::atomic_write(&self.policy_path(), |file| {
                serde_json::to_writer(file, &saved).map_err(io::Error::other)
            })
            .map_err(reason)?;
            self.ready.lock().expect("verified update").take();
        }
        Ok((client, origin, token, status))
    }

    async fn prepare(&self, window: &WebviewWindow, expected_manifest: Option<&Value>) -> Result<Value> {
        let _job = self
            .transfer
            .try_lock()
            .map_err(|_| "Unduhan update sedang berjalan.")?;
        self.ready.lock().expect("verified update").take();
        let (client, origin, token, status) = self.fetch_verified().await?;
        if status.is_null() {
            return Err("HOST belum menyediakan update.".into());
        }
        let manifest = &status["manifest"];
        if expected_manifest.is_some_and(|expected| expected != manifest) {
            return Err("Update di HOST berubah. Periksa versi terbaru sebelum melanjutkan.".into());
        }
        let size = manifest["size"].as_u64().expect("verified update size");
        let hash = manifest["sha256"]
            .as_str()
            .expect("verified update checksum");
        let version = manifest["version"]
            .as_str()
            .expect("verified update version");
        if !newer(version, &self.version) {
            return Err("Versi aplikasi ini sudah sama atau lebih baru.".into());
        }
        let mut response = client
            .get(format!("{origin}{}/download",self.endpoint()))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(reason)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!("Download update gagal ({})", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length != size)
        {
            return Err("Ukuran unduhan tidak cocok dengan manifest HOST.".into());
        }
        tokio::fs::create_dir_all(&self.downloads)
            .await
            .map_err(reason)?;
        let target = self.downloads.join(format!(
            "ChatX-CLIENT-SETUP-v{version}-{}.exe",
            uuid::Uuid::new_v4()
        ));
        let partial = Partial(target.with_extension("part"));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial.0)
            .await
            .map_err(reason)?;
        let mut received = 0u64;
        let mut previous = None;
        while let Some(chunk) = response.chunk().await.map_err(reason)? {
            received += chunk.len() as u64;
            if received > size || received > crate::updates::MAX_INSTALLER {
                return Err("Unduhan melebihi ukuran manifest HOST.".into());
            }
            file.write_all(&chunk).await.map_err(reason)?;
            let percent = (received * 100 / size).min(99);
            if previous != Some(percent) {
                previous = Some(percent);
                let _ = window.emit(
                    "client-update-progress",
                    json!({"received":received,"total":size,"pct":percent}),
                );
            }
        }
        if received != size {
            return Err("Unduhan update tidak lengkap.".into());
        }
        file.sync_all().await.map_err(reason)?;
        drop(file);
        let staged = partial.0.clone();
        let (actual, digest) =
            tokio::task::spawn_blocking(move || crate::updates::checked_installer(&staged))
                .await
                .map_err(reason)?
                .map_err(reason)?;
        if actual != size || digest != hash {
            return Err("Checksum installer tidak cocok dengan manifest HOST.".into());
        }
        if self.origin()? != origin {
            return Err("HOST berubah selama pengunduhan update.".into());
        }
        tokio::fs::rename(&partial.0, &target)
            .await
            .map_err(reason)?;
        *self.ready.lock().expect("verified update") =
            Some((target.clone(), size, hash.to_owned()));
        let _ = window.emit(
            "client-update-progress",
            json!({"received":size,"total":size,"pct":100}),
        );
        Ok(
            json!({"ok":true,"phase":"verified","path":target,"version":version,"required":manifest["required"]}),
        )
    }

    fn open(&self, owner: usize) -> Result<Value> {
        use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
        use windows_sys::Win32::{
            Storage::FileSystem::FILE_SHARE_READ,
            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        if self.fixture {
            return Err("Fixture pengujian tidak dijalankan.".into());
        }
        let ready = self.ready.lock().expect("verified update");
        let (path, size, hash) = ready
            .as_ref()
            .ok_or("Unduh dan verifikasi installer terlebih dahulu.")?;
        let _locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
            .map_err(reason)?;
        let checked = crate::updates::checked_installer(path).map_err(reason)?;
        if checked.0 != *size || checked.1 != *hash {
            return Err("File installer berubah setelah diverifikasi.".into());
        }
        let wide: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        let open = [b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];
        let result = unsafe {
            ShellExecuteW(
                owner as _,
                open.as_ptr(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!(
                "Installer terverifikasi tetapi gagal dibuka ({})",
                result as isize
            ));
        }
        Ok(json!({"ok":true,"phase":"opened","path":path}))
    }
}

fn newer(candidate: &str, installed: &str) -> bool {
    let parts = |version: &str| {
        version
            .split('.')
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()
    };
    matches!((parts(candidate),parts(installed)),(Some(candidate),Some(installed))
        if candidate.len()==3 && installed.len()==3 && candidate>installed)
}

struct Partial(PathBuf);
impl Drop for Partial {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn reason(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub async fn remember_update_host(
    updater: State<'_, Arc<ClientUpdater>>,
    key: String,
    fingerprint: String,
    token: String,
) -> Result<Value> {
    let updater = updater.inner().clone();
    let _job = updater.transfer.lock().await;
    updater.remember(&key, &fingerprint, &token)
}

#[tauri::command]
pub async fn remember_client_session(
    updater: State<'_, Arc<ClientUpdater>>,
    profile: State<'_, crate::client_profile::ProfileStore>,
    origin: String, key: String, fingerprint: String, token: String,
) -> Result<Value> {
    let address = crate::network::probe_primary(&origin).await.ok_or("HOST utama tidak dapat diverifikasi")?;
    let _job = updater.transfer.lock().await;
    updater.set_discovered(address);
    let result = updater.remember(&key, &fingerprint, &token)?;
    drop(_job);
    if let Ok(Some(identity)) = profile.load() {
        let updater = updater.inner().clone();
        tauri::async_runtime::spawn(async move { updater.local_permissions(&identity).await; });
    }
    Ok(result)
}

#[tauri::command]
pub async fn download_client_attachment(window: WebviewWindow, updater: State<'_, Arc<ClientUpdater>>,
    profile: State<'_, crate::client_profile::ProfileStore>, url: String, name: String) -> Result<Value> {
    if window.label() != "main" { return Err("Unduhan hanya tersedia di jendela CLIENT utama".into()); }
    updater.authorize_local(&profile, "downloadFiles", "unduh file").await?;
    let origin = updater.origin()?;
    let mut parsed = reqwest::Url::parse(&url).map_err(reason)?;
    if parsed.origin().ascii_serialization() != origin || !parsed.username().is_empty() || parsed.password().is_some()
        || parsed.query_pairs().any(|(key,_)| key != "kc_token") || parsed.fragment().is_some() {
        return Err("Lampiran harus berasal dari HOST yang terhubung".into());
    }
    // Renderer URLs carry an asset token. Native downloads use the current session header.
    parsed.set_query(None);
    let stored = parsed.path().strip_prefix("/download/").filter(|name| !name.is_empty()
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')))
        .ok_or("Alamat lampiran tidak valid")?;
    let expected = stored.get(..64).filter(|hash| hash.bytes().all(|byte| byte.is_ascii_hexdigit())).map(str::to_ascii_lowercase);
    let clean: String = name.chars().take(180).map(|ch| if ch.is_control() || "<>:\"/\\|?*".contains(ch) { '_' } else { ch }).collect();
    let clean = clean.trim_end_matches(['.', ' ']);
    let name = if clean.is_empty() { "lampiran" } else { clean }.to_owned();
    let owner = window.hwnd().map_err(reason)?.0 as usize;
    let destination = if crate::client_renderer_smoke() {
        std::env::var_os("KC_TEST_CLIENT_DOWNLOAD").map(PathBuf::from)
    } else {
        tokio::task::spawn_blocking(move || crate::image_files::choose_save_path(owner, &name,
            "Semua file\0*.*\0\0", "", "Simpan lampiran" )).await.map_err(reason)??
    };
    let Some(destination) = destination else { return Ok(json!({"ok":false,"canceled":true})); };
    let token = updater.token.lock().expect("update session").clone().ok_or("Sesi HOST belum terhubung")?;
    let temporary = destination.with_file_name(format!(".kc-download-{}.part", uuid::Uuid::new_v4()));
    let result = async {
        let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(60)).build().map_err(reason)?;
        let mut response = client.get(parsed).bearer_auth(token).send().await.map_err(reason)?;
        if response.status() != reqwest::StatusCode::OK { return Err(format!("Unduh lampiran HTTP {}", response.status().as_u16())); }
        let expected_length = response.content_length();
        let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&temporary).await.map_err(reason)?;
        let mut hash = Sha256::new();
        let mut total = 0u64;
        while let Some(bytes) = response.chunk().await.map_err(reason)? {
            total = total.checked_add(bytes.len() as u64).ok_or("Ukuran lampiran melebihi kapasitas")?;
            file.write_all(&bytes).await.map_err(reason)?;
            hash.update(&bytes);
        }
        file.sync_all().await.map_err(reason)?;
        drop(file);
        if expected_length.is_some_and(|length| length != total) { return Err("Unduhan lampiran belum lengkap".into()); }
        if expected.is_some_and(|expected| expected != format!("{:x}", hash.finalize())) { return Err("Checksum lampiran berbeda. Coba unduh ulang.".into()); }
        if updater.origin()? != origin { return Err("HOST berubah saat unduhan berjalan. Coba lagi.".into()); }
        tokio::fs::rename(&temporary, &destination).await.map_err(reason)?;
        Ok(json!({"ok":true,"path":destination,"size":total}))
    }.await;
    if result.is_err() { let _ = tokio::fs::remove_file(&temporary).await; }
    result
}

#[tauri::command]
pub async fn prepare_client_update(
    window: WebviewWindow,
    updater: State<'_, Arc<ClientUpdater>>,
    expected_manifest: Option<Value>,
) -> Result<Value> {
    updater.prepare(&window, expected_manifest.as_ref()).await
}

#[tauri::command]
pub fn client_required_update(updater: State<'_, Arc<ClientUpdater>>) -> Result<Value> {
    updater.required()
}

#[tauri::command]
pub async fn refresh_client_update(window: WebviewWindow, updater: State<'_, Arc<ClientUpdater>>) -> Result<Value> {
    let _job = updater
        .transfer
        .try_lock()
        .map_err(|_| "Unduhan update sedang berjalan.")?;
    let (_, _, _, status) = updater.fetch_verified().await?;
    let mut policy = updater.required()?;
    if updater.rust_channel {
        let _ = window.emit_to("game", "client-required-update", &policy);
        policy["status"] = status;
    }
    Ok(policy)
}

#[tauri::command]
pub async fn open_client_update(
    window: WebviewWindow,
    updater: State<'_, Arc<ClientUpdater>>,
) -> Result<Value> {
    let owner = window.hwnd().map_err(reason)?.0 as usize;
    let updater = updater.inner().clone();
    tokio::task::spawn_blocking(move || updater.open(owner))
        .await
        .map_err(reason)?
}
