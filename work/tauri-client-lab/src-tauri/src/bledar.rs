use axum::{
    body::{to_bytes, Body}, extract::{DefaultBodyLimit, FromRequest, Multipart, Path as RoutePath, Request, State},
    http::{header, HeaderMap, StatusCode, Uri}, response::{IntoResponse, Response}, routing::{delete, get, post}, Json, Router,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io, path::PathBuf, sync::{Arc, Mutex}};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const MAX_BYTES: usize = 100 * 1024 * 1024;

const CATALOG: &str = "[]";
const VIDEOS: &[&[u8]] = &[];
const POSTERS: &[&[u8]] = &[];

type Auth = dyn Fn(&HeaderMap, &Uri) -> bool + Send + Sync;
type Admin = dyn Fn(&HeaderMap) -> bool + Send + Sync;

#[derive(Clone)]
pub struct Library {
    directory: PathBuf,
    builtins: Arc<Vec<Value>>,
    selection: Arc<Mutex<(usize, String)>>,
    auth: Arc<Auth>,
    admin: Arc<Admin>,
    mutations: Arc<tokio::sync::Mutex<()>>,
}

impl Library {
pub fn open(directory: PathBuf, auth: impl Fn(&HeaderMap, &Uri) -> bool + Send + Sync + 'static,
    admin: impl Fn(&HeaderMap) -> bool + Send + Sync + 'static) -> io::Result<Self> {
    Ok(Self {
        directory, builtins: Arc::new(serde_json::from_str(CATALOG)?),
        selection: Arc::new(Mutex::new((0, String::new()))), auth: Arc::new(auth),
        admin: Arc::new(admin), mutations: Arc::new(tokio::sync::Mutex::new(())),
    })
}

pub fn router(self) -> Router {
    Router::new()
        .route("/bledar-videos", get(catalog))
        .route("/bledar-videos/pick", get(pick))
        .route("/bledar-videos/media/{id}", get(media))
        .route("/bledar-videos/posters/{id}", get(poster))
        .route("/admin/bledar-videos", get(admin_catalog).post(admin_upload)
            .layer(DefaultBodyLimit::max(MAX_BYTES + 64 * 1024)))
        .route("/admin/bledar-videos/settings", post(admin_settings))
        .route("/admin/bledar-videos/{id}", delete(admin_remove))
        .with_state(self)
}

pub async fn pick_video(&self) -> Result<Value, String> {
    Ok(self.select(&self.snapshot_for(&self.config().await?)))
}
}

impl Library {
    async fn snapshot(&self) -> Value {
        self.snapshot_for(&self.config().await.unwrap_or_default())
    }

    async fn config(&self) -> Result<Map<String, Value>, String> {
        match tokio::fs::read(self.directory.join("settings.json")).await {
            Ok(bytes) => serde_json::from_slice::<Map<String, Value>>(&bytes).ok()
                .filter(|data| ["disabled", "custom"].iter().all(|key| data.get(*key).is_none_or(Value::is_array)))
                .ok_or_else(|| "Pengaturan video BLEDAR rusak.".to_owned()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Map::new()),
            Err(error) => Err(format!("Pengaturan video BLEDAR gagal dibaca: {error}")),
        }
    }

    async fn persist(&self, mut settings: Map<String, Value>) -> Result<Value, String> {
        settings.entry("custom").or_insert_with(|| json!([]));
        settings.entry("disabled").or_insert_with(|| json!([]));
        tokio::fs::create_dir_all(&self.directory).await.map_err(|error| error.to_string())?;
        let path = self.directory.join("settings.json");
        let snapshot = self.snapshot_for(&settings);
        tokio::task::spawn_blocking(move || crate::host::atomic_save(&path, &settings)).await
            .map_err(|_| "Penyimpanan BLEDAR berhenti.".to_owned())?.map_err(|error| error.to_string())?;
        Ok(snapshot)
    }

    fn snapshot_for(&self, config: &Map<String, Value>) -> Value {
        let mut entries = self.builtins.as_ref().clone();
        if let Some(custom) = config.get("custom").and_then(Value::as_array) {
            entries.extend(custom.iter().filter(|row| row["id"].as_str().is_some_and(valid_custom_id)).cloned());
        }
        let items = entries.into_iter().filter_map(|entry| {
            let id = entry["id"].as_str()?;
            let mut fields = entry.as_object()?.clone();
            let disabled = config.get("disabled").and_then(Value::as_array).is_some_and(|items| items.iter().any(|item| item == id));
            fields.remove("path");
            fields.insert("enabled".into(), json!(!disabled));
            fields.insert("url".into(), json!(format!("/bledar-videos/media/{id}")));
            fields.insert("posterUrl".into(), json!(builtin_index(id).map(|_| format!("/bledar-videos/posters/{id}"))));
            Some(fields)
        }).collect::<Vec<_>>();
        json!({"ok":true,"mode":config.get("mode").and_then(Value::as_str).unwrap_or("random"),
            "selectedId":config.get("selectedId").and_then(Value::as_str).unwrap_or("builtin-01"),
            "maxBytes":MAX_BYTES,"items":items})
    }

    fn select(&self, snapshot: &Value) -> Value {
        let mut state = self.selection.lock().expect("BLEDAR selection");
        let enabled = snapshot["items"].as_array().into_iter().flatten()
            .filter(|row| row["enabled"] == true).collect::<Vec<_>>();
        if enabled.is_empty() { return Value::Null; }
        let video = match snapshot["mode"].as_str() {
            Some("fixed") => enabled.iter().copied().find(|row| row["id"] == snapshot["selectedId"]).unwrap_or(enabled[0]),
            Some("sequence") => {
                let video = enabled[state.0 % enabled.len()];
                state.0 = state.0.wrapping_add(1);
                video
            }
            _ => {
                let candidates = enabled.iter().copied().filter(|row| enabled.len() == 1 || row["id"] != state.1).collect::<Vec<_>>();
                let candidates = if candidates.is_empty() { &enabled } else { &candidates };
                candidates[(uuid::Uuid::new_v4().as_u128() % candidates.len() as u128) as usize]
            }
        };
        state.1 = video["id"].as_str().unwrap_or_default().to_owned();
        json!({"id":video["id"],"name":video["name"],"url":video["url"]})
    }
}

fn valid_custom_id(id: &str) -> bool {
    id.strip_prefix("custom-").is_some_and(|id| !id.is_empty() && id.len() <= 40
        && id.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) || byte == b'-'))
}

fn builtin_index(id: &str) -> Option<usize> {
    let digits = id.strip_prefix("builtin-")?;
    if digits.len() != 2 || !digits.bytes().all(|digit| digit.is_ascii_digit()) { return None; }
    let index = digits.parse::<usize>().ok()?.checked_sub(1)?;
    (index < VIDEOS.len()).then_some(index)
}

async fn catalog(State(library): State<Library>, headers: HeaderMap, uri: Uri) -> Response {
    if !(library.auth)(&headers, &uri) { return StatusCode::UNAUTHORIZED.into_response(); }
    Json(library.snapshot().await).into_response()
}

async fn pick(State(library): State<Library>, headers: HeaderMap, uri: Uri) -> Response {
    if !(library.auth)(&headers, &uri) { return StatusCode::UNAUTHORIZED.into_response(); }
    admin_response(library.pick_video().await.map(|video| json!({"ok":true,"video":video})))
}

fn admin_response(result: Result<Value, String>) -> Response {
    match result {
        Ok(data) => Json(data).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error}))).into_response(),
    }
}

fn admin_denied() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"ok":false,"error":"Izin pengelolaan BLEDAR diperlukan."}))).into_response()
}

async fn admin_catalog(State(library): State<Library>, headers: HeaderMap) -> Response {
    if !(library.admin)(&headers) { return admin_denied(); }
    admin_response(library.config().await.map(|settings| library.snapshot_for(&settings)))
}

async fn admin_settings(State(library): State<Library>, request: Request) -> Response {
    if !(library.admin)(request.headers()) { return admin_denied(); }
    let result = async {
        let data = to_bytes(request.into_body(), 64 * 1024).await.map_err(|_| "Pengaturan video terlalu besar.")?;
        let input: Value = serde_json::from_slice(&data).map_err(|_| "Pengaturan video tidak valid.")?;
        let mode = input["mode"].as_str().filter(|mode| matches!(*mode, "random" | "sequence" | "fixed"))
            .ok_or("Pilih mode acak, bergantian, atau satu video.")?;
        let disabled = input["disabled"].as_array().ok_or("Daftar video nonaktif tidak valid.")?;
        let _guard = library.mutations.lock().await;
        let mut settings = library.config().await?;
        let snapshot = library.snapshot_for(&settings);
        let items = snapshot["items"].as_array().expect("BLEDAR items");
        let ids: HashSet<_> = items.iter().filter_map(|row| row["id"].as_str()).collect();
        let mut excluded = HashSet::new();
        let mut disabled_ids = Vec::new();
        for value in disabled {
            let id = value.as_str().filter(|id| ids.contains(id)).ok_or("Daftar video nonaktif tidak valid.")?;
            if excluded.insert(id) { disabled_ids.push(id); }
        }
        let first = items.iter().find_map(|row| row["id"].as_str().filter(|id| !excluded.contains(id)))
            .ok_or("Aktifkan minimal satu video.")?;
        let requested = input["selectedId"].as_str().filter(|id| ids.contains(id) && !excluded.contains(id));
        if mode == "fixed" && requested.is_none() { return Err("Video pilihan belum aktif atau sudah dihapus.".into()); }
        settings.insert("mode".into(), json!(mode));
        settings.insert("selectedId".into(), json!(requested.unwrap_or(first)));
        settings.insert("disabled".into(), json!(disabled_ids));
        let snapshot = library.persist(settings).await?;
        *library.selection.lock().expect("BLEDAR selection") = (0, String::new());
        Ok(snapshot)
    }.await;
    admin_response(result)
}

struct TempUpload(PathBuf);

impl Drop for TempUpload {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

async fn admin_upload(State(library): State<Library>, request: Request) -> Response {
    if !(library.admin)(request.headers()) { return admin_denied(); }
    let result = async {
        let mut multipart = Multipart::from_request(request, &()).await.map_err(|_| "Unggahan video tidak valid.")?;
        let mut field = multipart.next_field().await.map_err(|_| "Unggahan video tidak valid.")?
            .filter(|field| field.name() == Some("video")).ok_or("Pilih video MP4.")?;
        let filename = field.file_name().filter(|name| name.to_ascii_lowercase().ends_with(".mp4"))
            .ok_or("Pilih video MP4.")?.to_owned();
        let name: String = filename[..filename.len()-4].rsplit(['/', '\\']).next().unwrap_or_default()
            .chars().take(100).map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
        let id = format!("custom-{}", uuid::Uuid::new_v4());
        tokio::fs::create_dir_all(&library.directory).await.map_err(|error| error.to_string())?;
        let pending = TempUpload(library.directory.join(format!(".{id}.upload")));
        let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&pending.0).await
            .map_err(|error| error.to_string())?;
        let mut digest = Sha256::new();
        let mut size = 0;
        let mut prefix = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(|_| "Unggahan tidak lengkap atau video melebihi 100 MB.")? {
            size += chunk.len();
            if size > MAX_BYTES { return Err("Video maksimal 100 MB.".into()); }
            if prefix.len() < 12 { prefix.extend_from_slice(&chunk[..chunk.len().min(12-prefix.len())]); }
            if prefix.len() >= 8 && &prefix[4..8] != b"ftyp" { return Err("Isi file bukan MP4 yang valid.".into()); }
            digest.update(&chunk);
            file.write_all(&chunk).await.map_err(|error| error.to_string())?;
        }
        if size < 12 { return Err("Isi file bukan MP4 yang valid.".into()); }
        drop(field);
        if multipart.next_field().await.map_err(|_| "Unggahan video tidak valid.")?.is_some() {
            return Err("Unggah satu video MP4 per permintaan.".into());
        }
        file.sync_all().await.map_err(|error| error.to_string())?;
        drop(file);
        let hash = format!("{:x}", digest.finalize());
        let _guard = library.mutations.lock().await;
        let mut settings = library.config().await?;
        let mut snapshot = library.snapshot_for(&settings);
        if snapshot["items"].as_array().expect("BLEDAR items").iter().any(|row| row["sha256"] == hash) {
            snapshot["duplicate"] = json!(true);
            return Ok(snapshot);
        }
        let custom = settings.entry("custom").or_insert_with(|| json!([])).as_array_mut().expect("custom video list");
        if custom.len() >= 200 { return Err("Maksimal 200 video tambahan. Hapus video yang tidak dipakai.".into()); }
        custom.push(json!({"id":id,"name":name,"size":size,"sha256":hash,"builtin":false}));
        let destination = library.directory.join(format!("{id}.mp4"));
        tokio::fs::rename(&pending.0, &destination).await.map_err(|error| error.to_string())?;
        let result = library.persist(settings).await;
        if result.is_err() { let _ = tokio::fs::remove_file(destination).await; }
        result
    }.await;
    admin_response(result)
}

async fn admin_remove(State(library): State<Library>, RoutePath(id): RoutePath<String>, headers: HeaderMap) -> Response {
    if !(library.admin)(&headers) { return admin_denied(); }
    let result = async {
        if !valid_custom_id(&id) { return Err("Video bawaan tidak dihapus; nonaktifkan jika tidak digunakan.".into()); }
        let _guard = library.mutations.lock().await;
        let mut settings = library.config().await?;
        let custom = settings.get_mut("custom").and_then(Value::as_array_mut)
            .ok_or("Video tambahan tidak ditemukan.")?;
        let index = custom.iter().position(|row| row["id"] == id).ok_or("Video tambahan tidak ditemukan.")?;
        custom.remove(index);
        if let Some(disabled) = settings.get_mut("disabled").and_then(Value::as_array_mut) {
            disabled.retain(|item| item != &id);
        }
        if settings.get("selectedId").is_some_and(|selected| selected == &id) {
            settings.insert("mode".into(), json!("random"));
            settings.insert("selectedId".into(), json!("builtin-01"));
        }
        if !library.snapshot_for(&settings)["items"].as_array().expect("BLEDAR items").iter().any(|row| row["enabled"] == true) {
            settings.get_mut("disabled").and_then(Value::as_array_mut).expect("all videos disabled").retain(|item| item != "builtin-01");
        }
        let snapshot = library.persist(settings).await?;
        let _ = tokio::fs::remove_file(library.directory.join(format!("{id}.mp4"))).await;
        Ok(snapshot)
    }.await;
    admin_response(result)
}

async fn poster(State(library): State<Library>, RoutePath(id): RoutePath<String>, headers: HeaderMap, uri: Uri) -> Response {
    if !(library.auth)(&headers, &uri) { return StatusCode::UNAUTHORIZED.into_response(); }
    match builtin_index(&id) {
        Some(index) => ([(header::CONTENT_TYPE, "image/png")], Body::from(POSTERS[index])).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn media(State(library): State<Library>, RoutePath(id): RoutePath<String>, headers: HeaderMap, uri: Uri) -> Response {
    if !(library.auth)(&headers, &uri) { return StatusCode::UNAUTHORIZED.into_response(); }
    if let Some(index) = builtin_index(&id) {
        let data = VIDEOS[index];
        let size = data.len() as u64;
        return match media_range(&headers, size) {
            Ok(Some((start, end))) => video_response(size, Some((start, end)), Body::from(&data[start as usize..=end as usize])),
            Ok(None) => video_response(size, None, Body::from(data)),
            Err(error) => error,
        };
    }
    if !valid_custom_id(&id) || !library.snapshot().await["items"].as_array()
        .is_some_and(|rows| rows.iter().any(|row| row["id"] == id)) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = library.directory.join(format!("{id}.mp4"));
    let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else { return StatusCode::NOT_FOUND.into_response() };
    if !metadata.is_file() { return StatusCode::NOT_FOUND.into_response(); }
    let Ok(mut file) = tokio::fs::File::open(path).await else { return StatusCode::NOT_FOUND.into_response() };
    let size = metadata.len();
    let range = match media_range(&headers, size) { Ok(range) => range, Err(error) => return error };
    let length = if let Some((start, end)) = range {
        if file.seek(io::SeekFrom::Start(start)).await.is_err() { return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
        end - start + 1
    } else { size };
    video_response(size, range, Body::from_stream(tokio_util::io::ReaderStream::new(file.take(length))))
}

fn media_range(headers: &HeaderMap, size: u64) -> Result<Option<(u64, u64)>, Response> {
    match headers.get(header::RANGE) {
        None => Ok(None),
        Some(value) => value.to_str().ok().and_then(|value| crate::host::byte_range(value, size))
            .map(Some).ok_or_else(|| (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{size}"))]).into_response()),
    }
}

fn video_response(size: u64, range: Option<(u64, u64)>, body: Body) -> Response {
    let status = if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    let length = range.map_or(size, |(start, end)| end - start + 1);
    let mut response = (status, [
        (header::CONTENT_TYPE, "video/mp4".into()), (header::CONTENT_LENGTH, length.to_string()),
        (header::ACCEPT_RANGES, "bytes".into()), (header::CACHE_CONTROL, "private, max-age=3600".into()),
    ], body).into_response();
    if let Some((start, end)) = range {
        response.headers_mut().insert(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}").parse().expect("content range"));
    }
    response
}
