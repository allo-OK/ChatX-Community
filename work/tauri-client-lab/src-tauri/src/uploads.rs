use axum::{
    body::Bytes, extract::{DefaultBodyLimit, FromRequest, Multipart, Path as RoutePath, Request, State},
    http::{HeaderMap, StatusCode}, routing::{get, post, put}, Json, Router,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions}, io::{self, Write}, path::{Path, PathBuf},
    sync::{Arc, Mutex}, time::{SystemTime, UNIX_EPOCH},
};

const CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
type Reply = (StatusCode, Json<Value>);
type Auth = dyn Fn(&HeaderMap) -> Option<(String, String)> + Send + Sync;

#[derive(Clone)]
struct Uploads {
    auth: Arc<Auth>,
    gate: Arc<Mutex<()>>,
}

pub fn router(auth: impl Fn(&HeaderMap) -> Option<(String, String)> + Send + Sync + 'static) -> Router {
    Router::new()
        .route("/upload", post(single).layer(DefaultBodyLimit::max(MAX_BYTES as usize + 64 * 1024)))
        .route("/upload/init", post(init))
        .route("/upload/status/{id}", get(status))
        .route("/upload/chunk/{id}", put(chunk).layer(DefaultBodyLimit::max(CHUNK_BYTES)))
        .route("/upload/finish", post(finish))
        .route("/upload/cancel", post(cancel))
        .with_state(Uploads { auth: Arc::new(auth), gate: Arc::new(Mutex::new(())) })
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

async fn single(State(state): State<Uploads>, request: Request) -> Reply {
    use tokio::io::AsyncWriteExt;
    let headers = request.headers().clone();
    let Some((device, key)) = (state.auth)(&headers) else {
        return error(StatusCode::UNAUTHORIZED, "Perangkat belum disetujui / sesi berakhir");
    };
    let result: Result<Reply, String> = async {
        let mut multipart = Multipart::from_request(request, &()).await.map_err(|_| "Unggahan file tidak valid.")?;
        let mut field = multipart.next_field().await.map_err(|_| "Unggahan file tidak valid.")?
            .filter(|field| field.name() == Some("file")).ok_or("Pilih file untuk dikirim.")?;
        let filename: String = field.file_name().unwrap_or("file").rsplit(['/', '\\']).next().unwrap_or("file")
            .chars().filter(|character| !character.is_control()).take(220).collect();
        let mimetype: String = field.content_type().unwrap_or("application/octet-stream").chars().take(180).collect();
        let id = uuid::Uuid::new_v4().to_string();
        let (metadata, part) = files(&id).expect("valid upload id");
        tokio::fs::create_dir_all(part.parent().expect("staging directory")).await.map_err(|error| error.to_string())?;
        let _cleanup = Staging(part.clone());
        let mut output = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&part).await.map_err(|error| error.to_string())?;
        let mut size = 0u64;
        while let Some(chunk) = field.chunk().await.map_err(|_| "Unggahan file terputus atau terlalu besar.")? {
            size += chunk.len() as u64;
            if size > MAX_BYTES { return Err("Maksimal ukuran file 2 GB.".into()); }
            output.write_all(&chunk).await.map_err(|error| error.to_string())?;
        }
        drop(field);
        if multipart.next_field().await.map_err(|_| "Unggahan file tidak valid.")?.is_some() {
            return Err("Kirim satu file per permintaan.".into());
        }
        output.sync_all().await.map_err(|error| error.to_string())?;
        drop(output);
        let metadata_cleanup = Staging(metadata.clone());
        let initialized = state.clone().run(headers.clone(), move |current_device, current_key| {
            if current_device != device || current_key != key {
                return Ok(error(StatusCode::UNAUTHORIZED, "Sesi upload berubah."));
            }
            let value = json!({"name":filename,"mimetype":mimetype,"size":size,"owner":device,"key":key});
            crate::host::atomic_save(&metadata, value.as_object().expect("upload metadata"))?;
            Ok(ok(json!({"ok":true})))
        }).await;
        if initialized.0 != StatusCode::OK { return Ok(initialized); }
        let result = finish(State(state), headers, Json(json!({"id":id}))).await;
        drop(metadata_cleanup);
        Ok(result)
    }.await;
    match result {
        Ok(reply) => reply,
        Err(message) => error(StatusCode::BAD_REQUEST, &message),
    }
}

fn ok(value: Value) -> Reply {
    (StatusCode::OK, Json(value))
}

fn error(status: StatusCode, message: &str) -> Reply {
    (status, Json(json!({"ok":false,"error":message})))
}

impl Uploads {
    async fn run(
        self, headers: HeaderMap,
        operation: impl FnOnce(String, String) -> io::Result<Reply> + Send + 'static,
    ) -> Reply {
        let Some((device, key)) = (self.auth)(&headers) else {
            return error(StatusCode::UNAUTHORIZED, "Perangkat belum disetujui / sesi berakhir");
        };
        match tokio::task::spawn_blocking(move || {
            let _guard = self.gate.lock().expect("upload storage");
            operation(device, key)
        }).await {
            Ok(Ok(reply)) => reply,
            _ => error(StatusCode::INTERNAL_SERVER_ERROR, "Penyimpanan upload gagal"),
        }
    }
}

fn files(id: &str) -> Option<(PathBuf, PathBuf)> {
    let id = uuid::Uuid::parse_str(id).ok()?.to_string();
    let root = Path::new("uploads").join("incoming");
    Some((root.join(format!("{id}.json")), root.join(format!("{id}.part"))))
}

fn owned(id: &str, device: &str, key: &str) -> io::Result<Option<(Value, PathBuf, PathBuf)>> {
    let Some((meta, part)) = files(id) else { return Ok(None) };
    let file = match File::open(&meta) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let value: Value = serde_json::from_reader(file)?;
    Ok((value["owner"].as_str() == Some(device) && value["key"].as_str() == Some(key))
        .then_some((value, meta, part)))
}

fn missing() -> Reply {
    error(StatusCode::NOT_FOUND, "Upload session tidak ditemukan")
}

pub async fn chat_attachment(stored: &str, device: &str, key: &str) -> io::Result<Option<Map<String, Value>>> {
    let Some(hash) = stored.get(..64) else { return Ok(None); };
    if !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let bytes = match tokio::fs::read("file-index.json").await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let index: Map<String, Value> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let Some(mut entry) = index.get(hash).and_then(Value::as_object).cloned() else {
        return Ok(None);
    };
    if entry.get("stored").and_then(Value::as_str) != Some(stored) || !owns_attachment(&entry, device, key) {
        return Ok(None);
    }
    entry.remove("ownerKeys");
    entry.remove("owners");
    Ok(Some(entry))
}

pub(crate) fn owns_attachment(entry: &Map<String, Value>, device: &str, key: &str) -> bool {
    if key.is_empty() { return false; }
    if let Some(bound) = entry.get("ownerKeys").and_then(|keys| keys.get(device)) {
        return bound.as_str() == Some(key);
    }
    entry.get("owners").and_then(Value::as_array)
        .is_some_and(|owners| owners.iter().any(|owner| owner.as_str() == Some(device)))
}

#[cfg(test)]
#[test]
fn legacy_attachment_owner_cannot_override_a_rust_key_binding() {
    let mut entry = json!({"owners":["legacy","bound"],"ownerKeys":{"bound":"original"}})
        .as_object().unwrap().clone();
    assert!(owns_attachment(&entry, "legacy", "verified"));
    assert!(!owns_attachment(&entry, "legacy", ""));
    assert!(!owns_attachment(&entry, "other", "verified"));
    assert!(!owns_attachment(&entry, "bound", "different"));
    assert!(owns_attachment(&entry, "bound", "original"));
    entry.remove("owners");
    assert!(!owns_attachment(&entry, "legacy", "verified"));
}

async fn init(State(state): State<Uploads>, headers: HeaderMap, Json(body): Json<Value>) -> Reply {
    let Some(size) = body["size"].as_u64() else {
        return error(StatusCode::BAD_REQUEST, "Ukuran file tidak valid");
    };
    if size > MAX_BYTES {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "Maksimal ukuran file 2 GB");
    }
    state.run(headers, move |device, key| {
        let id = uuid::Uuid::new_v4().to_string();
        let (meta, part) = files(&id).expect("valid UUID");
        fs::create_dir_all(part.parent().expect("staging path"))?;
        let name: String = body["name"].as_str().unwrap_or("file").rsplit(['/', '\\']).next()
            .unwrap_or("file").chars().filter(|character| !character.is_control()).take(220).collect();
        let name = if name.trim().is_empty() { "file" } else { &name };
        let mimetype: String = body["mimetype"].as_str().unwrap_or("application/octet-stream").chars().take(180).collect();
        File::create_new(&part)?.sync_all()?;
        let value = json!({"name":name,"mimetype":mimetype,"size":size,"owner":device,"key":key});
        if let Err(error) = crate::host::atomic_save(&meta, value.as_object().expect("upload metadata")) {
            let _ = fs::remove_file(part);
            return Err(error);
        }
        Ok(ok(json!({"ok":true,"id":id,"offset":0,"chunkBytes":CHUNK_BYTES,"maxFileBytes":MAX_BYTES})))
    }).await
}

async fn status(State(state): State<Uploads>, RoutePath(id): RoutePath<String>, headers: HeaderMap) -> Reply {
    state.run(headers, move |device, key| {
        let Some((meta, _, part)) = owned(&id, &device, &key)? else { return Ok(missing()) };
        Ok(ok(json!({"ok":true,"id":id,"offset":fs::metadata(part)?.len(),"size":meta["size"],"chunkBytes":CHUNK_BYTES})))
    }).await
}

async fn chunk(State(state): State<Uploads>, RoutePath(id): RoutePath<String>, headers: HeaderMap, bytes: Bytes) -> Reply {
    let offset = headers.get("x-upload-offset").and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u64>().ok());
    state.run(headers, move |device, key| {
        let Some((meta, _, part)) = owned(&id, &device, &key)? else { return Ok(missing()) };
        let mut file = OpenOptions::new().append(true).open(part)?;
        let actual = file.metadata()?.len();
        if offset != Some(actual) {
            return Ok((StatusCode::CONFLICT, Json(json!({"ok":false,"error":"Offset upload tidak cocok","expectedOffset":actual}))));
        }
        if bytes.is_empty() || actual + bytes.len() as u64 > meta["size"].as_u64().unwrap_or(0) {
            return Ok(error(StatusCode::BAD_REQUEST, "Ukuran chunk tidak valid"));
        }
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_data()) {
            file.set_len(actual)?;
            return Err(error);
        }
        Ok(ok(json!({"ok":true,"offset":actual+bytes.len() as u64,"size":meta["size"]})))
    }).await
}

async fn finish(State(state): State<Uploads>, headers: HeaderMap, Json(body): Json<Value>) -> Reply {
    state.run(headers, move |device, key| {
        let id = body["id"].as_str().unwrap_or_default();
        let Some((meta, metadata_path, part)) = owned(id, &device, &key)? else { return Ok(missing()) };
        let mut file = File::open(&part)?;
        let size = file.metadata()?.len();
        if Some(size) != meta["size"].as_u64() {
            return Ok((StatusCode::CONFLICT, Json(json!({"ok":false,"error":"Upload belum lengkap","offset":size,"size":meta["size"]}))));
        }
        let mut digest = Sha256::new();
        io::copy(&mut file, &mut digest)?;
        drop(file);
        let hash = format!("{:x}", digest.finalize());
        let index_path = Path::new("file-index.json");
        let mut index: Map<String, Value> = match File::open(index_path) {
            Ok(file) => serde_json::from_reader(file)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(error) => return Err(error),
        };
        let name = meta["name"].as_str().unwrap_or("file");
        let extension = Path::new(name).extension().and_then(|ext| ext.to_str())
            .filter(|ext| !ext.is_empty() && ext.len() <= 15 && ext.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        let indexed = index.contains_key(&hash);
        let entry = index.entry(hash.clone()).or_insert_with(|| {
            let stored = extension.map_or_else(|| hash.clone(), |ext| format!("{hash}.{ext}"));
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
            json!({
                "stored":stored,"name":name,"originalName":name,"size":size,"hash":hash,
                "mimetype":meta["mimetype"],"createdAt":now,
                "url":format!("/uploads/{stored}"),"downloadUrl":format!("/download/{stored}")
            })
        });
        let stored = entry["stored"].as_str().filter(|stored| stored.starts_with(&hash)
            && stored.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
            && !stored.contains("..") && !stored.ends_with('.'))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Nama file storage tidak valid"))?;
        let upload_dir = crate::host::storage_location::upload_dir(Path::new("."))?;
        fs::create_dir_all(&upload_dir)?;
        let target = upload_dir.join(stored);
        let duplicate = match fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_file() => indexed,
            Ok(_) => return Err(io::Error::other("Tujuan upload bukan file biasa")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if let Err(error) = fs::hard_link(&part, &target) {
                    if error.kind() == io::ErrorKind::AlreadyExists {return Err(error)}
                    crate::host::storage_location::copy_verified(&part, &target, &hash)?;
                }
                false
            }
            Err(error) => return Err(error),
        };
        if !entry["ownerKeys"].is_object() {
            entry["ownerKeys"] = json!({});
        }
        entry["ownerKeys"][&device] = json!(key);
        let mut result = entry.clone();
        for field in ["owners", "ownerKeys"] {
            result.as_object_mut().expect("file index entry").remove(field);
        }
        crate::host::atomic_save(index_path, &index)?;
        if duplicate && record_duplicate(id, size).is_err() {
            return Ok(error(StatusCode::INTERNAL_SERVER_ERROR, "Statistik penyimpanan tidak dapat diperbarui. Coba lagi setelah memeriksa penyimpanan HOST."));
        }
        fs::remove_file(metadata_path)?;
        let _ = fs::remove_file(part);
        Ok(ok(json!({"ok":true,"duplicate":duplicate,"file":result})))
    }).await
}

fn record_duplicate(id: &str, size: u64) -> io::Result<()> {
    // Upload finalization runs under the upload gate. Keep IDs only while
    // their staging metadata exists so a retried finish cannot count twice.
    let path = Path::new("storage-stats.json");
    let mut stats: Map<String, Value> = match File::open(path) {
        Ok(file) => serde_json::from_reader(file)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(error) => return Err(error),
    };
    let mut pending = stats.remove("rustCountedUploadIds")
        .and_then(|value| value.as_object().cloned()).unwrap_or_default();
    pending.retain(|id, _| files(id).is_some_and(|(metadata, _)| metadata.exists()));
    if pending.contains_key(id) {
        return Ok(());
    }
    pending.insert(id.to_owned(), json!(true));
    for (field, increment) in [("duplicatesAvoided", 1), ("bytesSaved", size)] {
        let count = stats.get(field).and_then(Value::as_u64).unwrap_or_default();
        stats.insert(field.into(), json!(count.saturating_add(increment)));
    }
    stats.insert("rustCountedUploadIds".into(), Value::Object(pending));
    crate::host::atomic_save(path, &stats)
}

async fn cancel(State(state): State<Uploads>, headers: HeaderMap, Json(body): Json<Value>) -> Reply {
    state.run(headers, move |device, key| {
        if let Some((_, meta, part)) = owned(body["id"].as_str().unwrap_or_default(), &device, &key)? {
            fs::remove_file(meta)?;
            let _ = fs::remove_file(part);
        }
        Ok(ok(json!({"ok":true})))
    }).await
}
