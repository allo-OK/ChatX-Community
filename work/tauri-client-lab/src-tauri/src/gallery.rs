use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const COLUMNS: &str = "device_id,local_id,file_id,name,ext,size,mtime_ms,kind,previewable,root,relative_path,search_name,search_folder,owner_name,owner_pc,updated_at";
const UPSERT: &str = "ON CONFLICT(device_id,local_id) DO UPDATE SET
 file_id=excluded.file_id,name=excluded.name,ext=excluded.ext,size=excluded.size,
 mtime_ms=excluded.mtime_ms,kind=excluded.kind,previewable=excluded.previewable,root=excluded.root,
 relative_path=excluded.relative_path,search_name=excluded.search_name,search_folder=excluded.search_folder,
 owner_name=excluded.owner_name,owner_pc=excluded.owner_pc,updated_at=excluded.updated_at";
const SNAPSHOT_TTL: i64 = 30 * 60 * 1000;
const LEGACY_SNAPSHOT: &str = "legacy-register";

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Gallery {
    db: PathBuf,
    legacy: Value,
}

pub struct Device {
    pub id: String,
    pub name: String,
    pub pc: String,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn text<'a>(value: &'a Value, key: &str, default: &'a str) -> &'a str {
    value[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
}

fn trim(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

pub fn file_id(device: &str, local_id: &str) -> String {
    let hash = Sha1::digest(format!("{device}|{local_id}").as_bytes());
    hash.iter()
        .take(14)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl Gallery {
    pub fn open(root: PathBuf) -> Result<Self> {
        let legacy = match std::fs::read(root.join("network-file-index.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
            Err(error) => return Err(error.into()),
        };
        let gallery = Self {
            db: root.join("gallery-index-v2.sqlite"),
            legacy,
        };
        let mut db = gallery.connection()?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS gallery_files(
                device_id TEXT NOT NULL,local_id TEXT NOT NULL,file_id TEXT NOT NULL UNIQUE,name TEXT NOT NULL,
                ext TEXT,size INTEGER,mtime_ms REAL,kind TEXT,previewable INTEGER,root TEXT,relative_path TEXT,
                search_name TEXT,search_folder TEXT,owner_name TEXT,owner_pc TEXT,updated_at INTEGER,
                PRIMARY KEY(device_id,local_id));
             CREATE TABLE IF NOT EXISTS gallery_devices(device_id TEXT PRIMARY KEY,name TEXT,pc TEXT,last_seq INTEGER DEFAULT 0,updated_at INTEGER);
             CREATE INDEX IF NOT EXISTS idx_gallery_kind ON gallery_files(kind);
             CREATE INDEX IF NOT EXISTS idx_gallery_mtime ON gallery_files(mtime_ms DESC);
             CREATE INDEX IF NOT EXISTS idx_gallery_size ON gallery_files(size DESC);
             CREATE INDEX IF NOT EXISTS idx_gallery_name_order ON gallery_files(name COLLATE NOCASE,file_id);
             CREATE TABLE IF NOT EXISTS rust_gallery_generations(device_id TEXT PRIMARY KEY,generation TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS rust_gallery_snapshots(device_id TEXT PRIMARY KEY,id TEXT NOT NULL,generation TEXT NOT NULL,updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS rust_gallery_stage AS SELECT * FROM gallery_files WHERE 0;
             CREATE INDEX IF NOT EXISTS rust_gallery_stage_device ON rust_gallery_stage(device_id);
             CREATE TABLE IF NOT EXISTS rust_gallery_signatures(device_id TEXT PRIMARY KEY,signature TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS rust_gallery_receipts(device_id TEXT PRIMARY KEY,id TEXT NOT NULL,generation TEXT NOT NULL,seq INTEGER NOT NULL);"
        )?;
        gallery.import_legacy(&mut db)?;
        Ok(gallery)
    }

    fn import_legacy(&self, db: &mut Connection) -> Result<()> {
        let Some(devices) = self.legacy.as_object() else {
            return Ok(());
        };
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (id, row) in devices {
            let Some(files) = row["files"].as_array() else {
                continue;
            };
            if id.is_empty()
                || id.len() > 120
                || Self::count(&tx, id)? != 0
                || tx
                    .query_row(
                        "SELECT 1 FROM gallery_devices WHERE device_id=?",
                        [id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some()
            {
                continue;
            }
            let device = Device {
                id: id.clone(),
                name: trim(text(row, "name", ""), 80),
                pc: trim(text(row, "pc", ""), 80),
            };
            write_rows(&tx, &device, files, false)?;
            let seq = row["lastSeq"].as_i64().unwrap_or_default().max(0);
            let stamp = row["updatedAt"]
                .as_i64()
                .filter(|value| *value > 0)
                .unwrap_or_else(now);
            Self::save_progress(&tx, &device, seq, stamp)?;
            tx.execute(
                "INSERT OR IGNORE INTO rust_gallery_generations VALUES (?,?)",
                params![id, trim(text(row, "generation", ""), 200)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn connection(&self) -> rusqlite::Result<Connection> {
        let db = Connection::open(&self.db)?;
        db.busy_timeout(Duration::from_secs(5))?;
        Ok(db)
    }

    pub(crate) fn row_count(&self) -> rusqlite::Result<i64> {
        self.connection()?.query_row("SELECT COUNT(*) FROM gallery_files", [], |row| row.get(0))
    }

    fn generation(&self, db: &Connection, id: &str) -> rusqlite::Result<String> {
        Ok(db
            .query_row(
                "SELECT generation FROM rust_gallery_generations WHERE device_id=?",
                [id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| text(&self.legacy[id], "generation", "").into()))
    }

    fn progress(db: &Connection, id: &str) -> rusqlite::Result<(i64, i64)> {
        db.query_row("SELECT COALESCE(last_seq,0),COALESCE(updated_at,0) FROM gallery_devices WHERE device_id=?", [id],
            |row| Ok((row.get(0)?,row.get(1)?))).optional().map(Option::unwrap_or_default)
    }

    fn count(db: &Connection, id: &str) -> rusqlite::Result<i64> {
        db.query_row(
            "SELECT COUNT(*) FROM gallery_files WHERE device_id=?",
            [id],
            |row| row.get(0),
        )
    }

    pub fn status(&self, id: &str) -> Result<Value> {
        let db = self.connection()?;
        let db = db.unchecked_transaction()?;
        let (last_seq, updated_at) = Self::progress(&db, id)?;
        let signature = db
            .query_row(
                "SELECT signature FROM rust_gallery_signatures WHERE device_id=?",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| text(&self.legacy[id], "signature", "").into());
        Ok(
            json!({"ok":true,"database":true,"count":Self::count(&db,id)?,
            "generation":self.generation(&db,id)?,"lastSeq":last_seq,"updatedAt":updated_at,
            "signature":signature}),
        )
    }

    fn save_progress(
        db: &Connection,
        device: &Device,
        seq: i64,
        stamp: i64,
    ) -> rusqlite::Result<()> {
        db.execute("INSERT OR REPLACE INTO gallery_devices (device_id,name,pc,last_seq,updated_at) VALUES (?,?,?,?,?)",
            params![device.id,device.name,device.pc,seq,stamp])?;
        Ok(())
    }

    pub fn register(&self, device: &Device, raw: &Value) -> Result<Value> {
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stamp = now();
        prune_staging(&tx, stamp)?;
        if raw["reset"] == true {
            begin_staging(
                &tx,
                &device.id,
                LEGACY_SNAPSHOT,
                &trim(text(raw, "signature", ""), 80),
                stamp,
            )?;
        }
        let signature = tx
            .query_row(
                "SELECT generation FROM rust_gallery_snapshots WHERE device_id=? AND id=?",
                params![device.id, LEGACY_SNAPSHOT],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or("RESYNC_REQUIRED")?;
        let rows = raw["files"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let staged = staged_count(&tx, &device.id)?;
        if rows.len() > 5000 || staged + rows.len() > 150_000 {
            clear_staging(&tx, &device.id)?;
            tx.commit()?;
            return Err("Index Gallery melewati batas".into());
        }
        write_rows(&tx, device, rows, true)?;
        if raw["final"] == false {
            touch_staging(&tx, &device.id, stamp)?;
            tx.commit()?;
            return Ok(json!({"ok":true,"count":staged+rows.len(),"staged":true}));
        }
        publish_staging(&tx, &device.id)?;
        let (last, _) = Self::progress(&tx, &device.id)?;
        Self::save_progress(&tx, device, last.saturating_add(1).max(1), stamp)?;
        tx.execute(
            "INSERT OR REPLACE INTO rust_gallery_signatures VALUES (?,?)",
            params![device.id, signature],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO rust_gallery_generations VALUES (?,'')",
            [&device.id],
        )?;
        tx.execute(
            "DELETE FROM rust_gallery_receipts WHERE device_id=?",
            [&device.id],
        )?;
        let count = Self::count(&tx, &device.id)?;
        tx.commit()?;
        Ok(json!({"ok":true,"count":count,"persisted":true,"database":true}))
    }

    pub fn delta(&self, device: &Device, raw: &Value) -> Result<Value> {
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let generation = text(raw, "generation", "");
        let seq = raw["seq"].as_i64().unwrap_or_default().max(0);
        let reset = raw["reset"] == true;
        let upserts = raw["upserts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        if raw["full"] == true {
            let snapshot = text(raw, "snapshotId", "");
            if snapshot.is_empty()
                || snapshot == LEGACY_SNAPSHOT
                || snapshot.len() > 200
                || generation.is_empty()
                || generation.len() > 200
            {
                return Err("Snapshot tidak valid".into());
            }
            let stamp = now();
            let replayed=tx.query_row("SELECT 1 FROM rust_gallery_receipts WHERE device_id=? AND id=? AND generation=? AND seq=?",
                params![device.id,snapshot,generation,seq],|_|Ok(())).optional()?.is_some();
            if replayed {
                return Ok(json!({"ok":true,"count":Self::count(&tx,&device.id)?,
                    "lastSeq":Self::progress(&tx,&device.id)?.0,"duplicate":true,"database":true}));
            }
            prune_staging(&tx, stamp)?;
            if reset {
                begin_staging(&tx, &device.id, snapshot, generation, stamp)?;
            }
            let valid = tx.query_row("SELECT 1 FROM rust_gallery_snapshots WHERE device_id=? AND id=? AND generation=?",
                params![device.id,snapshot,generation], |_| Ok(())).optional()?.is_some();
            if !valid {
                return Err("RESYNC_REQUIRED".into());
            }
            if staged_count(&tx, &device.id)? + upserts.len() > 2_000_000 {
                return Err("Index terlalu besar".into());
            }
            write_rows(&tx, device, upserts, true)?;
            if raw["final"] != true {
                touch_staging(&tx, &device.id, stamp)?;
                tx.commit()?;
                return Ok(json!({"ok":true,"staged":true,"lastSeq":0}));
            }
            publish_staging(&tx, &device.id)?;
            tx.execute(
                "INSERT OR REPLACE INTO rust_gallery_generations VALUES (?,?)",
                params![device.id, generation],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO rust_gallery_receipts VALUES (?,?,?,?)",
                params![device.id, snapshot, generation, seq],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO rust_gallery_signatures VALUES (?,'')",
                [&device.id],
            )?;
        } else {
            let (last, updated) = Self::progress(&tx, &device.id)?;
            if generation != self.generation(&tx, &device.id)? {
                return Err("RESYNC_REQUIRED".into());
            }
            if !reset && updated != 0 && seq <= last {
                return Ok(
                    json!({"ok":true,"count":Self::count(&tx,&device.id)?,"lastSeq":last,"duplicate":true,"database":true}),
                );
            }
            if !reset && raw["baseSeq"].as_i64().is_some_and(|base| base != last) {
                return Err("RESYNC_REQUIRED".into());
            }
            clear_staging(&tx, &device.id)?;
            if reset {
                tx.execute("DELETE FROM gallery_files WHERE device_id=?", [&device.id])?;
            }
            write_rows(&tx, device, upserts, false)?;
            if let Some(deletes) = raw["deletes"].as_array() {
                let mut statement =
                    tx.prepare("DELETE FROM gallery_files WHERE device_id=? AND local_id=?")?;
                for id in deletes.iter().filter_map(Value::as_str) {
                    statement.execute(params![device.id, id])?;
                }
            }
        }
        Self::save_progress(&tx, device, seq, now())?;
        let count = Self::count(&tx, &device.id)?;
        tx.commit()?;
        Ok(json!({"ok":true,"count":count,"lastSeq":seq,"database":true}))
    }

    pub fn query(
        &self,
        raw: &Value,
        online: &HashSet<String>,
        history: &Path,
        conversations: &Value,
    ) -> Result<Value> {
        let started = std::time::Instant::now();
        let db = self.connection()?;
        let tx = db.unchecked_transaction()?;
        let kind = text(raw, "kind", "all");
        let offset = raw["offset"]
            .as_i64()
            .unwrap_or_default()
            .clamp(0, 2_000_000);
        let limit = raw["limit"].as_i64().unwrap_or(300).clamp(50, 500);
        let scope_folder = raw["scope"] == "file-folder";
        let search = text(raw, "q", "").to_lowercase();
        let terms: Vec<_> = search.split_whitespace().take(20).collect();
        let upload_dir = crate::host::storage_location::upload_dir(history.parent().unwrap_or(Path::new(".")))?;
        let chat = chat_files(history, conversations, &upload_dir)?;
        let indexed_chat = chat.len() as i64;
        let mut chat: Vec<_> = chat
            .into_iter()
            .filter(|file| {
                let name = text(file, "searchName", "");
                let folder = text(file, "searchFolder", "");
                (kind == "all" || file["kind"] == kind)
                    && terms
                        .iter()
                        .all(|term| name.contains(term) || (scope_folder && folder.contains(term)))
            })
            .collect();
        let chat_count = chat.len() as i64;
        let order = match text(raw, "sort", "az") {
            "za" => "name COLLATE NOCASE DESC,file_id",
            "newest" => "mtime_ms DESC,file_id",
            "largest" => "size DESC,file_id",
            _ => "name COLLATE NOCASE ASC,file_id",
        };
        let mut filters = Vec::new();
        let mut args = Vec::<rusqlite::types::Value>::new();
        if kind != "all" {
            filters.push("kind=?");
            args.push(kind.to_owned().into());
        }
        for term in terms {
            let escaped = term
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let like = format!("%{escaped}%");
            args.push(like.clone().into());
            if scope_folder {
                filters
                    .push("(search_name LIKE ? ESCAPE '\\' OR search_folder LIKE ? ESCAPE '\\')");
                args.push(like.into());
            } else {
                filters.push("search_name LIKE ? ESCAPE '\\'");
            }
        }
        let condition = if filters.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filters.join(" AND "))
        };
        let indexed_total: i64 =
            tx.query_row("SELECT COUNT(*) FROM gallery_files", [], |row| row.get(0))?;
        let total: i64 = tx.query_row(
            &format!("SELECT COUNT(*) FROM gallery_files {condition}"),
            params_from_iter(&args),
            |row| row.get(0),
        )?;
        let pc_offset = (offset - chat_count).max(0);
        args.extend([(offset + limit - pc_offset).into(), pc_offset.into()]);
        let mut statement = tx.prepare(&format!(
            "SELECT {COLUMNS} FROM gallery_files {condition} ORDER BY {order} LIMIT ? OFFSET ?"
        ))?;
        let mut items = statement
            .query_map(params_from_iter(&args), |row| {
                let mut item = pc_file(row)?;
                item["sourceOnline"] = json!(item["ownerDeviceId"]
                    .as_str()
                    .is_some_and(|owner| online.contains(owner)));
                Ok(item)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !chat.is_empty() {
            let cmp = |a: &Value, b: &Value| compare(a, b, text(raw, "sort", "az"));
            items.append(&mut chat);
            items.sort_by(cmp);
            items = items
                .into_iter()
                .skip((offset - pc_offset.min(total)) as usize)
                .take(limit as usize)
                .collect();
        }
        let total = total + chat_count;
        Ok(
            json!({"ok":true,"items":items,"total":total,"indexedTotal":indexed_total+indexed_chat,"offset":offset,"limit":limit,
            "hasMore":offset+(items.len() as i64)<total,"chatFiles":chat_count,"database":true,"worker":true,
            "searchesFullIndex":true,"queryMs":started.elapsed().as_millis() as u64}),
        )
    }

    pub fn find(&self, id: &str, history: &Path, rooms: &Value) -> Result<Option<Value>> {
        let db = self.connection()?;
        let pc = db
            .query_row(
                &format!("SELECT {COLUMNS} FROM gallery_files WHERE file_id=?"),
                [id],
                pc_file,
            )
            .optional()?;
        if pc.is_some() {
            return Ok(pc);
        }
        let upload_dir = crate::host::storage_location::upload_dir(history.parent().unwrap_or(Path::new(".")))?;
        Ok(
            chat_files(history, rooms, &upload_dir)?
                .into_iter()
                .find(|row| row["fileId"] == id),
        )
    }
}

fn prune_staging(db: &Connection, stamp: i64) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM rust_gallery_stage WHERE device_id IN
        (SELECT device_id FROM rust_gallery_snapshots WHERE updated_at < ?)",
        [stamp - SNAPSHOT_TTL],
    )?;
    db.execute(
        "DELETE FROM rust_gallery_snapshots WHERE updated_at < ?",
        [stamp - SNAPSHOT_TTL],
    )?;
    Ok(())
}

fn clear_staging(db: &Connection, id: &str) -> rusqlite::Result<()> {
    db.execute("DELETE FROM rust_gallery_stage WHERE device_id=?", [id])?;
    db.execute("DELETE FROM rust_gallery_snapshots WHERE device_id=?", [id])?;
    Ok(())
}

fn begin_staging(
    db: &Connection,
    device: &str,
    id: &str,
    generation: &str,
    stamp: i64,
) -> rusqlite::Result<()> {
    clear_staging(db, device)?;
    db.execute(
        "INSERT INTO rust_gallery_snapshots VALUES (?,?,?,?)",
        params![device, id, generation, stamp],
    )?;
    Ok(())
}

fn touch_staging(db: &Connection, device: &str, stamp: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE rust_gallery_snapshots SET updated_at=? WHERE device_id=?",
        params![stamp, device],
    )?;
    Ok(())
}

fn staged_count(db: &Connection, device: &str) -> rusqlite::Result<usize> {
    db.query_row(
        "SELECT COUNT(*) FROM rust_gallery_stage WHERE device_id=?",
        [device],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count as usize)
}

fn publish_staging(db: &Connection, device: &str) -> rusqlite::Result<()> {
    db.execute("DELETE FROM gallery_files WHERE device_id=?", [device])?;
    db.execute(
        &format!(
            "INSERT INTO gallery_files ({COLUMNS}) SELECT {COLUMNS} FROM rust_gallery_stage
        WHERE device_id=? ORDER BY rowid {UPSERT}"
        ),
        [device],
    )?;
    clear_staging(db, device)
}

fn pc_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"ownerDeviceId":row.get::<_,String>(0)?,"localId":row.get::<_,String>(1)?,"fileId":row.get::<_,String>(2)?,
                "name":row.get::<_,String>(3)?,"ext":row.get::<_,Option<String>>(4)?.unwrap_or_default(),
                "size":row.get::<_,Option<i64>>(5)?.unwrap_or_default(),"mtimeMs":row.get::<_,Option<f64>>(6)?.unwrap_or_default(),
                "kind":row.get::<_,Option<String>>(7)?.unwrap_or_default(),"previewable":row.get::<_,Option<bool>>(8)?.unwrap_or_default(),
                "root":row.get::<_,Option<String>>(9)?.unwrap_or_default(),"relativePath":row.get::<_,Option<String>>(10)?.unwrap_or_default(),
                "searchName":row.get::<_,Option<String>>(11)?.unwrap_or_default(),"searchFolder":row.get::<_,Option<String>>(12)?.unwrap_or_default(),
                "ownerName":row.get::<_,Option<String>>(13)?.unwrap_or_default(),"ownerPc":row.get::<_,Option<String>>(14)?.unwrap_or_default(),
                "sourceType":"pc"}),
    )
}

fn compare(a: &Value, b: &Value, sort: &str) -> std::cmp::Ordering {
    let number = |item: &Value, key: &str| item[key].as_f64().unwrap_or_default();
    let name = |item: &Value| text(item, "name", "").to_lowercase();
    let order = match sort {
        "newest" => number(b, "mtimeMs").total_cmp(&number(a, "mtimeMs")),
        "largest" => number(b, "size").total_cmp(&number(a, "size")),
        "za" => name(b).cmp(&name(a)),
        _ => name(a).cmp(&name(b)),
    };
    order.then_with(|| text(a, "fileId", "").cmp(text(b, "fileId", "")))
}

fn chat_files(history: &Path, conversations: &Value, uploads: &Path) -> Result<Vec<Value>> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for message in crate::history::gallery_attachments(history, conversations)? {
        let file = &message["file"];
        let stored = text(file, "stored", "");
        let valid = stored
            .as_bytes()
            .get(..64)
            .is_some_and(|hash| hash.iter().all(u8::is_ascii_hexdigit))
            && stored.len() <= 80
            && stored
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            && !stored.contains("..")
            && !stored.ends_with('.');
        if !valid || !seen.insert(stored.to_owned()) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(uploads.join(stored)) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let name = text(file, "name", stored);
        let ext = Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let image_mime = text(file, "mimetype", "")
            .to_ascii_lowercase()
            .starts_with("image/");
        let previewable = image_mime
            || matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg" | "heic" | "heif" | "avif"
            );
        let kind = match ext.as_str() {
            _ if previewable => "image",
            "tif" | "tiff" | "raw" | "cr2" | "nef" | "arw" | "dng" => "image",
            "psd" | "psb" => "photoshop",
            "ai" | "eps" => "illustrator",
            "cdr" | "cdt" => "coreldraw",
            "pdf" => "pdf",
            _ => "other",
        };
        let room = text(&message, "conversationId", "general");
        let label = conversations[room].as_str().unwrap_or("Chat");
        let sender = text(&message, "sender", "");
        let timestamp = message["ts"]
            .as_f64()
            .filter(|ts| *ts != 0.0)
            .or_else(|| {
                metadata
                    .modified()
                    .ok()
                    .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as f64)
            })
            .unwrap_or_default();
        rows.push(json!({"fileId":file_id("chat",stored),"localId":format!("chat:{stored}"),"name":trim(name,220),
            "ext":ext,"size":file["size"].as_u64().filter(|size|*size != 0).unwrap_or_else(||metadata.len()),"mtimeMs":timestamp,
            "kind":kind,"previewable":previewable,"root":"ChatX","relativePath":if sender.is_empty(){label.into()}else{format!("{label} / {sender}")},
            "searchName":name.to_lowercase(),"searchFolder":format!("chatx {label} {sender}").to_lowercase(),
            "ownerDeviceId":"__chat__","ownerName":if sender.is_empty(){"ChatX"}else{sender},"ownerPc":"HOST · Chat",
            "sourceOnline":true,"sourceType":"chat","chatStored":stored,"previewUrl":if previewable {format!("/uploads/{stored}")}else{String::new()}}));
    }
    Ok(rows)
}

fn write_rows(
    db: &Connection,
    device: &Device,
    rows: &[Value],
    staging: bool,
) -> rusqlite::Result<()> {
    let (table, conflict) = if staging {
        ("rust_gallery_stage", "")
    } else {
        ("gallery_files", UPSERT)
    };
    let mut put = db.prepare(&format!(
        "INSERT INTO {table} ({COLUMNS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) {conflict}"
    ))?;
    let updated = now();
    for row in rows {
        let local_id = trim(text(row, "localId", ""), 80);
        if local_id.is_empty() {
            continue;
        }
        let name = trim(text(row, "name", "file"), 220);
        let root = trim(text(row, "root", ""), 120);
        let relative = trim(text(row, "relativePath", ""), 700);
        let folder = relative
            .rsplit_once('/')
            .map(|(folder, _)| folder)
            .unwrap_or_default();
        put.execute(params![
            device.id,
            local_id,
            file_id(&device.id, &local_id),
            name,
            trim(&text(row, "ext", "").to_lowercase(), 16),
            row["size"].as_i64().unwrap_or_default().max(0),
            row["mtimeMs"].as_f64().unwrap_or_default(),
            trim(text(row, "kind", "other"), 30),
            row["previewable"] == true,
            root,
            relative,
            name.to_lowercase(),
            format!("{root} {folder}").to_lowercase(),
            device.name,
            device.pc,
            updated
        ])?;
    }
    Ok(())
}
