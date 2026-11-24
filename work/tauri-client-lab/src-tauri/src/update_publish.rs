use super::{installer, Channel};
use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{
    pkcs8::{
        spki::der::pem::LineEnding, DecodePrivateKey, DecodePublicKey, EncodePrivateKey,
        EncodePublicKey,
    },
    Signature, Signer, SigningKey, VerifyingKey,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use socketioxide::SocketIo;
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;

pub(crate) const MAX_INSTALLER: u64 = 2 * 1024 * 1024 * 1024;
const MIN_INSTALLER: u64 = 100_000;
const BACKUP_LIFETIME: Duration = Duration::from_secs(30 * 60);
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Default)]
pub struct UpdatePermit(Mutex<Option<Instant>>);

impl UpdatePermit {
    pub fn grant(&self) {
        *self.0.lock().expect("update backup permit") = Some(Instant::now());
    }
    fn ready(&self) -> bool {
        self.0
            .lock()
            .expect("update backup permit")
            .is_some_and(|stamp| stamp.elapsed() <= BACKUP_LIFETIME)
    }
    fn consume(&self) -> bool {
        self.0
            .lock()
            .expect("update backup permit")
            .take()
            .is_some_and(|stamp| stamp.elapsed() <= BACKUP_LIFETIME)
    }
}

#[derive(Default)]
pub struct Identity(Mutex<Option<(SigningKey, String)>>);

impl Identity {
    fn with_key<T>(&self, callback: impl FnOnce(&SigningKey, &str) -> Result<T>) -> Result<T> {
        let mut stored = self.0.lock().expect("update signing identity");
        if stored.is_none() {
            let path = Path::new("update-signing-key.pem");
            let key = match fs::read_to_string(path) {
                Ok(pem) => SigningKey::from_pkcs8_pem(&pem)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    use windows_sys::Win32::Security::Cryptography::{
                        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                    };
                    let mut seed = [0; 32];
                    let status = unsafe {
                        BCryptGenRandom(
                            std::ptr::null_mut(),
                            seed.as_mut_ptr(),
                            32,
                            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                        )
                    };
                    if status != 0 {
                        return Err("Windows tidak dapat membuat kunci update.".into());
                    }
                    let key = SigningKey::from_bytes(&seed);
                    let pem = key.to_pkcs8_pem(LineEnding::LF)?;
                    crate::host::atomic_write(path, |file| file.write_all(pem.as_bytes()))?;
                    key
                }
                Err(error) => return Err(error.into()),
            };
            let public = key.verifying_key().to_public_key_pem(LineEnding::LF)?;
            *stored = Some((key, public));
        }
        let (key, public) = stored.as_ref().expect("loaded update identity");
        callback(key, public)
    }

    pub fn public(&self) -> Result<(String, String)> {
        self.with_key(|_, public| {
            Ok((
                public.to_owned(),
                format!("{:x}", Sha256::digest(public.as_bytes())),
            ))
        })
    }

    fn sign(
        &self,
        version: &str,
        size: u64,
        hash: &str,
        published: u64,
        required: bool,
        channel: Channel,
    ) -> Result<Value> {
        self.with_key(|key,public| {
            let canonical=canonical(version,size,hash,published);
            let policy=policy(&canonical,required);
            let mut manifest = json!({"version":version,"size":size,"sha256":hash,"publishedAt":published,"minProtocol":2,"required":required,
                "signature":STANDARD.encode(key.sign(canonical.as_bytes()).to_bytes()),
                "policySignature":STANDARD.encode(key.sign(policy.as_bytes()).to_bytes()),"publicKey":public});
            if channel == Channel::Rust {
                manifest["channel"] = json!("rust");
                manifest["channelSignature"] = json!(STANDARD.encode(key.sign(rust_policy(&policy).as_bytes()).to_bytes()));
            }
            Ok(manifest)
        })
    }

    fn verify_archive(&self, meta: &Value, size: u64, hash: &str, channel: Channel) -> Result<()> {
        let m = &meta["manifest"];
        let version = meta["version"]
            .as_str()
            .filter(|v| valid_version(v))
            .ok_or("Versi arsip tidak valid")?;
        if m["version"] != version
            || m["size"].as_u64() != Some(size)
            || m["sha256"] != hash
            || m["minProtocol"] != 2
        {
            return Err("Checksum atau manifest arsip update berbeda".into());
        }
        self.with_key(|_, public| {
            verify_manifest(m, public)?;
            if channel == Channel::Rust { verify_rust_manifest(m, public)?; }
            let required = m["required"] == true;
            if meta["required"]
                .as_bool()
                .is_some_and(|value| value != required)
            {
                return Err("Kebijakan arsip berbeda dengan manifest".into());
            }
            Ok(())
        })
    }
}

pub(crate) fn verify_manifest(manifest: &Value, trusted: &str) -> Result<()> {
    let invalid = "Tanda tangan / manifest update tidak valid";
    if manifest["publicKey"] != trusted || manifest["minProtocol"] != 2 {
        return Err(invalid.into());
    }
    let version = manifest["version"]
        .as_str()
        .filter(|version| valid_version(version))
        .ok_or(invalid)?;
    let size = manifest["size"]
        .as_u64()
        .filter(|size| (MIN_INSTALLER..=MAX_INSTALLER).contains(size))
        .ok_or(invalid)?;
    let hash = manifest["sha256"]
        .as_str()
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or(invalid)?;
    let published = manifest["publishedAt"]
        .as_u64()
        .filter(|stamp| *stamp <= 9_007_199_254_740_991)
        .ok_or(invalid)?;
    let signed = canonical(version, size, hash, published);
    let signature = |field: &str| -> Result<Signature> {
        let encoded = manifest[field]
            .as_str()
            .filter(|value| value.len() == 88)
            .ok_or(invalid)?;
        Ok(Signature::from_slice(&STANDARD.decode(encoded)?)?)
    };
    let key = VerifyingKey::from_public_key_pem(trusted)?;
    key.verify_strict(signed.as_bytes(), &signature("signature")?)
        .map_err(|_| invalid)?;
    let required = manifest["required"]
        .as_bool()
        .ok_or("Kebijakan update wajib tidak valid")?;
    key.verify_strict(
        policy(&signed, required).as_bytes(),
        &signature("policySignature")?,
    )
    .map_err(|_| "Tanda tangan kebijakan update tidak valid")?;
    Ok(())
}

// The legacy client signs these fields in insertion order, independent of JSON map sorting.
fn canonical(version: &str, size: u64, hash: &str, published: u64) -> String {
    format!(
        r#"{{"version":{},"size":{size},"sha256":{},"publishedAt":{published},"minProtocol":2}}"#,
        json!(version),
        json!(hash)
    )
}
fn policy(canonical: &str, required: bool) -> String {
    format!(
        r#"{{"manifest":{},"required":{required}}}"#,
        json!(canonical)
    )
}

fn rust_policy(policy: &str) -> String {
    format!("ChatX/Community/Rust-CLIENT/v1\n{policy}")
}

pub(crate) fn verify_rust_manifest(manifest: &Value, trusted: &str) -> Result<()> {
    verify_manifest(manifest, trusted)?;
    if manifest["channel"] != "rust" { return Err("Manifest bukan untuk CLIENT Rust".into()); }
    let canonical = canonical(manifest["version"].as_str().unwrap(), manifest["size"].as_u64().unwrap(),
        manifest["sha256"].as_str().unwrap(), manifest["publishedAt"].as_u64().unwrap());
    let signed = rust_policy(&policy(&canonical, manifest["required"].as_bool().unwrap()));
    let signature = Signature::from_slice(&STANDARD.decode(manifest["channelSignature"].as_str()
        .ok_or("Tanda tangan kanal Rust tidak tersedia")?)?)?;
    VerifyingKey::from_public_key_pem(trusted)?.verify_strict(signed.as_bytes(), &signature)?;
    Ok(())
}

fn valid_version(version: &str) -> bool {
    version.len() <= 40
        && version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}
fn valid_name(name: &str, channel: Channel) -> bool {
    if !name.is_ascii() || name.len() > 220 || name.contains(['\\', '/', ':']) {
        return false;
    }
    let chatx = name.to_ascii_lowercase().strip_prefix("chatx-client-setup-v")
        .and_then(|suffix| suffix.strip_suffix("-andromeda.exe"))
        .is_some_and(valid_version);
    chatx
}
fn stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn save_json(path: &Path, value: &Value) -> io::Result<()> {
    crate::host::atomic_write(path, |file| {
        serde_json::to_writer(file, value).map_err(io::Error::other)
    })
}
fn read_json(path: &Path) -> io::Result<Value> {
    serde_json::from_reader(File::open(path)?).map_err(io::Error::other)
}

struct Candidate(PathBuf);
impl Candidate {
    fn new(channel: Channel) -> io::Result<Self> {
        fs::create_dir_all(channel.root())?;
        Ok(Self(
            channel.root().join(format!(".candidate-{}.exe", uuid::Uuid::new_v4())),
        ))
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(crate) fn checked_installer(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if !(MIN_INSTALLER..=MAX_INSTALLER).contains(&size) {
        return Err("Ukuran installer tidak valid".into());
    }
    let mut header = [0; 64];
    file.read_exact(&mut header)?;
    if &header[..2] != b"MZ" {
        return Err("File update bukan executable Windows".into());
    }
    let pe = u32::from_le_bytes(header[60..64].try_into()?) as u64;
    if pe < 64 || pe + 4 > size {
        return Err("Header installer Windows tidak valid".into());
    }
    file.seek(SeekFrom::Start(pe))?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"PE\0\0" {
        return Err("Header installer Windows tidak valid".into());
    }
    file.rewind()?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 512 * 1024];
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        digest.update(&buffer[..len]);
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

#[derive(Clone)]
struct Service {
    channel: Channel,
    admin: Option<String>,
    identity: Arc<Identity>,
    permit: Arc<UpdatePermit>,
    io: SocketIo,
    commit: Arc<Mutex<()>>,
}

pub fn router(
    admin: Option<String>,
    identity: Arc<Identity>,
    permit: Arc<UpdatePermit>,
    io: SocketIo,
) -> Router {
    let commit = Arc::new(Mutex::new(()));
    let channel_routes = |channel: Channel, prefix: &str| {
        Router::new()
            .route(prefix, post(upload).layer(DefaultBodyLimit::max(MAX_INSTALLER as usize + 1024 * 1024)))
            .route(&format!("{prefix}/history"), get(history))
            .route(&format!("{prefix}/rollback"), post(rollback))
            .with_state(Service {
                channel, admin: admin.clone(), identity: identity.clone(), permit: permit.clone(),
                io: io.clone(), commit: commit.clone(),
            })
    };
    channel_routes(Channel::Electron, "/admin/client-update")
        .merge(channel_routes(Channel::Rust, "/admin/rust-client-update"))
}

fn error(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(json!({"ok":false,"error":message.to_string()})),
    )
        .into_response()
}
fn authorized(service: &Service, headers: &HeaderMap) -> bool {
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    service
        .admin
        .as_deref()
        .zip(supplied)
        .is_some_and(|(expected, supplied)| {
            expected.len() == supplied.len()
                && expected
                    .bytes()
                    .zip(supplied.bytes())
                    .fold(0, |diff, (a, b)| diff | (a ^ b))
                    == 0
        })
}

async fn upload(
    State(service): State<Service>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if !authorized(&service, &headers) {
        return error(StatusCode::FORBIDDEN, "Host only");
    }
    if !service.permit.ready() {
        // Finish reading the bounded form so WebView can receive the error response.
        while let Ok(Some(mut field)) = multipart.next_field().await {
            while matches!(field.chunk().await, Ok(Some(_))) {}
        }
        return error(
            StatusCode::PRECONDITION_FAILED,
            "Buat backup sebelum publikasi update.",
        );
    }
    let parsed=async {
        let mut candidate=None;
        let mut name=String::new();
        let mut fields=json!({});
        let mut rejected=None;
        while let Some(mut field)=multipart.next_field().await? {
            let key=field.name().unwrap_or_default().to_owned();
            if key=="installer" {
                let filename=field.file_name().filter(|name|valid_name(name, service.channel)).map(str::to_owned);
                if candidate.is_some() || filename.is_none() {
                    rejected=Some(if candidate.is_some() {"Pilih satu installer CLIENT SETUP"} else {"Pilih installer ChatX CLIENT-SETUP .exe"});
                    while field.chunk().await?.is_some() {}
                    continue;
                }
                name=filename.expect("validated installer name");
                let staged=Candidate::new(service.channel)?;
                let mut output=tokio::fs::OpenOptions::new().write(true).create_new(true).open(&staged.0).await?;
                let mut size=0u64;
                while let Some(chunk)=field.chunk().await? {
                    size+=chunk.len() as u64;
                    if size>MAX_INSTALLER {return Err("Installer terlalu besar".into());}
                    output.write_all(&chunk).await?;
                }
                output.sync_all().await?;
                candidate=Some(staged);
            } else if matches!(key.as_str(),"version"|"notes"|"changelog"|"required") {
                let mut bytes=Vec::new();
                while let Some(chunk)=field.chunk().await? {
                    if bytes.len()+chunk.len()>64*1024 {return Err("Keterangan update terlalu besar".into());}
                    bytes.extend_from_slice(&chunk);
                }
                fields[&key]=json!(String::from_utf8(bytes)?);
            }
        }
        if let Some(rejected)=rejected {return Err(rejected.into());}
        let candidate=candidate.ok_or("Pilih installer CLIENT .exe")?;
        let version=fields["version"].as_str().unwrap_or_default().trim();
        if !valid_version(version) {return Err("Versi harus seperti 1.0.0".into());}
        let meta=json!({"version":version,"required":fields["required"]!="false","originalName":name,
            "notes":clip(&fields["notes"],1000),"changelog":clip(&fields["changelog"],16000)});
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>((candidate,meta))
    }.await;
    let (candidate, meta) = match parsed {
        Ok(data) => data,
        Err(failure) => return error(StatusCode::BAD_REQUEST, failure),
    };
    let worker = service.clone();
    let result = tokio::task::spawn_blocking(move || {
        let (size, hash) = checked_installer(&candidate.0)?;
        worker.activate(candidate, meta, size, &hash)
    })
    .await;
    publication_result(service, result).await
}

fn clip(value: &Value, limit: usize) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(limit)
        .collect()
}
fn archive_root(channel: Channel) -> PathBuf {
    channel.root().join("archive")
}
fn safe_archive_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 180
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
fn archived(channel: Channel, key: &str) -> Result<(Value, PathBuf)> {
    if !safe_archive_name(key) {
        return Err("Versi rollback tidak ditemukan".into());
    }
    let mut meta = read_json(&archive_root(channel).join(format!("{key}.json")))?;
    let name = meta["exe"]
        .as_str()
        .filter(|name| safe_archive_name(name) && name.ends_with(".exe"))
        .ok_or("Installer rollback tidak valid")?;
    let path = archive_root(channel).join(name);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err("Installer rollback hilang".into());
    }
    meta["key"] = json!(key);
    Ok((meta, path))
}
fn archive_items(channel: Channel) -> io::Result<Vec<Value>> {
    let entries = match fs::read_dir(archive_root(channel)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut rows = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let key = name.to_str()?.strip_suffix(".json")?;
            archived(channel, key).ok().map(|(meta, _)| meta)
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row["publishedAt"].as_u64().unwrap_or_default()));
    Ok(rows)
}

impl Service {
    fn activate(
        &self,
        candidate: Candidate,
        mut meta: Value,
        size: u64,
        hash: &str,
    ) -> Result<Value> {
        let _commit = self.commit.lock().expect("update publication");
        let published = stamp();
        let version = meta["version"]
            .as_str()
            .filter(|v| valid_version(v))
            .ok_or("Versi update tidak valid")?;
        meta["manifest"] =
            self.identity
                .sign(version, size, hash, published, meta["required"] == true, self.channel)?;
        meta["publishedAt"] = json!(published);
        if !self.permit.consume() {
            return Err(
                "Backup publikasi sudah dipakai atau kedaluwarsa. Buat backup baru.".into(),
            );
        }
        let channel = self.channel;
        let previous = match read_json(&channel.metadata()) {
            Ok(value) => Some(value),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(old) = &previous {
            if let Some(path) = installer(channel, old).filter(|path| path.is_file()) {
                fs::create_dir_all(archive_root(channel))?;
                let key = format!(
                    "{}_{}",
                    old["publishedAt"].as_u64().unwrap_or(published),
                    uuid::Uuid::new_v4()
                );
                let name = format!("{key}.exe");
                let target = archive_root(channel).join(&name);
                fs::copy(path, &target)?;
                let mut old = old.clone();
                old["exe"] = json!(name);
                old["archivedAt"] = json!(published);
                if let Err(error) = save_json(&archive_root(channel).join(format!("{key}.json")), &old) {
                    let _ = fs::remove_file(target);
                    return Err(error.into());
                }
            }
        }
        let name = format!("payload-{}.exe", uuid::Uuid::new_v4());
        let payload = channel.root().join(&name);
        fs::rename(&candidate.0, &payload)?;
        meta["installer"] = json!(name);
        if let Err(error) = save_json(&channel.metadata(), &meta) {
            let _ = fs::remove_file(payload);
            return Err(error.into());
        }
        if let Some(old) = previous
            .as_ref()
            .filter(|old| old.get("installer").is_some())
            .and_then(|old| installer(channel, old))
        {
            let _ = fs::remove_file(old);
        }
        if let Ok(rows) = archive_items(channel) {
            for row in rows.into_iter().skip(5) {
                if let Some(key) = row["key"].as_str() {
                    if let Ok((_, path)) = archived(channel, key) {
                        if fs::remove_file(&path).is_ok() {
                            let _ = fs::remove_file(archive_root(channel).join(format!("{key}.json")));
                        }
                    }
                }
            }
        }
        Ok(meta)
    }
}

async fn history(State(service): State<Service>, headers: HeaderMap) -> Response {
    if !authorized(&service, &headers) {
        return error(StatusCode::FORBIDDEN, "Host only");
    }
    let result = tokio::task::spawn_blocking(move || {
        let _commit = service.commit.lock().expect("update publication");
        Ok::<_, io::Error>(
            json!({"ok":true,"items":archive_items(service.channel)?.into_iter().take(10).collect::<Vec<_>>(),
            "current":read_json(&service.channel.metadata()).ok()}),
        )
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Riwayat update tidak bisa dibaca",
        ),
    }
}

async fn rollback(
    State(service): State<Service>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    if !authorized(&service, &headers) {
        return error(StatusCode::FORBIDDEN, "Host only");
    }
    if !service.permit.ready() {
        return error(
            StatusCode::PRECONDITION_FAILED,
            "Buat backup sebelum rollback update.",
        );
    }
    let key = input["key"].as_str().unwrap_or_default().to_owned();
    let worker = service.clone();
    let result=tokio::task::spawn_blocking(move||{
        let (archived,path)=archived(worker.channel,&key)?;
        let staged=Candidate::new(worker.channel)?;
        fs::copy(path,&staged.0)?;
        let (size,hash)=checked_installer(&staged.0)?;
        worker.identity.verify_archive(&archived,size,&hash,worker.channel)?;
        let meta=json!({"version":archived["version"],"required":archived["manifest"]["required"],
            "notes":format!("ROLLBACK · {}",clip(&archived["notes"],985)),"changelog":clip(&archived["changelog"],16000),
            "originalName":archived["originalName"],"rollbackFromArchive":key});
        worker.activate(staged,meta,size,&hash)
    }).await;
    publication_result(service, result).await
}

async fn publication_result(
    service: Service,
    result: std::result::Result<Result<Value>, tokio::task::JoinError>,
) -> Response {
    match result {
        Ok(Ok(meta)) => {
            let event = if service.channel == Channel::Rust { "rust-client-update-available" } else { "client-update-available" };
            let _ = service.io.emit(event, &meta).await;
            Json(json!({"ok":true,"meta":meta})).into_response()
        }
        Ok(Err(failure)) => error(StatusCode::BAD_REQUEST, failure),
        Err(failure) => error(StatusCode::INTERNAL_SERVER_ERROR, failure),
    }
}
