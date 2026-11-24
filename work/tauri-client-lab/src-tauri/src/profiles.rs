use axum::{body::to_bytes, extract::{DefaultBodyLimit, FromRequest, Multipart, Path as RoutePath, Request, State},
    http::{header, HeaderMap, StatusCode, Uri}, response::{IntoResponse, Response}, routing::get, Json, Router};
use serde_json::{json, Map, Value};
use std::{io, path::PathBuf, sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH}};

type Identity = dyn Fn(&HeaderMap, &Uri) -> Option<String> + Send + Sync;
type Changed = dyn Fn(Value) + Send + Sync;
const GROUP_AVATAR_LIMIT: usize = 10 * 1024 * 1024;

pub struct Profiles { directory: PathBuf, data: Mutex<Map<String, Value>> }

impl Profiles {
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.data.lock().expect("profiles").contains_key(&key(name))
    }

    pub fn open(directory: PathBuf) -> io::Result<Self> {
        let data = match std::fs::File::open(directory.join("profiles.json")) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { directory, data: Mutex::new(data) })
    }
    pub fn get(&self, name: &str) -> Value {
        self.data.lock().expect("profiles").get(&key(name)).cloned().unwrap_or_else(|| json!({"name":name,"avatarUrl":null}))
    }
    fn save(&self, name: &str, update: impl FnOnce(&mut Value)) -> Result<Value, String> {
        let mut data = self.data.lock().expect("profiles");
        let mut next = data.clone();
        let row = next.entry(key(name)).or_insert_with(|| json!({"name":name}));
        update(row);
        row["updatedAt"] = json!(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64);
        let result = row.clone();
        crate::host::atomic_save(&self.directory.join("profiles.json"), &next).map_err(|error| error.to_string())?;
        *data = next;
        Ok(result)
    }
    fn display_name(&self, owner: &str, display: &str) -> Result<Value, String> {
        let display = display.split_whitespace().collect::<Vec<_>>().join(" ");
        if display.is_empty() || display.encode_utf16().count() > 40 || display.chars().any(|ch| ch.is_control()) {
            return Err("Nama profil harus diisi, maksimal 40 karakter.".into());
        }
        let mut data = self.data.lock().expect("profiles");
        let owner_key = key(owner);
        let mut next = data.clone();
        let row = next.entry(owner_key).or_insert_with(|| json!({"name":owner}));
        row["displayName"] = json!(display);
        row["updatedAt"] = json!(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64);
        let row = row.clone();
        crate::host::atomic_save(&self.directory.join("profiles.json"), &next).map_err(|error| error.to_string())?;
        *data = next;
        Ok(row)
    }
}

pub(crate) fn key(name: &str) -> String {
    let mut output = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') { output.push(ch); }
        else if !output.ends_with('-') { output.push('-'); }
    }
    let output = output.trim_matches('-');
    if output.is_empty() { "editor".into() } else { output.into() }
}

#[derive(Clone)]
struct Service { profiles: Arc<Profiles>, identity: Arc<Identity>, changed: Arc<Changed> }

pub fn router(profiles: Arc<Profiles>, identity: impl Fn(&HeaderMap, &Uri) -> Option<String> + Send + Sync + 'static,
    changed: impl Fn(Value) + Send + Sync + 'static) -> Router {
    Router::new().route("/profile/{name}", get(profile))
        .route("/profile/{name}/avatar", axum::routing::post(avatar).layer(DefaultBodyLimit::max(8 * 1024 * 1024)))
        .route("/profile/{name}/name", axum::routing::post(display_name))
        .route("/group-avatar/upload", axum::routing::post(group_avatar)
            .layer(DefaultBodyLimit::max(GROUP_AVATAR_LIMIT + 64 * 1024)))
        .route("/avatars/{stored}", get(avatar_file))
        .with_state(Service { profiles, identity: Arc::new(identity), changed: Arc::new(changed) })
}

async fn display_name(State(service): State<Service>, RoutePath(name): RoutePath<String>, request: Request) -> Response {
    let Some(owner) = (service.identity)(request.headers(), request.uri()) else { return denied() };
    if !owner.trim().eq_ignore_ascii_case(name.trim()) { return denied() }
    let result: Result<Value, String> = async {
        let bytes = to_bytes(request.into_body(), 4096).await.map_err(|_| "Nama profil terlalu besar.")?;
        let input: Value = serde_json::from_slice(&bytes).map_err(|_| "Nama profil tidak valid.")?;
        let profiles = service.profiles.clone();
        let row = tokio::task::spawn_blocking(move || profiles.display_name(&owner, input["displayName"].as_str().unwrap_or_default()))
            .await.map_err(|_| "Penyimpanan nama profil berhenti.")??;
        (service.changed)(row.clone());
        Ok(json!({"ok":true,"profile":row}))
    }.await;
    match result {
        Ok(row) => Json(row).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error}))).into_response(),
    }
}

fn denied() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"ok":false,"error":"Sesi profil tidak valid."}))).into_response()
}

async fn profile(State(service): State<Service>, RoutePath(name): RoutePath<String>, headers: HeaderMap, uri: Uri) -> Response {
    if (service.identity)(&headers, &uri).is_none() { return denied(); }
    Json(json!({"ok":true,"profile":service.profiles.get(&name)})).into_response()
}

async fn avatar(State(service): State<Service>, RoutePath(name): RoutePath<String>, request: Request) -> Response {
    let Some(owner) = (service.identity)(request.headers(), request.uri()) else { return denied() };
    if owner.trim().to_lowercase() != name.trim().to_lowercase() { return denied() }
    let result: Result<Value, String> = async {
        let mut multipart = Multipart::from_request(request, &()).await.map_err(|_| "Unggahan foto tidak valid.")?;
        let field = multipart.next_field().await.map_err(|_| "Unggahan foto tidak valid.")?
            .filter(|field| field.name() == Some("avatar")).ok_or("Pilih foto profil.")?;
        let bytes = field.bytes().await.map_err(|_| "Foto terlalu besar atau terputus.")?;
        if bytes.len() > 8 * 1024 * 1024 { return Err("Foto profil terlalu besar.".into()); }
        let decoder = png::Decoder::new(io::Cursor::new(&bytes)).read_info().map_err(|_| "Hasil crop foto harus berupa PNG.")?;
        if decoder.info().width > 4096 || decoder.info().height > 4096 { return Err("Ukuran foto profil terlalu besar.".into()); }
        drop(decoder);
        if multipart.next_field().await.map_err(|_| "Unggahan foto tidak valid.")?.is_some() { return Err("Pilih satu foto profil.".into()); }
        let stored = format!("profile-{}.png", uuid::Uuid::new_v4());
        let profiles = service.profiles.clone();
        let (row, old) = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let folder = profiles.directory.join("avatars");
            std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
            let path = folder.join(&stored);
            use std::io::Write;
            crate::host::atomic_write(&path, |file| file.write_all(&bytes)).map_err(|error| error.to_string())?;
            let old = profiles.get(&name)["avatarStored"].as_str().map(str::to_owned);
            match profiles.save(&name, |row| {
                row["avatarStored"] = json!(stored);
                row["avatarUrl"] = json!(format!("/avatars/{stored}"));
            }) {
                Ok(row) => Ok((row, old)),
                Err(error) => { let _ = std::fs::remove_file(path); Err(error) }
            }
        }).await.map_err(|_| "Penyimpanan profil berhenti.")??;
        if let Some(old) = old.filter(|name| valid_stored(name)) {
            let _ = tokio::fs::remove_file(service.profiles.directory.join("avatars").join(old)).await;
        }
        (service.changed)(row.clone());
        Ok(json!({"ok":true,"profile":row}))
    }.await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error}))).into_response(),
    }
}

fn valid_stored(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && name.len() <= 200
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
}

async fn group_avatar(State(service): State<Service>, request: Request) -> Response {
    if (service.identity)(request.headers(), request.uri()).is_none() {
        // Drain a bounded upload so HTTP clients can receive the authentication error.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2),
            to_bytes(request.into_body(), GROUP_AVATAR_LIMIT + 64 * 1024)).await;
        return denied();
    }
    let upload = async {
        let mut multipart = Multipart::from_request(request, &()).await.map_err(|_| "Unggahan foto grup tidak valid.")?;
        let field = multipart.next_field().await.map_err(|_| "Unggahan foto grup tidak valid.")?
            .filter(|field| field.name() == Some("avatar")).ok_or("Pilih gambar grup.")?;
        let bytes = field.bytes().await.map_err(|_| "Foto grup terlalu besar atau terputus.")?;
        if multipart.next_field().await.map_err(|_| "Unggahan foto grup tidak valid.")?.is_some() {
            return Err("Pilih satu gambar grup.");
        }
        Ok(bytes)
    }.await;
    let bytes = match upload {
        Ok(bytes) if bytes.len() <= GROUP_AVATAR_LIMIT => bytes,
        Ok(_) => return (StatusCode::PAYLOAD_TOO_LARGE, Json(json!({"ok":false,"error":"Foto grup maksimal 10 MB."}))).into_response(),
        Err(error) => return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error}))).into_response(),
    };
    let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"Pilih foto PNG, JPEG, GIF, atau WebP."}))).into_response();
    };
    let stored = format!("group-{}.{extension}", uuid::Uuid::new_v4());
    let folder = service.profiles.directory.join("avatars");
    let path = folder.join(&stored);
    let saved = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        std::fs::create_dir_all(folder)?;
        crate::host::atomic_write(&path, |file| file.write_all(&bytes))
    }).await;
    if matches!(saved, Ok(Ok(()))) {
        Json(json!({"ok":true,"avatarUrl":format!("/avatars/{stored}")})).into_response()
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":"Foto grup belum tersimpan."}))).into_response()
    }
}

async fn avatar_file(State(service): State<Service>, RoutePath(stored): RoutePath<String>, headers: HeaderMap, uri: Uri) -> Response {
    if (service.identity)(&headers, &uri).is_none() { return denied() }
    if !valid_stored(&stored) { return StatusCode::BAD_REQUEST.into_response() }
    let kind = match stored.rsplit('.').next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "webp" => "image/webp", "gif" => "image/gif",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let path = service.profiles.directory.join("avatars").join(&stored);
    let limit = if stored.starts_with("group-") { GROUP_AVATAR_LIMIT } else { 8 * 1024 * 1024 };
    if !tokio::fs::symlink_metadata(&path).await.is_ok_and(|metadata| metadata.is_file() && metadata.len() <= limit as u64) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "private, max-age=3600")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
