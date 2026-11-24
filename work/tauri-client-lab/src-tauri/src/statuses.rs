use serde_json::{json, Value};
use std::{io, path::PathBuf, sync::Mutex};
use axum::{body::Body, extract::{DefaultBodyLimit, FromRequest, Multipart, Path as RoutePath, Request, State},
    http::{header, HeaderMap, StatusCode, Uri}, response::{IntoResponse, Response}, routing::{get,post}, Json, Router};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const MAX_MEDIA: u64 = 250 * 1024 * 1024;

pub(super) fn router(access: super::Attachments) -> Router {
    Router::new()
        .route("/status/upload", post(upload).layer(DefaultBodyLimit::max(MAX_MEDIA as usize + 64 * 1024)))
        .route("/status-media/{stored}", get(media_file))
        .with_state(access)
}

struct PartialMedia(PathBuf);
impl Drop for PartialMedia {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

async fn upload(State(access): State<super::Attachments>, request: Request) -> Response {
    if let Err(status) = access.search_identity(request.headers(), request.uri(), "general") {
        return (status,Json(json!({"ok":false,"error":"Sesi Status tidak valid"}))).into_response();
    }
    let result: Result<Value,String> = async {
        let mut multipart = Multipart::from_request(request,&()).await.map_err(|_| "Unggahan Status tidak valid")?;
        let mut field = multipart.next_field().await.map_err(|_| "Unggahan Status terputus")?
            .filter(|field|field.name()==Some("media")).ok_or("Pilih gambar, video, atau audio")?;
        let mime = field.content_type().unwrap_or_default().to_lowercase();
        if mime.len()>120 || !["image/","video/","audio/"].iter().any(|prefix|mime.starts_with(prefix))
            || mime.bytes().any(|byte|byte<32 || byte>=127) {
            return Err("Pilih gambar, video, atau audio".into());
        }
        let name = field.file_name().unwrap_or("Media Status").chars().take(180).collect::<String>();
        let extension = std::path::Path::new(&name).extension().and_then(|ext|ext.to_str())
            .filter(|ext|ext.len()<=15 && ext.bytes().all(|byte|byte.is_ascii_alphanumeric())).unwrap_or("bin").to_lowercase();
        tokio::fs::create_dir_all("status-media").await.map_err(|error|error.to_string())?;
        let stored = format!("{}.{extension}",uuid::Uuid::new_v4());
        let target = PathBuf::from("status-media").join(&stored);
        let partial = PartialMedia(target.with_extension("part"));
        let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&partial.0).await.map_err(|error|error.to_string())?;
        let mut size = 0_u64;
        while let Some(chunk) = field.chunk().await.map_err(|_| "Unggahan Status terlalu besar atau terputus")? {
            size += chunk.len() as u64;
            if size>MAX_MEDIA {return Err("Media Status maksimal 250 MB".into());}
            file.write_all(&chunk).await.map_err(|error|error.to_string())?;
        }
        drop(field);
        if size==0 {return Err("Media Status kosong".into());}
        if multipart.next_field().await.map_err(|_| "Unggahan Status terputus")?.is_some() {
            return Err("Pilih satu media Status".into());
        }
        file.sync_all().await.map_err(|error|error.to_string())?;
        drop(file);
        tokio::fs::rename(&partial.0,&target).await.map_err(|error|error.to_string())?;
        Ok(json!({"ok":true,"media":{"stored":stored,"url":format!("/status-media/{stored}"),"name":name,"size":size,"mimetype":mime}}))
    }.await;
    match result {
        Ok(value)=>Json(value).into_response(),
        Err(error)=>(StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":error}))).into_response(),
    }
}

async fn media_file(State(access): State<super::Attachments>, RoutePath(stored): RoutePath<String>, headers: HeaderMap, uri: Uri) -> Response {
    if let Err(status) = access.search_identity(&headers,&uri,"general") {return status.into_response();}
    if !valid_media_name(&stored) {return StatusCode::BAD_REQUEST.into_response();}
    let path = PathBuf::from("status-media").join(&stored);
    let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else {return StatusCode::NOT_FOUND.into_response();};
    if !metadata.is_file() || metadata.len()>MAX_MEDIA {return StatusCode::NOT_FOUND.into_response();}
    let Ok(mut file) = tokio::fs::File::open(path).await else {return StatusCode::NOT_FOUND.into_response();};
    let size = metadata.len();
    let range = match headers.get(header::RANGE) {
        None=>None,
        Some(value)=>match value.to_str().ok().and_then(|value|super::byte_range(value,size)) {
            Some(range)=>Some(range),
            None=>return (StatusCode::RANGE_NOT_SATISFIABLE,[(header::CONTENT_RANGE,format!("bytes */{size}"))]).into_response(),
        }
    };
    let length = if let Some((start,end))=range {
        if file.seek(io::SeekFrom::Start(start)).await.is_err() {return StatusCode::INTERNAL_SERVER_ERROR.into_response();}
        end-start+1
    } else {size};
    let mime = match stored.rsplit('.').next().unwrap_or_default() {
        "png"=>"image/png","jpg"|"jpeg"=>"image/jpeg","gif"=>"image/gif","webp"=>"image/webp",
        "mp4"|"m4v"=>"video/mp4","webm"=>"video/webm","mov"=>"video/quicktime",
        "mp3"=>"audio/mpeg","wav"=>"audio/wav","ogg"|"oga"=>"audio/ogg","m4a"=>"audio/mp4",
        _=>"application/octet-stream",
    };
    let mut response = (
        if range.is_some(){StatusCode::PARTIAL_CONTENT}else{StatusCode::OK},
        [(header::CONTENT_TYPE,mime.to_owned()),(header::CONTENT_LENGTH,length.to_string()),
         (header::ACCEPT_RANGES,"bytes".into()),(header::CACHE_CONTROL,"private, max-age=3600".into())],
        Body::from_stream(tokio_util::io::ReaderStream::new(file.take(length))),
    ).into_response();
    if let Some((start,end))=range {
        response.headers_mut().insert(header::CONTENT_RANGE,format!("bytes {start}-{end}/{size}").parse().expect("media range"));
    }
    response
}

fn valid_media_name(name: &str) -> bool {
    !name.is_empty() && name.len()<=200 && !name.contains("..")
        && name.bytes().all(|byte|byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

pub struct Statuses { path: PathBuf, rows: Mutex<Vec<Value>>, last_cleanup: Mutex<u64> }

impl Statuses {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let rows = match std::fs::File::open(&path) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        let store = Self {path, rows:Mutex::new(rows), last_cleanup:Mutex::new(0)};
        let directory = store.path.with_file_name("status-media");
        match std::fs::read_dir(&directory) {
            Ok(entries)=>for entry in entries {
                let entry=entry?;
                if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|ext|ext=="part") {
                    if let Err(error)=std::fs::remove_file(entry.path()) {eprintln!("STATUS_PARTIAL_CLEANUP_FAILED: {error}");}
                }
            },
            Err(error) if error.kind()==io::ErrorKind::NotFound=>{},
            Err(error)=>eprintln!("STATUS_PARTIAL_CLEANUP_FAILED: {error}"),
        }
        Ok(store)
    }

    fn save(&self, rows: &[Value]) -> Result<(), String> {
        crate::host::atomic_write(&self.path, |file| serde_json::to_writer(file, rows).map_err(io::Error::other))
            .map_err(|error| format!("Status belum tersimpan. {error}"))
    }

    fn commit(&self, rows: &mut Vec<Value>, next: Vec<Value>) -> Result<(),String> {
        self.save(&next)?;
        let old = std::mem::replace(rows,next);
        for stored in old.iter().filter_map(|row|row["media"]["stored"].as_str()).filter(|name|valid_media_name(name)) {
            if rows.iter().any(|row|row["media"]["stored"]==stored) {continue;}
            if let Err(error)=std::fs::remove_file(self.path.with_file_name("status-media").join(stored)) {
                if error.kind()!=io::ErrorKind::NotFound {eprintln!("STATUS_MEDIA_CLEANUP_FAILED {stored}: {error}");}
            }
        }
        Ok(())
    }

    fn prune(&self, rows: &mut Vec<Value>, now: u64) -> Result<(), String> {
        let keep: Vec<_> = rows.iter().filter(|row| row["expiresAt"].as_u64().unwrap_or_default() > now).cloned().collect();
        if keep.len() != rows.len() { self.commit(rows,keep)?; }
        let mut last = self.last_cleanup.lock().expect("status cleanup");
        if now.saturating_sub(*last) >= 60_000 {
            *last=now;
            if let Err(error)=self.sweep_unused(rows,now) {eprintln!("STATUS_MEDIA_CLEANUP_FAILED: {error}");}
        }
        Ok(())
    }

    fn sweep_unused(&self, rows: &[Value], now: u64) -> io::Result<()> {
        let directory=self.path.with_file_name("status-media");
        let entries=match std::fs::read_dir(&directory) {
            Ok(entries)=>entries,
            Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(()),
            Err(error)=>return Err(error),
        };
        for entry in entries {
            let entry=entry?;
            let Some(name)=entry.file_name().to_str().map(str::to_owned) else {continue;};
            if !entry.file_type()?.is_file() || !valid_media_name(&name) || name.ends_with(".part")
                || rows.iter().any(|row|row["media"]["stored"]==name) {continue;}
            let modified=entry.metadata()?.modified()?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
            if now.saturating_sub(modified)>24*60*60*1000 {
                if let Err(error)=std::fs::remove_file(entry.path()) {eprintln!("STATUS_MEDIA_CLEANUP_FAILED {name}: {error}");}
            }
        }
        Ok(())
    }

    pub fn list(&self, now: u64, profiles: &crate::profiles::Profiles) -> Result<Value, String> {
        let mut rows = self.rows.lock().expect("statuses");
        self.prune(&mut rows, now)?;
        let mut items = rows.clone();
        items.sort_by_key(|row| std::cmp::Reverse(row["createdAt"].as_u64().unwrap_or_default()));
        for item in &mut items {item["avatarUrl"] = profiles.get(item["user"].as_str().unwrap_or_default())["avatarUrl"].clone();}
        Ok(Value::Array(items))
    }

    pub fn create(&self, actor: &str, device: &str, input: &Value, now: u64) -> Result<Value, String> {
        let mut rows = self.rows.lock().expect("statuses");
        let text: String = input["text"].as_str().unwrap_or_default().trim().chars().take(1500).collect();
        let media = input["media"].as_object().map(|media| Value::Object(media.clone())).unwrap_or(Value::Null);
        if text.is_empty() && media.is_null() {return Err("Status kosong".into());}
        let media = if media.is_null() {media} else {
            let stored = media["stored"].as_str().filter(|name| !name.is_empty() && name.len() <= 200
                && !name.contains("..") && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)))
                .ok_or("Media status tidak valid")?;
            if !std::fs::symlink_metadata(self.path.with_file_name("status-media").join(stored)).is_ok_and(|metadata| metadata.is_file()) {
                return Err("Media status belum tersedia".into());
            }
            json!({"stored":stored,"url":format!("/status-media/{stored}"),
                "name":media["name"].as_str().unwrap_or_default().chars().take(180).collect::<String>(),
                "size":media["size"].as_u64().unwrap_or_default(),
                "mimetype":media["mimetype"].as_str().unwrap_or_default().chars().take(120).collect::<String>()})
        };
        let mime = media["mimetype"].as_str().unwrap_or_default();
        let kind = if mime.starts_with("image/") {"image"} else if mime.starts_with("video/") {"video"}
            else if mime.starts_with("audio/") {"audio"} else {"text"};
        let item = json!({"id":uuid::Uuid::new_v4().to_string(),"user":actor,"deviceId":device,
            "type":kind,"text":text,"media":media,"createdAt":now,"expiresAt":now+24*60*60*1000});
        let mut next: Vec<_> = rows.iter().filter(|row|row["expiresAt"].as_u64().unwrap_or_default()>now).cloned().collect();
        next.push(item.clone());
        if next.len()>3000 {next.drain(..next.len()-3000);}
        self.commit(&mut rows,next)?;
        Ok(item)
    }

    pub fn delete(&self, actor: &str, host: bool, id: &str) -> Result<(), String> {
        let mut rows = self.rows.lock().expect("statuses");
        let item = rows.iter().find(|row| row["id"] == id).ok_or("Status tidak ditemukan")?;
        if !host && crate::profiles::key(item["user"].as_str().unwrap_or_default()) != crate::profiles::key(actor) {
            return Err("Tidak boleh menghapus status orang lain".into());
        }
        let next: Vec<_> = rows.iter().filter(|row| row["id"] != id).cloned().collect();
        self.commit(&mut rows,next)?;
        Ok(())
    }

    pub fn reply(&self, id: &str, conversation: &Value, actor: &str, now: u64) -> Result<Value, String> {
        if id.is_empty() || conversation["type"] != "dm" {return Ok(Value::Null);}
        let mut rows = self.rows.lock().expect("statuses");
        self.prune(&mut rows, now)?;
        let Some(item) = rows.iter().find(|row| row["id"] == id) else {return Ok(Value::Null);};
        let owner = crate::profiles::key(item["user"].as_str().unwrap_or_default());
        if owner == crate::profiles::key(actor) || !conversation["members"].as_array().into_iter().flatten()
            .filter_map(Value::as_str).any(|name| crate::profiles::key(name) == owner) {return Ok(Value::Null);}
        Ok(json!({"id":item["id"],"user":item["user"],"type":item["type"],
            "text":item["text"].as_str().unwrap_or_default().chars().take(300).collect::<String>(),
            "mediaName":item["media"]["name"].as_str().unwrap_or_default(),
            "mediaUrl":item["media"]["url"].as_str().unwrap_or_default(),
            "mediaMimetype":item["media"]["mimetype"].as_str().unwrap_or_default(),
            "createdAt":item["createdAt"]}))
    }
}
