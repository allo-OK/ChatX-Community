use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha1::{Digest as _, Sha1};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, Metadata},
    io::{Read, Seek, SeekFrom},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{State, WebviewWindow};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_YESNO,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const REPARSE_POINT: u32 = 0x400;
const MAX_CHUNK: usize = 512 * 1024;

pub struct LocalGallery {
    path: PathBuf,
    selected: Mutex<HashSet<PathBuf>>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn mtime(metadata: &Metadata) -> f64 {
    // Windows FILETIME has 100-nanosecond precision, also used by Node fs.stat.
    metadata
        .last_write_time()
        .saturating_sub(116_444_736_000_000_000) as f64
        / 10_000.0
}

fn local_id(path: &Path) -> String {
    Sha1::digest(path.to_string_lossy().to_lowercase().as_bytes())
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn kind(ext: &str) -> Option<(&'static str, bool)> {
    Some(match ext {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" | "heic"
        | "heif" | "avif" => ("image", true),
        "raw" | "cr2" | "nef" | "arw" | "dng" => ("image", false),
        "psd" | "psb" => ("photoshop", true),
        "ai" | "eps" => ("illustrator", false),
        "cdr" | "cdt" => ("coreldraw", false),
        "pdf" => ("pdf", false),
        "afdesign" | "xcf" | "kra" => ("other", false),
        _ => return None,
    })
}

impl LocalGallery {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("gallery-index-v2.sqlite"),
            selected: Mutex::default(),
        }
    }

    pub fn allow(&self, path: &Path) {
        self.selected
            .lock()
            .expect("gallery source folders")
            .insert(path.to_owned());
    }

    fn choose(&self, owner: usize) -> Result<Value> {
        let mut picked = Vec::new();
        let mut unique = HashSet::new();
        while let Some(folder) = crate::image_files::choose_folder(owner)? {
            if unique.insert(folder.to_string_lossy().to_lowercase()) {
                self.allow(&folder);
                picked.push(folder);
            }
            let title: Vec<_> = "Folder File Gallery".encode_utf16().chain([0]).collect();
            let message: Vec<_> =
                format!("{} folder dipilih. Tambahkan folder lain?", picked.len())
                    .encode_utf16()
                    .chain([0])
                    .collect();
            let reply = unsafe {
                MessageBoxW(
                    owner as _,
                    message.as_ptr(),
                    title.as_ptr(),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                )
            };
            if reply != IDYES {
                break;
            }
        }
        Ok(json!(picked))
    }

    fn db(&self) -> Result<Connection> {
        fs::create_dir_all(self.path.parent().expect("gallery directory"))?;
        let db = Connection::open(&self.path)?;
        db.busy_timeout(Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL;
          CREATE TABLE IF NOT EXISTS files(local_id TEXT PRIMARY KEY, local_path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL, ext TEXT,size INTEGER,mtime_ms REAL,kind TEXT,previewable INTEGER,root TEXT,
            relative_path TEXT,folder_path TEXT,fingerprint TEXT,seen_scan TEXT,updated_at INTEGER);
          CREATE INDEX IF NOT EXISTS idx_files_name ON files(name COLLATE NOCASE);
          CREATE TABLE IF NOT EXISTS changes(seq INTEGER PRIMARY KEY AUTOINCREMENT,
            action TEXT NOT NULL,local_id TEXT NOT NULL,payload TEXT,ts INTEGER NOT NULL);
          CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT);")?;
        Ok(db)
    }

    fn scan(&self, input: &Value) -> Result<Value> {
        let mut db = self.db()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = roots(&tx)?;
        let mut seen = HashSet::new();
        let mut configured = Vec::<PathBuf>::new();
        {
            let selected = self.selected.lock().expect("gallery source folders");
            for folder in input["folders"].as_array().into_iter().flatten() {
                let Some(path) = folder.as_str().map(PathBuf::from) else {
                    continue;
                };
                if !path.is_absolute() || (!selected.contains(&path) && !previous.contains(&path)) {
                    return Err("Pilih folder sumber melalui dialog Galeri.".into());
                }
                if seen.insert(path.to_string_lossy().to_lowercase()) {
                    configured.push(path);
                }
            }
        }
        if input["refresh"] != true && configured == previous && count(&tx)? > 0 {
            return Ok(
                json!({"ok":true,"database":true,"unlimited":true,"cached":true,
                "folders":configured,"count":count(&tx)?,"maxSeq":sequence(&tx)?}),
            );
        }
        let stamp = now();
        let scan = uuid::Uuid::new_v4().to_string();
        let mut warnings = Vec::new();
        let mut skipped = Vec::new();
        let mut changed = 0;
        let mut deleted = 0;
        {
            let mut paths = HashSet::new();
            let mut write=tx.prepare("INSERT INTO files(local_id,local_path,name,ext,size,mtime_ms,kind,previewable,
                root,relative_path,folder_path,fingerprint,seen_scan,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                ON CONFLICT(local_id) DO UPDATE SET local_path=excluded.local_path,name=excluded.name,
                ext=excluded.ext,size=excluded.size,mtime_ms=excluded.mtime_ms,kind=excluded.kind,
                previewable=excluded.previewable,root=excluded.root,relative_path=excluded.relative_path,
                folder_path=excluded.folder_path,fingerprint=excluded.fingerprint,seen_scan=excluded.seen_scan,updated_at=excluded.updated_at
                WHERE files.size IS NOT excluded.size OR files.mtime_ms IS NOT excluded.mtime_ms
                  OR files.root IS NOT excluded.root OR files.relative_path IS NOT excluded.relative_path
                  OR files.folder_path IS NOT excluded.folder_path OR files.name IS NOT excluded.name")?;
            let mut touch = tx.prepare("UPDATE files SET seen_scan=? WHERE local_id=?")?;
            let mut change =
                tx.prepare("INSERT INTO changes(action,local_id,payload,ts) VALUES (?,?,?,?)")?;
            for root in &configured {
                let root_label = root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                let mut pending = vec![root.clone()];
                while let Some(directory) = pending.pop() {
                    let entries = match fs::read_dir(&directory) {
                        Ok(entries) => entries,
                        Err(error) => {
                            warnings.push(format!(
                                "Folder tidak terbaca; index dipertahankan: {} ({error})",
                                directory.display()
                            ));
                            skipped.push(directory);
                            continue;
                        }
                    };
                    for entry in entries {
                        let entry = match entry {
                            Ok(entry) => entry,
                            Err(_) => {
                                skipped.push(directory.clone());
                                continue;
                            }
                        };
                        let path = entry.path();
                        let metadata = match entry.metadata() {
                            Ok(meta) => meta,
                            Err(_) => {
                                skipped.push(path);
                                continue;
                            }
                        };
                        if metadata.file_attributes() & REPARSE_POINT != 0 {
                            continue;
                        }
                        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                            continue;
                        };
                        if metadata.is_dir() {
                            if !matches!(
                                name.to_ascii_lowercase().as_str(),
                                "node_modules"
                                    | ".git"
                                    | "$recycle.bin"
                                    | "system volume information"
                            ) {
                                pending.push(path);
                            }
                            continue;
                        }
                        if !metadata.is_file() {
                            continue;
                        }
                        let ext = path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        let Some((kind, previewable)) = kind(&ext) else {
                            continue;
                        };
                        let id = local_id(&path);
                        if !paths.insert(id.clone()) {
                            continue;
                        }
                        let size = i64::try_from(metadata.len())?;
                        let mtime = mtime(&metadata);
                        let relative = path
                            .strip_prefix(root)?
                            .to_string_lossy()
                            .replace('\\', "/");
                        let path_text = path.to_string_lossy();
                        let folder = root.to_string_lossy();
                        let fingerprint = format!("{size}|{mtime}");
                        let updated = write.execute(params![
                            id,
                            path_text,
                            name,
                            ext,
                            size,
                            mtime,
                            kind,
                            previewable,
                            root_label,
                            relative,
                            folder,
                            fingerprint,
                            scan,
                            stamp
                        ])?;
                        if updated > 0 {
                            let row = json!({"localId":id,"name":name,"ext":ext,"size":size,"mtimeMs":mtime,"kind":kind,
                                "previewable":previewable,"root":root_label,"relativePath":relative});
                            change.execute(params!["upsert", id, row.to_string(), stamp])?;
                            changed += 1;
                        } else {
                            touch.execute(params![scan, id])?;
                        }
                    }
                }
            }
            let stale = tx
                .prepare(
                    "SELECT local_id,local_path FROM files WHERE seen_scan IS NULL OR seen_scan<>?",
                )?
                .query_map([&scan], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut delete = tx.prepare("DELETE FROM files WHERE local_id=?")?;
            for (id, path) in stale {
                if skipped
                    .iter()
                    .any(|prefix| Path::new(&path).starts_with(prefix))
                {
                    continue;
                }
                delete.execute([&id])?;
                change.execute(params!["delete", id, Option::<String>::None, stamp])?;
                deleted += 1;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO meta VALUES ('roots',?)",
            [serde_json::to_string(&configured)?],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta VALUES ('last_scan',?)",
            [stamp.to_string()],
        )?;
        let result = json!({"ok":true,"database":true,"unlimited":true,"folders":configured,"changed":changed,"deleted":deleted,
            "count":count(&tx)?,"maxSeq":sequence(&tx)?,"warnings":warnings});
        tx.commit()?;
        Ok(result)
    }

    fn index(&self, action: &str, input: &Value) -> Result<Value> {
        let db = self.db()?;
        let limit = input["limit"].as_i64().unwrap_or(1000).clamp(100, 2000);
        match action {
            "status" => {
                db.execute(
                    "INSERT OR IGNORE INTO meta VALUES ('generation',?)",
                    [uuid::Uuid::new_v4().to_string()],
                )?;
                let generation: String =
                    db.query_row("SELECT value FROM meta WHERE key='generation'", [], |row| {
                        row.get(0)
                    })?;
                Ok(
                    json!({"ok":true,"generation":generation,"count":count(&db)?,"maxSeq":sequence(&db)?}),
                )
            }
            "export" => {
                let tx = db.unchecked_transaction()?;
                let offset = input["offset"].as_i64().unwrap_or(0).max(0);
                let items=tx.prepare("SELECT local_id,name,ext,size,mtime_ms,kind,previewable,root,relative_path
                    FROM files ORDER BY rowid LIMIT ? OFFSET ?")?.query_map(params![limit,offset],|row|
                    Ok(json!({"localId":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"ext":row.get::<_,String>(2)?,
                        "size":row.get::<_,i64>(3)?,"mtimeMs":row.get::<_,f64>(4)?,"kind":row.get::<_,String>(5)?,
                        "previewable":row.get::<_,bool>(6)?,"root":row.get::<_,String>(7)?,"relativePath":row.get::<_,String>(8)?})))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let count = count(&tx)?;
                Ok(
                    json!({"ok":true,"items":items,"count":count,"maxSeq":sequence(&tx)?,"hasMore":offset.saturating_add(items.len() as i64)<count}),
                )
            }
            "changes" => {
                let after = input["afterSeq"].as_i64().unwrap_or(0).max(0);
                let mut statement=db.prepare("SELECT seq,action,local_id,payload FROM changes WHERE seq>? ORDER BY seq LIMIT ?")?;
                let mut query = statement.query(params![after, limit])?;
                let mut items = Vec::new();
                let mut seq = after;
                while let Some(row) = query.next()? {
                    seq = row.get(0)?;
                    let payload = row
                        .get::<_, Option<String>>(3)?
                        .map(|payload| serde_json::from_str::<Value>(&payload))
                        .transpose()?;
                    items.push(json!({"seq":seq,"action":row.get::<_,String>(1)?,"localId":row.get::<_,String>(2)?,"row":payload}));
                }
                Ok(
                    json!({"ok":true,"items":items,"maxSeq":seq,"hasMore":items.len() as i64==limit}),
                )
            }
            "ack" => {
                let seq = input["seq"].as_i64().ok_or("Urutan indeks tidak valid")?;
                db.execute("DELETE FROM changes WHERE seq<=?", [seq])?;
                Ok(json!({"ok":true}))
            }
            _ => Err("Operasi indeks tidak dikenal".into()),
        }
    }

    fn source(&self, id: &str) -> Result<PathBuf> {
        let db = self.db()?;
        let (path, root): (String, String) = db
            .query_row(
                "SELECT local_path,folder_path FROM files WHERE local_id=?",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or("File sumber tidak ditemukan / perlu scan ulang")?;
        let root = PathBuf::from(root);
        if !roots(&db)?.contains(&root) {
            return Err("Folder sumber sudah dilepas dari Galeri".into());
        }
        let path = Path::new(&path).canonicalize()?;
        if !path.starts_with(root.canonicalize()?) {
            return Err("File berada di luar folder sumber Galeri".into());
        }
        Ok(path)
    }

    fn file(&self, action: &str, input: &Value) -> Result<Value> {
        let id = input["localId"]
            .as_str()
            .ok_or("Identitas file tidak valid")?;
        let path = self.source(id)?;
        if action == "preview" {
            let metadata = fs::metadata(&path)?;
            let cache_key: String =
                Sha1::digest(format!("{id}|{}|{}", metadata.len(), mtime(&metadata)).as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
            let directory = self.path.with_file_name("gallery-thumbnails-v2");
            let cached = directory.join(format!("{cache_key}.jpg"));
            let bytes = match File::open(&cached).and_then(|file| {
                let mut bytes = Vec::new();
                file.take(2 * 1024 * 1024).read_to_end(&mut bytes)?;
                Ok(bytes)
            }) {
                Ok(bytes) if bytes.starts_with(&[255, 216]) && bytes.ends_with(&[255, 217]) => {
                    bytes
                }
                _ => {
                    let bytes = crate::upscale::gallery_thumbnail(&path)?;
                    let _ = fs::create_dir_all(directory).and_then(|_| {
                        crate::host::atomic_write(&cached, |file| {
                            use std::io::Write;
                            file.write_all(&bytes)
                        })
                    });
                    bytes
                }
            };
            return Ok(
                json!({"ok":true,"thumbnail":true,"dataUrl":format!("data:image/jpeg;base64,{}",STANDARD.encode(bytes))}),
            );
        }
        let mut file = File::open(&path)?;
        let meta = file.metadata()?;
        let total = meta.len();
        match action {
            "info" => {
                let mut hash = Sha256::new();
                let mut bytes = vec![0; MAX_CHUNK];
                loop {
                    let size = file.read(&mut bytes)?;
                    if size == 0 {
                        break;
                    }
                    hash.update(&bytes[..size]);
                }
                let latest = file.metadata()?;
                if total != latest.len() || meta.last_write_time() != latest.last_write_time() {
                    return Err("File sumber berubah saat dibaca".into());
                }
                Ok(
                    json!({"ok":true,"size":total,"mtimeMs":mtime(&meta),"sha256":format!("{:x}",hash.finalize())}),
                )
            }
            "chunk" => {
                let offset = input["offset"].as_u64().ok_or("Offset tidak valid")?;
                if offset > total {
                    return Err("File sumber berubah; Scan Ulang".into());
                }
                let length = input["length"]
                    .as_u64()
                    .unwrap_or(MAX_CHUNK as u64)
                    .clamp(65536, MAX_CHUNK as u64);
                let mut bytes = vec![0; (total - offset).min(length) as usize];
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(&mut bytes)?;
                let next = offset + bytes.len() as u64;
                Ok(
                    json!({"ok":true,"offset":offset,"total":total,"nextOffset":next,"eof":next==total,"base64":STANDARD.encode(bytes)}),
                )
            }
            _ => Err("Operasi file tidak dikenal".into()),
        }
    }
}

fn count(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
}
fn sequence(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row(
        "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='changes'),0)",
        [],
        |row| row.get(0),
    )
}
fn roots(db: &Connection) -> Result<Vec<PathBuf>> {
    let roots = db
        .query_row("SELECT value FROM meta WHERE key='roots'", [], |row| {
            row.get::<_, String>(0)
        })
        .optional()?;
    Ok(roots
        .map(|roots| serde_json::from_str(&roots))
        .transpose()?
        .unwrap_or_default())
}

#[tauri::command]
pub async fn gallery_local(
    window: WebviewWindow,
    gallery: State<'_, Arc<LocalGallery>>,
    action: String,
    input: Value,
) -> std::result::Result<Value, String> {
    if !matches!(window.label(), "host" | "main") {
        return Err("Galeri hanya tersedia dari jendela chat.".into());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    let gallery = gallery.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<Value> {
        match action.as_str() {
            "choose" => gallery.choose(owner),
            "scan" => crate::host::host_compression::background_io(|| gallery.scan(&input)),
            "preview" | "info" | "chunk" => gallery.file(&action, &input),
            _ => gallery.index(&action, &input),
        }
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}
