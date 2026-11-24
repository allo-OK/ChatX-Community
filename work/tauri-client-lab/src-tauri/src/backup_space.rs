use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{State, WebviewWindow};
use windows_sys::Win32::Storage::FileSystem::{
    FileRenameInfoEx, GetDiskFreeSpaceExW, GetFileInformationByHandle,
    SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE, FILE_RENAME_INFO,
    FILE_SHARE_DELETE, FILE_SHARE_READ,
};
use windows_sys::Win32::{
    Foundation::SYSTEMTIME,
    System::SystemInformation::{
        GetLocalTime, GetSystemTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const REPARSE: u32 = 0x400;
const MANIFEST: &str = "BACKUP_INFO.json";

pub struct BackupSpace {
    legacy_profile: Option<PathBuf>,
    root: PathBuf,
    busy: Mutex<()>,
    settings: Mutex<()>,
    snapshot: crate::host::SnapshotGate,
}

impl BackupSpace {
    pub fn new(root: PathBuf, legacy_profile: Option<PathBuf>) -> Self {
        Self {
            root,
            busy: Mutex::new(()),
            settings: Mutex::new(()),
            snapshot: crate::host::SnapshotGate::default(),
            legacy_profile,
        }
    }

    pub fn gate(&self) -> crate::host::SnapshotGate {
        self.snapshot.clone()
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("backup-config.json")
    }

    fn config(&self) -> Result<Value> {
        match File::open(self.config_path()) {
            Ok(file) => Ok(serde_json::from_reader(file)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(json!({"backupDir":"","backupKeep":7,"autoBackup":false}))
            }
            Err(error) => Err(error.into()),
        }
    }

    fn update_config(&self, update: impl FnOnce(&mut Value) -> Result<()>) -> Result<Value> {
        let _lock = self.settings.lock().expect("backup settings");
        let mut config = self.config()?;
        update(&mut config)?;
        fs::create_dir_all(&self.root)?;
        crate::host::atomic_write(&self.config_path(), |file| {
            serde_json::to_writer(file, &config).map_err(io::Error::other)
        })?;
        Ok(config)
    }

    async fn create(self: Arc<Self>, mode: SnapshotMode) -> Result<Value> {
        let snapshot = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.snapshot.clone().write_owned(),
        )
        .await
        .map_err(|_| "HOST masih memproses penulisan data. Coba backup lagi.")?;
        tokio::task::spawn_blocking(move || {
            if Path::new("storage-pending.json").exists() || self.root.join("storage-pending.json").exists() {
                return Err("Pemulihan storage belum selesai. Selesaikan operasi storage atau restart HOST sebelum backup.".into());
            }
            let _busy = self
                .busy
                .try_lock()
                .map_err(|_| "Backup lain sedang berjalan.")?;
            let config = self.config()?;
            let (base, keep) = match &mode {
                SnapshotMode::BeforeUpdate => (self.root.join("pre-update-backups"), 1),
                _ => {
                    let path = config["backupDir"]
                        .as_str()
                        .filter(|path| !path.is_empty())
                        .ok_or("Pilih folder backup dahulu")?;
                    (
                        PathBuf::from(path),
                        config["backupKeep"].as_u64().unwrap_or(7).clamp(2, 30) as usize,
                    )
                }
            };
            if let SnapshotMode::Automatic(day) = &mode {
                if config["autoBackup"] != true || config["lastAutoBackupDate"] == *day {
                    return Ok(json!({"ok":true,"skipped":true}));
                }
            }
            let mut result = crate::host::host_compression::background_io(|| create_snapshot(
                &self.root,
                &base,
                keep,
                matches!(mode, SnapshotMode::BeforeUpdate),
            ))?;
            drop(snapshot);
            if let SnapshotMode::Automatic(day) = mode {
                self.update_config(|config| {
                    config["lastAutoBackupDate"] = json!(day);
                    Ok(())
                })?;
            } else if matches!(mode, SnapshotMode::BeforeUpdate) {
                match self.compact(false) {
                    Ok(compaction) => result["compaction"] = compaction,
                    Err(error) => {
                        result["compaction"] = json!({"ok":false,"error":error.to_string()})
                    }
                }
            }
            Ok(result)
        })
        .await?
    }

    pub async fn schedule(
        self: Arc<Self>,
        status: crate::HostStatus,
        log: Arc<crate::support_log::SupportLog>,
    ) {
        let smoke_clock = std::env::args_os().nth(1).is_some_and(|arg| arg == "--smoke-gui")
            && std::env::var_os("KC_TEST_AUTO_BACKUP").is_some();
        let delay = if smoke_clock {
            std::time::Duration::from_millis(200)
        } else {
            std::time::Duration::from_secs(15 * 60)
        };
        let mut timer = tokio::time::interval(delay);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        timer.tick().await;
        loop {
            timer.tick().await;
            let mut clock = clock_time(true);
            if smoke_clock {
                clock.wHour = 18;
            }
            if clock.wHour < 18 || status.lock().expect("status HOST")["state"] != "running" {
                continue;
            }
            let day = format!("{:04}-{:02}-{:02}", clock.wYear, clock.wMonth, clock.wDay);
            match self.config() {
                Ok(config)
                    if config["autoBackup"] == true && config["lastAutoBackupDate"] != day =>
                {
                    let result = self.clone().create(SnapshotMode::Automatic(day)).await;
                    let detail = match result {
                        Ok(value) => json!({"ok":true,"fileCount":value["fileCount"]}),
                        Err(error) => json!({"ok":false,"error":error.to_string()}),
                    };
                    let _ = log.record("runtime", "auto-backup", &detail);
                }
                Err(error) => {
                    let _ = log.record(
                        "runtime",
                        "auto-backup-config-failed",
                        &json!({"error":error.to_string()}),
                    );
                }
                _ => (),
            }
        }
    }

    fn compact(&self, dry_run: bool) -> Result<Value> {
        let root = self.root.canonicalize()?;
        let mut candidates = HashMap::<(String, u64, u32), Vec<Candidate>>::new();
        let mut warnings = Vec::<String>::new();
        if !dry_run {
            for entry in fs::read_dir(&root)? {
                let entry = entry?;
                let name = entry.file_name();
                if name
                    .to_str()
                    .and_then(|name| name.strip_prefix(".kc-backup-link-"))
                    .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
                    && regular(&entry.path()).is_ok()
                {
                    if fs::remove_file(entry.path()).is_err() {
                        warnings.push("File sementara optimasi lama masih terkunci.".into());
                    }
                }
            }
        }
        let mut locations = vec![
            (root.join("pre-update-backups"), MANIFEST, false),
            (root.join("recovery"), "CHECKPOINT.json", true),
        ];
        if let Some(profile) = &self.legacy_profile {
            locations.push((profile.join("pre-update-backups"), MANIFEST, false));
            locations.push((profile.join("recovery"), "CHECKPOINT.json", true));
        }
        for (base, manifest, checkpoint) in locations {
            match fs::symlink_metadata(&base) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
                Ok(meta) if !meta.is_dir() || meta.file_attributes() & REPARSE != 0 => {
                    return Err("Folder backup berupa junction atau bukan direktori.".into())
                }
                Ok(_) => (),
            }
            for entry in fs::read_dir(base)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if !snapshot_name(&name, checkpoint) || !directory(&entry.path()).is_ok() {
                    continue;
                }
                let rows = read_candidates(&entry.path(), manifest, checkpoint);
                match rows {
                    Ok(rows) => {
                        for row in rows {
                            candidates
                                .entry((row.digest.clone(), row.size, row.identity.volume))
                                .or_default()
                                .push(row);
                        }
                    }
                    Err(_) => warnings.push(format!(
                        "Backup {name} dilewati karena manifest atau file tidak valid."
                    )),
                }
            }
        }
        let mut saved = 0;
        let mut files = 0;
        let mut checked = 0;
        for rows in candidates.values().filter(|rows| rows.len() > 1) {
            let mut identities = HashMap::new();
            for row in rows {
                *identities.entry(row.identity.key()).or_insert(0u32) += 1;
            }
            let mut first: Option<(&Candidate, File)> = None;
            for row in rows {
                if first
                    .as_ref()
                    .is_some_and(|(first, _)| first.identity.key() == row.identity.key())
                {
                    continue;
                }
                if row.identity.links > identities[&row.identity.key()] {
                    warnings.push(
                        "File memiliki hard link di luar kumpulan backup yang diverifikasi.".into(),
                    );
                    continue;
                }
                let valid = (|| -> Result<File> {
                    let mut handle = locked_file(&row.path)?;
                    let identity = identity(&handle)?;
                    if identity != row.identity
                        || handle.metadata()?.len() != row.size
                        || digest(&mut handle)? != row.digest
                    {
                        return Err("Checksum backup berbeda".into());
                    }
                    Ok(handle)
                })();
                let Ok(handle) = valid else {
                    warnings
                        .push("Checksum backup berbeda atau file berubah; file dilewati.".into());
                    continue;
                };
                checked += 1;
                let Some((source, _)) = &first else {
                    first = Some((row, handle));
                    continue;
                };
                if row.identity.links > 1 {
                    continue;
                }
                if !dry_run && link_replace(&root, source, row).is_err() {
                    warnings.push(
                        "Sebagian file tidak bisa dihemat. Salinan asli dipertahankan.".into(),
                    );
                    continue;
                }
                saved += row.size;
                files += 1;
            }
        }
        let mut seen = HashSet::new();
        warnings.retain(|message| seen.insert(message.clone()));
        warnings.truncate(12);
        Ok(
            json!({"ok":true,"dryRun":dry_run,"savedBytes":saved,"files":files,"checked":checked,
            "warnings":warnings,"method":"shared-immutable-backup-files"}),
        )
    }
}

enum SnapshotMode {
    Manual,
    BeforeUpdate,
    Automatic(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    volume: u32,
    index: u64,
    links: u32,
}
impl Identity {
    fn key(self) -> (u32, u64) {
        (self.volume, self.index)
    }
}

struct Candidate {
    path: PathBuf,
    digest: String,
    size: u64,
    identity: Identity,
}

fn identity(file: &File) -> io::Result<Identity> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        links: info.nNumberOfLinks,
    })
}

fn snapshot_name(name: &str, checkpoint: bool) -> bool {
    if checkpoint {
        let Some(version) = name
            .strip_prefix("checkpoint-host-")
            .or_else(|| name.strip_prefix("checkpoint-client-"))
        else {
            return false;
        };
        return version.split('.').count() == 3
            && version
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    }
    snapshot_timestamp(name).is_some_and(|date| {
        !date.is_empty()
            && date
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'T' | b'Z' | b'.' | b'-'))
    })
}

fn snapshot_timestamp(name: &str) -> Option<&str> {
    name.strip_prefix("ChatX-Backup-")
}

fn regular(path: &Path) -> Result<fs::Metadata> {
    let info = fs::symlink_metadata(path)?;
    if info.file_attributes() & REPARSE != 0 || !info.is_file() {
        return Err("File backup berupa junction atau bukan file reguler.".into());
    }
    Ok(info)
}

fn directory(path: &Path) -> Result<()> {
    let info = fs::symlink_metadata(path)?;
    if info.file_attributes() & REPARSE != 0 || !info.is_dir() {
        return Err("Folder backup tidak valid.".into());
    }
    Ok(())
}

fn locked_file(path: &Path) -> Result<File> {
    regular(path)?;
    Ok(OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(path)?)
}

fn resolve(root: &Path, name: &str) -> Result<PathBuf> {
    let relative = Path::new(name);
    if relative.as_os_str().is_empty()
        || relative.components().any(|part| {
            !matches!(part,Component::Normal(name)
        if !name.to_string_lossy().contains(':') && !name.to_string_lossy().ends_with(['.',' ']))
        })
    {
        return Err("Path manifest tidak aman".into());
    }
    let path = root.join(relative);
    for parent in path
        .ancestors()
        .skip(1)
        .take_while(|parent| *parent != root)
    {
        directory(parent)?;
    }
    regular(&path)?;
    if !path.canonicalize()?.starts_with(root) {
        return Err("Path manifest di luar folder backup".into());
    }
    Ok(path)
}

fn digest(file: &mut File) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 512 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn read_manifest(root: &Path, name: &str, checkpoint: bool) -> Result<Value> {
    let manifest: Value = serde_json::from_reader(locked_file(&root.join(name))?)?;
    if manifest["files"].as_array().is_none()
        || if checkpoint {
            manifest["completed"] != true
        } else {
            manifest["format"] != 2
        }
    {
        return Err("Format manifest backup tidak valid".into());
    }
    Ok(manifest)
}

fn checksum(row: &Value) -> Option<&str> {
    row["sha256"].as_str().filter(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn read_candidates(root: &Path, manifest: &str, checkpoint: bool) -> Result<Vec<Candidate>> {
    let root = root.canonicalize()?;
    let data = read_manifest(&root, manifest, checkpoint)?;
    let mut paths = HashSet::new();
    let mut rows = Vec::new();
    for row in data["files"].as_array().expect("validated manifest") {
        let Some(size) = row["size"].as_u64().filter(|size| *size >= 65536) else {
            continue;
        };
        let Some(sha) = checksum(row) else {
            continue;
        };
        let file = resolve(
            &root,
            row["path"].as_str().ok_or("Path manifest tidak valid")?,
        )?;
        if !paths.insert(file.to_string_lossy().to_lowercase()) {
            return Err("Path manifest duplikat".into());
        }
        let handle = locked_file(&file)?;
        if handle.metadata()?.len() != size {
            return Err("File backup berubah".into());
        }
        rows.push(Candidate {
            path: file,
            digest: sha.into(),
            size,
            identity: identity(&handle)?,
        });
    }
    Ok(rows)
}

fn link_replace(root: &Path, source: &Candidate, target: &Candidate) -> Result<()> {
    let temporary = root.join(format!(".kc-backup-link-{}", uuid::Uuid::new_v4()));
    fs::hard_link(&source.path, &temporary)?;
    let result = (|| {
        if identity(&locked_file(&temporary)?)?.key() != source.identity.key()
            || identity(&locked_file(&target.path)?)? != target.identity
        {
            return Err("File backup berubah saat optimasi".into());
        }
        replace_locked(&temporary, &target.path)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn replace_locked(source: &Path, target: &Path) -> Result<()> {
    const REPLACE_IF_EXISTS: u32 = 1;
    const POSIX_SEMANTICS: u32 = 2;
    let source = OpenOptions::new()
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(source)?;
    let name: Vec<_> = target.as_os_str().encode_wide().chain([0]).collect();
    let buffer_name_bytes = std::mem::size_of_val(name.as_slice());
    let name_bytes = buffer_name_bytes - std::mem::size_of::<u16>();
    let size = (std::mem::offset_of!(FILE_RENAME_INFO, FileName) + buffer_name_bytes)
        .max(std::mem::size_of::<FILE_RENAME_INFO>());
    let mut buffer = vec![
        0usize;
        size.div_ceil(std::mem::size_of::<usize>())
    ];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // POSIX replacement keeps both validated handles locked against writes through the rename.
    let replaced = unsafe {
        (*info).Anonymous.Flags = REPLACE_IF_EXISTS | POSIX_SEMANTICS;
        (*info).FileNameLength = u32::try_from(name_bytes)?;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        SetFileInformationByHandle(
            source.as_raw_handle().cast(),
            FileRenameInfoEx,
            info.cast(),
            u32::try_from(size)?,
        )
    };
    if replaced == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn backup_replacement_preserves_exact_unicode_names() -> Result<()> {
    let directory = std::env::temp_dir().join(format!("kc-backup-rename-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory)?;
    let result = (|| -> Result<()> {
        for length in 0..8 {
            let source = directory.join("replacement.tmp");
            let target = directory.join(format!("cadangan-é-🧵{}.exe", "x".repeat(length)));
            fs::write(&source, b"replacement")?;
            fs::write(&target, b"old")?;
            replace_locked(&source, &target)?;
            assert_eq!(fs::read(&target)?, b"replacement");
            assert_eq!(fs::read_dir(&directory)?.count(), 1);
            fs::remove_file(target)?;
        }
        Ok(())
    })();
    fs::remove_dir_all(directory)?;
    result
}

fn verify(root: &Path) -> Result<Value> {
    directory(root)?;
    let root = root.canonicalize()?;
    let manifest = read_manifest(&root, MANIFEST, false)?;
    let mut known = HashSet::new();
    let mut bytes = 0u64;
    for row in manifest["files"].as_array().expect("validated manifest") {
        let name = row["path"].as_str().ok_or("Path manifest tidak valid")?;
        let path = resolve(&root, name)?;
        if !known.insert(path.clone()) {
            return Err("Path manifest duplikat".into());
        }
        let mut file = locked_file(&path)?;
        let size = file.metadata()?.len();
        if row["size"].as_u64() != Some(size) || checksum(row) != Some(digest(&mut file)?.as_str())
        {
            return Err(format!("Checksum berbeda: {name}").into());
        }
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sqlite"))
        {
            let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let mut statement = db.prepare("PRAGMA integrity_check")?;
            let checks = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if checks != ["ok"] {
                return Err(format!("Database rusak: {name}").into());
            }
        }
        bytes = bytes
            .checked_add(size)
            .ok_or("Ukuran backup melewati batas")?;
    }
    let mut pending = vec![root.clone()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_attributes() & REPARSE != 0 {
                return Err("Junction di dalam backup ditolak".into());
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if path != root.join(MANIFEST) && !known.remove(&path) {
                return Err("Isi folder backup tidak cocok dengan manifest".into());
            }
        }
    }
    if !known.is_empty() {
        return Err("Isi folder backup tidak lengkap".into());
    }
    Ok(
        json!({"ok":true,"path":root,"fileCount":manifest["files"].as_array().unwrap().len(),
        "totalBytes":bytes,"createdAt":manifest["createdAt"],"manifest":manifest}),
    )
}

fn latest(base: &Path) -> Result<PathBuf> {
    directory(base)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| snapshot_name(name, false))
            && directory(&entry.path()).is_ok()
        {
            paths.push(entry.path());
        }
    }
    paths
        .into_iter()
        .max()
        .ok_or_else(|| "Belum ada backup".into())
}

fn restore(source: &Path, base: &Path, active: &Path) -> Result<Value> {
    let verified = verify(source)?;
    let source = source.canonicalize()?;
    let base = base.canonicalize()?;
    let active = active.canonicalize()?;
    if base.starts_with(&source) || base.starts_with(&active) {
        return Err("Pilih tujuan di luar backup sumber dan data aktif.".into());
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let target = base.join(format!("ChatX-Recovered-{stamp}"));
    let partial = base.join(format!(
        "ChatX-Recovered-{stamp}-{}.incomplete",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&partial)?;
    let result = (|| -> Result<Value> {
        for row in verified["manifest"]["files"]
            .as_array()
            .expect("validated manifest")
        {
            let name = row["path"].as_str().expect("verified path");
            let from = resolve(&source, name)?;
            let to = partial.join(name);
            fs::create_dir_all(to.parent().expect("restored parent"))?;
            let mut input = locked_file(&from)?;
            let mut output = OpenOptions::new().write(true).create_new(true).open(&to)?;
            io::copy(&mut input, &mut output)?;
            output.sync_all()?;
        }
        fs::copy(source.join(MANIFEST), partial.join(MANIFEST))?;
        verify(&partial)?;
        let dir = partial.join("server-data");
        fs::create_dir_all(&dir)?;
        let source_target = target.join("server-data");
        crate::host::atomic_write(&dir.join("storage-location.json"), |file| {
            serde_json::to_writer(
                file,
                &json!({"storageRoot":source_target,"updatedAt":stamp as u64}),
            )
            .map_err(io::Error::other)
        })?;
        fs::write(partial.join("RESTORE_README.txt"),
            "Backup terverifikasi. Data dipulihkan ke server-data dengan uploads lokal. Aplikasi aktif belum diubah. Simpan folder aktif lama sebagai rollback.\n")?;
        move_no_replace(&partial, &target)?;
        Ok(json!({"ok":true,"path":target,"fileCount":verified["fileCount"]}))
    })();
    result.map_err(|error| {
        format!("Pemulihan belum selesai; salinan parsial disimpan terpisah. {error}").into()
    })
}

pub(crate) fn import_host_backup(source: &Path, target: &Path) -> Result<Value> {
    let base = target.parent().ok_or("Folder data Rust tidak valid")?.canonicalize()?;
    let target = base.join(target.file_name().ok_or("Nama folder data Rust tidak valid")?);
    match fs::symlink_metadata(&target) {
        Ok(_) => return Err("Folder data Rust sudah ada. Impor memerlukan folder baru.".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let source = source.canonicalize()?;
    if target.starts_with(&source) {
        return Err("Tujuan harus berada di luar backup sumber".into());
    }
    let verified = verify(&source)?;
    let rows = verified["manifest"]["files"].as_array().expect("verified manifest");
    let mut files = Vec::with_capacity(rows.len());
    for row in rows {
        let name = row["path"].as_str().expect("verified path");
        let relative = Path::new(name);
        let destination = relative.strip_prefix("server-data").map_err(|_| format!("File backup di luar struktur HOST: {name}"))?;
        files.push((resolve(&source, name)?, destination.to_owned(), row));
    }
    check_space(&base, &files.iter().map(|(path, _, _)| path.clone()).collect::<Vec<_>>())?;
    let partial = base.join(format!("host-rust-import-{}.incomplete", uuid::Uuid::new_v4()));
    fs::create_dir(&partial)?;
    let result = (|| -> Result<Value> {
        for (from, name, row) in files {
            let destination = partial.join(&name);
            fs::create_dir_all(destination.parent().expect("import parent"))?;
            let mut input = locked_file(&from)?;
            let mut output = OpenOptions::new().write(true).create_new(true).open(&destination)?;
            io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            drop(output);
            let mut copied = locked_file(&destination)?;
            if Some(copied.metadata()?.len()) != row["size"].as_u64()
                || Some(digest(&mut copied)?.as_str()) != checksum(row)
            {
                return Err(format!("Checksum salinan berbeda: {}", name.display()).into());
            }
        }
        let history = partial.join("chat.sqlite");
        let legacy = partial.join("chat-messages-v1.sqlite");
        if !history.exists() && legacy.exists() {
            fs::rename(legacy, &history)?;
        }
        if !history.exists() {
            let messages: Vec<Value> = match locked_file(&partial.join("messages.json")) {
                Ok(file) => serde_json::from_reader(file)?,
                Err(error) if error.downcast_ref::<io::Error>().is_some_and(|error| error.kind() == io::ErrorKind::NotFound) => Vec::new(),
                Err(error) => return Err(error),
            };
            crate::history::initialize(&history)?;
            crate::history::write_batch(&history, &messages, 0)?;
        }
        let read_markers = crate::history::import_read_markers(&partial, &history)?;
        let db = Connection::open_with_flags(&history, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let messages: i64 = db.query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get(0))?;
        drop(db);
        crate::host::atomic_write(&partial.join("storage-location.json"), |file| {
            serde_json::to_writer(file, &json!({"storageRoot":target})).map_err(io::Error::other)
        })?;
        move_no_replace(&partial, &target)?;
        Ok(json!({"ok":true,"path":target,"fileCount":rows.len(),"messageCount":messages,"readMarkerCount":read_markers}))
    })();
    result.map_err(|error| format!("Impor HOST belum selesai di {}. {error}", partial.display()).into())
}

fn clock_time(local: bool) -> SYSTEMTIME {
    let mut time = SYSTEMTIME::default();
    unsafe {
        if local {
            GetLocalTime(&mut time)
        } else {
            GetSystemTime(&mut time)
        }
    };
    time
}

fn source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or("Nama file backup tidak valid")?;
            if (folder == root
                && matches!(
                    name,
                    "pre-update-backups"
                        | "recovery"
                        | "tmp"
                ))
                || name.ends_with("-wal")
                || name.ends_with("-shm")
                || name.ends_with("-journal")
                || name.ends_with(".tmp")
                || name.ends_with(".migrate-part")
                || name.starts_with(".kc-backup-link-")
            {
                continue;
            }
            let path = entry.path();
            let info = fs::symlink_metadata(&path)?;
            if info.file_attributes() & REPARSE != 0 {
                return Err(format!("Junction di data HOST ditolak: {}", path.display()).into());
            }
            if info.is_dir() {
                pending.push(path);
            } else if info.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn check_space(base: &Path, files: &[PathBuf]) -> Result<()> {
    let bytes = files.iter().try_fold(0u64, |sum, path| {
        regular(path).map(|info| sum.saturating_add(info.len()))
    })?;
    let need = bytes
        .saturating_add(bytes.div_ceil(20))
        .saturating_add(64 * 1024 * 1024);
    let base: Vec<_> = base.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0;
    if unsafe {
        GetDiskFreeSpaceExW(
            base.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    if free < need {
        return Err(format!(
            "Ruang disk backup tidak cukup. Tersedia {:.2} GB, perlu sekitar {:.2} GB.",
            free as f64 / 1073741824.0,
            need as f64 / 1073741824.0
        )
        .into());
    }
    Ok(())
}

fn create_snapshot(root: &Path, base: &Path, keep: usize, pre_update: bool) -> Result<Value> {
    let root = root.canonicalize()?;
    if pre_update {
        fs::create_dir_all(base)?;
    }
    directory(base)?;
    let base = base.canonicalize()?;
    if base.starts_with(&root) && (!pre_update || base != root.join("pre-update-backups")) {
        return Err("Folder backup harus berada di luar data aktif.".into());
    }
    let upload_dir = crate::host::storage_location::upload_dir(&root)?;
    let local_uploads = root.join("uploads");
    let mut files = source_files(&root)?;
    let external_uploads = upload_dir != local_uploads;
    let upload_dir = if external_uploads {upload_dir.canonicalize()?} else {upload_dir};
    if external_uploads {
        if base.starts_with(&upload_dir) {
            return Err("Folder backup harus berada di luar uploads aktif.".into());
        }
        files.retain(|file| !file.starts_with(&upload_dir)
            && (!file.starts_with(&local_uploads) || file.starts_with(local_uploads.join("incoming"))));
        files.extend(source_files(&upload_dir)?.into_iter()
            .filter(|file| !file.starts_with(upload_dir.join("incoming"))));
    }
    check_space(&base, &files)?;
    let time = clock_time(false);
    let label = format!(
        "ChatX-Backup-{:04}-{:02}-{:02}T{:02}-{:02}-{:02}-{:03}Z",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    );
    let target = base.join(&label);
    let partial = base.join(format!("{label}.incomplete"));
    fs::create_dir(&partial)?;
    let compression_warning = if pre_update {
        crate::host::host_compression::archive_directory(&partial).err()
            .map(|error| format!("Kompresi penyimpanan backup tidak tersedia: {error}"))
    } else { None };
    (||->Result<Value>{
        let mut rows=Vec::with_capacity(files.len());
        for source in files {
            let upload_name;
            let relative=if external_uploads && source.starts_with(&upload_dir) {
                upload_name=Path::new("uploads").join(source.strip_prefix(&upload_dir)?);
                upload_name.as_path()
            } else {source.strip_prefix(&root)?};
            let name=Path::new("server-data").join(relative);
            let target=partial.join(&name);
            fs::create_dir_all(target.parent().expect("backup file parent"))?;
            if source.extension().is_some_and(|ext|ext.eq_ignore_ascii_case("sqlite")) {
                let db=Connection::open_with_flags(&source,OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                db.busy_timeout(std::time::Duration::from_secs(5))?;
                db.execute("VACUUM INTO ?",[target.to_str().ok_or("Path database backup tidak valid")?])?;
            } else {
                let mut source=locked_file(&source)?;
                let mut output=OpenOptions::new().write(true).create_new(true).open(&target)?;
                io::copy(&mut source,&mut output)?;
                output.sync_all()?;
            }
            let mut file=locked_file(&target)?;
            rows.push(json!({"path":name.to_string_lossy().replace('\\',"/"),"size":file.metadata()?.len(),"sha256":digest(&mut file)?}));
        }
        let created_at=SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let manifest=json!({"format":2,"createdAt":created_at,"version":"1.1.43","source":{"dataDir":root,"uploadDir":upload_dir},
            "files":rows,"verified":true,"verification":"sha256+sqlite-integrity"});
        crate::host::atomic_write(&partial.join(MANIFEST),|file|serde_json::to_writer_pretty(file,&manifest).map_err(io::Error::other))?;
        verify(&partial)?;
        move_no_replace(&partial,&target)?;
        let mut warnings: Vec<_> = compression_warning.into_iter().collect();
        if let Err(error)=retain_snapshots(&base,keep) {warnings.push(error.to_string());}
        Ok(json!({"ok":true,"target":target,"fileCount":rows.len(),"warnings":warnings}))
    })().map_err(|error|format!("Backup belum selesai; salinan parsial disimpan terpisah. {error}").into())
}

fn retain_snapshots(base: &Path, keep: usize) -> Result<()> {
    for path in list_snapshots(base)?.into_iter().skip(keep) {
        if path.canonicalize()?.parent() != Some(base) {
            return Err("Folder retensi backup di luar tujuan.".into());
        }
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn list_snapshots(base: &Path) -> Result<Vec<PathBuf>> {
    let mut snapshots = Vec::new();
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let path = entry.path();
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| snapshot_name(name, false))
            && directory(&path).is_ok()
            && read_manifest(&path, MANIFEST, false).is_ok()
        {
            snapshots.push(path);
        }
    }
    snapshots.sort_by(|left, right| {
        let left = left.file_name().and_then(|name| name.to_str()).and_then(snapshot_timestamp);
        let right = right.file_name().and_then(|name| name.to_str()).and_then(snapshot_timestamp);
        right.cmp(&left)
    });
    Ok(snapshots)
}

fn inventory_files(root: &Path, mut visit: impl FnMut(&Path, u64), skipped: &mut u64) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        match fs::symlink_metadata(&path) {
            Ok(info) if info.file_attributes() & REPARSE != 0 => *skipped += 1,
            Ok(info) if info.is_file() => visit(&path, info.len()),
            Ok(info) if info.is_dir() => match fs::read_dir(path) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => pending.push(entry.path()),
                            Err(_) => *skipped += 1,
                        }
                    }
                }
                Err(_) => *skipped += 1,
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            _ => *skipped += 1,
        }
    }
}

#[tauri::command]
pub async fn host_storage_inventory(
    window: WebviewWindow,
    backup: State<'_, Arc<BackupSpace>>,
    delete_backup: Option<String>,
) -> std::result::Result<Value, String> {
    if window.label() != "host" {
        return Err("Pemindai penyimpanan hanya tersedia di HOST.".into());
    }
    let backup = backup.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<Value> {
        let _busy = backup.busy.try_lock().map_err(|_| "Operasi backup sedang berjalan. Coba lagi setelah selesai.")?;
        let config = backup.config()?;
        let mut bases = vec![(backup.root.join("pre-update-backups"), 2)];
        let mut roots = vec![
            backup.root.clone(),
            crate::host::storage_location::upload_dir(&backup.root)?,
        ];
        if let Some(profile) = &backup.legacy_profile {
            bases.push((profile.join("pre-update-backups"), 2));
            roots.push(profile.join("pre-update-backups"));
            roots.push(profile.join("recovery"));
        }
        if let Some(path) = config["backupDir"].as_str().filter(|path| !path.is_empty()) {
            let path = PathBuf::from(path);
            bases.push((path.clone(), config["backupKeep"].as_u64().unwrap_or(7).clamp(2, 30) as usize));
        }

        let mut snapshots = HashMap::new();
        for (base, keep) in bases {
            if directory(&base).is_err() {
                continue;
            }
            let base = base.canonicalize()?;
            for (index, path) in list_snapshots(&base)?.into_iter().enumerate() {
                let overlaps_profile = [&backup.root].into_iter().chain(backup.legacy_profile.as_ref())
                    .filter_map(|profile| profile.canonicalize().ok())
                    .any(|profile| profile.starts_with(&path));
                let removable = index >= keep && !overlaps_profile;
                roots.push(path.clone());
                snapshots.entry(path).and_modify(|allowed| *allowed &= removable).or_insert(removable);
            }
        }
        for base in std::iter::once(&backup.root).chain(backup.legacy_profile.as_ref()) {
            let base = base.join("recovery");
            if directory(&base).is_err() {
                continue;
            }
            for entry in fs::read_dir(base.canonicalize()?)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_name().to_str().is_some_and(|name| snapshot_name(name, true))
                    && directory(&path).is_ok()
                    && read_manifest(&path, "CHECKPOINT.json", true).is_ok()
                {
                    snapshots.insert(path, false);
                }
            }
        }
        if let Some(path) = delete_backup {
            let path = PathBuf::from(path);
            if snapshots.get(&path) != Some(&true) || path.canonicalize()? != path {
                return Err("Backup ini dilindungi atau sudah tidak tersedia.".into());
            }
            fs::remove_dir_all(&path)?;
            snapshots.remove(&path);
        }

        let mut files = std::collections::BinaryHeap::new();
        let mut snapshot_sizes = HashMap::<PathBuf, u64>::new();
        let mut skipped = 0;
        let mut roots: Vec<_> = roots.into_iter().filter(|path| directory(path).is_ok())
            .filter_map(|path| path.canonicalize().ok()).collect();
        roots.sort();
        let mut scanned = Vec::<PathBuf>::new();
        for root in roots {
            if scanned.iter().any(|parent| root.starts_with(parent)) {
                continue;
            }
            inventory_files(&root, |path, bytes| {
                if let Some(snapshot) = path.ancestors().find(|parent| snapshots.contains_key(*parent)) {
                    *snapshot_sizes.entry(snapshot.to_owned()).or_default() += bytes;
                }
                files.push(std::cmp::Reverse((bytes, path.to_owned())));
                if files.len() > 75 {
                    files.pop();
                }
            }, &mut skipped);
            scanned.push(root);
        }
        let files: Vec<_> = files.into_sorted_vec().into_iter()
            .map(|std::cmp::Reverse((bytes, path))| json!({"path":path,"bytes":bytes})).collect();
        let mut backups: Vec<_> = snapshots.into_iter().map(|(path, removable)| {
            let bytes = snapshot_sizes.get(&path).copied().unwrap_or(0);
            json!({"path":path,"name":path.file_name(),"bytes":bytes,"removable":removable})
        }).collect();
        backups.sort_by_key(|backup| std::cmp::Reverse(backup["bytes"].as_u64().unwrap_or(0)));
        Ok(json!({"ok":true,"files":files,"backups":backups,"skipped":skipped}))
    }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())
}

pub(crate) use crate::image_files::move_no_replace;

fn cpu_name() -> String {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let key: Vec<u16> = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\0".encode_utf16().collect();
    let value: Vec<u16> = "ProcessorNameString\0".encode_utf16().collect();
    let mut buffer = [0u16; 256];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    let code = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ,
            std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut bytes)
    };
    if code == 0 {
        let name = String::from_utf16_lossy(&buffer[..buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len())]);
        if !name.trim().is_empty() {
            return name.trim().to_owned();
        }
    }
    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default()
}

fn graphics_adapters() -> Vec<Value> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new(); };
    let mut adapters = Vec::new();
    for index in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else { break; };
        let Ok(info) = (unsafe { adapter.GetDesc1() }) else { continue; };
        if info.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let end = info.Description.iter().position(|unit| *unit == 0).unwrap_or(info.Description.len());
        let name = String::from_utf16_lossy(&info.Description[..end]);
        adapters.push((info.DedicatedVideoMemory, name));
    }
    adapters.sort_by_key(|(vram, _)| std::cmp::Reverse(*vram));
    adapters.into_iter().map(|(vram, name)| json!({"name":name,"vram":vram})).collect()
}

#[tauri::command]
pub async fn backup_space(
    window: WebviewWindow,
    backup: State<'_, Arc<BackupSpace>>,
    input: Value,
) -> std::result::Result<Value, String> {
    if window.label() != "host" {
        return Err("Pengelolaan backup hanya tersedia di HOST".into());
    }
    let dry_run = input["dryRun"] != false;
    if !dry_run && input["confirmed"] != true {
        return Err("Analisis lalu konfirmasi optimasi dahulu".into());
    }
    let backup = backup.inner().clone();
    tokio::task::spawn_blocking(move || {
        let _busy = backup
            .busy
            .try_lock()
            .map_err(|_| "Backup lain sedang berjalan.".to_owned())?;
        backup.compact(dry_run).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn backup_recovery(
    window: WebviewWindow,
    backup: State<'_, Arc<BackupSpace>>,
    action: String,
) -> std::result::Result<Value, String> {
    if window.label() != "host" {
        return Err("Pemulihan backup hanya tersedia di HOST".into());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    let backup = backup.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<Value> {
        let _busy = backup
            .busy
            .try_lock()
            .map_err(|_| "Backup lain sedang berjalan.")?;
        let pick = |test_env: &str| -> Result<Option<PathBuf>> {
            if std::env::args_os()
                .nth(1)
                .is_some_and(|argument| argument == "--smoke-gui")
            {
                if let Some(path) = std::env::var_os(test_env) {
                    return Ok(Some(path.into()));
                }
            }
            Ok(crate::image_files::choose_folder(owner)?)
        };
        let cancelled = || json!({"ok":false,"canceled":true});
        match action.as_str() {
            "verify" => {
                let configured = backup.config()?;
                let base = if let Some(path) = configured["backupDir"]
                    .as_str()
                    .filter(|path| !path.is_empty())
                {
                    PathBuf::from(path)
                } else {
                    let Some(base) = pick("KC_TEST_BACKUP_BASE")? else {
                        return Ok(cancelled());
                    };
                    base
                };
                verify(&latest(&base)?)
            }
            "restore" => {
                let Some(source) = pick("KC_TEST_BACKUP_SOURCE")? else {
                    return Ok(cancelled());
                };
                let Some(target) = pick("KC_TEST_BACKUP_TARGET")? else {
                    return Ok(cancelled());
                };
                restore(&source, &target, &backup.root)
            }
            _ => Err("Operasi pemulihan backup tidak dikenal".into()),
        }
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn backup_config(
    window: WebviewWindow,
    backup: State<'_, Arc<BackupSpace>>,
    action: String,
    input: Value,
) -> std::result::Result<Value, String> {
    if window.label() != "host" {
        return Err("Pengaturan backup hanya tersedia di HOST".into());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    let backup = backup.inner().clone();
    tokio::task::spawn_blocking(move||->Result<Value>{
        match action.as_str() {
            "choose"=>{
                let path=if std::env::args_os().nth(1).is_some_and(|arg|arg=="--smoke-gui") {
                    std::env::var_os("KC_TEST_BACKUP_MANUAL").map(PathBuf::from)
                } else {None};
                let path=match path {Some(path)=>Some(path),None=>crate::image_files::choose_folder(owner)?};
                let Some(path)=path else {return Ok(Value::Null);};
                directory(&path)?;
                if path.canonicalize()?.starts_with(backup.root.canonicalize()?) {
                    return Err("Folder backup harus berada di luar data aktif.".into());
                }
                backup.update_config(|config|{config["backupDir"]=json!(path);Ok(())})?;
                Ok(json!(path))
            }
            "settings"=>backup.update_config(|config|{
                if let Some(keep)=input["backupKeep"].as_u64() {config["backupKeep"]=json!(keep.clamp(2,30));}
                if let Some(auto)=input["autoBackup"].as_bool() {config["autoBackup"]=json!(auto);}
                Ok(())
            }),
            "stats"=>{
                let mut memory=MEMORYSTATUSEX{dwLength:std::mem::size_of::<MEMORYSTATUSEX>() as u32,..unsafe {std::mem::zeroed()}};
                unsafe {GlobalMemoryStatusEx(&mut memory)};
                Ok(json!({"ok":true,"version":"1.1.43","role":"host",
                    "cpu":cpu_name(),"gpus":graphics_adapters(),
                    "cpuCount":std::thread::available_parallelism().map(|count|count.get()).unwrap_or(0),
                    "totalMem":memory.ullTotalPhys,"freeMem":memory.ullAvailPhys,"backup":backup.config()?}))
            }
            _=>Err("Pengaturan backup tidak dikenal".into()),
        }
    }).await.map_err(|error|error.to_string())?.map_err(|error|error.to_string())
}

#[tauri::command]
pub async fn backup_create(
    window: WebviewWindow,
    backup: State<'_, Arc<BackupSpace>>,
    status: State<'_, crate::HostStatus>,
    log: State<'_, Arc<crate::support_log::SupportLog>>,
    pre_update: bool,
    update_permit: State<'_, Arc<crate::updates::UpdatePermit>>,
) -> std::result::Result<Value, String> {
    if window.label() != "host" || status.lock().expect("status HOST")["state"] != "running" {
        return Err("HOST belum siap membuat backup.".into());
    }
    let mode = if pre_update {
        SnapshotMode::BeforeUpdate
    } else {
        SnapshotMode::Manual
    };
    let result = backup.inner().clone().create(mode).await;
    if pre_update && result.as_ref().is_ok_and(|value|value["ok"]==true) {
        update_permit.grant();
    }
    let event = if pre_update {
        "pre-update-backup"
    } else {
        "manual-backup"
    };
    let detail = match &result {
        Ok(value) => json!({"ok":true,"fileCount":value["fileCount"]}),
        Err(error) => json!({"ok":false,"error":error.to_string()}),
    };
    let _ = log.record("runtime", event, &detail);
    result.map_err(|error| error.to_string())
}
