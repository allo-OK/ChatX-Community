use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{State, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS},
    Storage::FileSystem::MoveFileExW,
};

type Result<T> = std::result::Result<T, String>;
const MAX_CHUNK: usize = 512 * 1024;

pub struct Downloads {
    journal: PathBuf,
    folders: Mutex<HashSet<PathBuf>>,
    lock: Mutex<()>,
}

impl Downloads {
    pub fn new(data: &Path) -> Self {
        Self {
            journal: data.join("gallery-transfers/sessions.json"),
            folders: Mutex::default(),
            lock: Mutex::default(),
        }
    }

    pub fn allow(&self, folder: &Path) -> Result<()> {
        self.folders
            .lock()
            .expect("gallery folders")
            .insert(folder.canonicalize().map_err(reason)?);
        Ok(())
    }

    fn read(&self) -> Result<Map<String, Value>> {
        match File::open(&self.journal) {
            Ok(file) => serde_json::from_reader(file).map_err(reason),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Map::new()),
            Err(error) => Err(reason(error)),
        }
    }

    fn save(&self, sessions: &Map<String, Value>) -> Result<()> {
        fs::create_dir_all(self.journal.parent().expect("gallery transfer directory"))
            .map_err(reason)?;
        crate::host::atomic_save(&self.journal, sessions).map_err(reason)
    }

    fn resume(&self, input: &Value) -> Result<Option<Value>> {
        let _lock = self.lock.lock().expect("gallery downloads");
        resume(&self.read()?, input)
    }

    fn start(&self, input: &Value, final_path: &Path) -> Result<Value> {
        let _lock = self.lock.lock().expect("gallery downloads");
        let mut sessions = self.read()?;
        if let Some(old) = resume(&sessions, input)? {
            return Ok(old);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let part = partial_path(final_path, &id);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .map_err(reason)?;
        sessions.insert(id.clone(),json!({"id":id,"part":part,"final":final_path,
            "fileId":input["fileId"],"expected":input["size"],"mtimeMs":input["mtimeMs"],"written":0}));
        if let Err(error) = self.save(&sessions) {
            let _ = fs::remove_file(&part);
            return Err(error);
        }
        Ok(json!({"ok":true,"id":id,"path":final_path,"offset":0}))
    }

    fn write(&self, input: &Value) -> Result<Value> {
        let encoded = input["base64"]
            .as_str()
            .filter(|encoded| encoded.len() <= 4 * MAX_CHUNK.div_ceil(3))
            .ok_or("Data chunk tidak valid")?;
        let bytes = STANDARD.decode(encoded).map_err(reason)?;
        if bytes.len() > MAX_CHUNK {
            return Err("Data chunk terlalu besar".into());
        }
        let offset = input["offset"].as_u64().ok_or("Offset tidak valid")?;
        let _lock = self.lock.lock().expect("gallery downloads");
        let mut sessions = self.read()?;
        let id = transfer_id(input)?;
        let (part, _) = paths(&sessions, id)?;
        let expected = sessions[id]["expected"]
            .as_u64()
            .ok_or("Ukuran file tidak valid")?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .filter(|end| *end <= expected)
            .ok_or("Ukuran chunk tidak valid")?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&part)
            .map_err(reason)?;
        let written = file.metadata().map_err(reason)?.len();
        file.seek(SeekFrom::Start(offset)).map_err(reason)?;
        if offset < written && end <= written {
            let mut previous = vec![0; bytes.len()];
            file.read_exact(&mut previous).map_err(reason)?;
            if previous != bytes {
                return Err("Retry chunk berbeda".into());
            }
            return Ok(json!({"ok":true,"written":written,"duplicate":true}));
        }
        if offset != written {
            return Err("Offset tidak cocok".into());
        }
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_data()) {
            let _ = file.set_len(written);
            return Err(reason(error));
        }
        sessions.get_mut(id).expect("validated transfer")["written"] = json!(end);
        self.save(&sessions)?;
        Ok(json!({"ok":true,"written":end}))
    }

    fn finish(&self, input: &Value) -> Result<Value> {
        let expected_hash = input["sha256"]
            .as_str()
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("Checksum tidak valid")?;
        let id = transfer_id(input)?;
        let _lock = self.lock.lock().expect("gallery downloads");
        let mut sessions = self.read()?;
        let (part, final_path) = paths(&sessions, id)?;
        let mut file = File::open(&part).map_err(reason)?;
        let length = file.metadata().map_err(reason)?.len();
        if sessions[id]["expected"].as_u64() != Some(length) {
            return Err("File belum lengkap".into());
        }
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; MAX_CHUNK];
        loop {
            let n = file.read(&mut buffer).map_err(reason)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        drop(file);
        let hash = format!("{:x}", hasher.finalize());
        if !hash.eq_ignore_ascii_case(expected_hash) {
            fs::remove_file(&part).map_err(reason)?;
            sessions.remove(id);
            self.save(&sessions)?;
            return Err("Checksum file tidak cocok; unduh ulang".into());
        }
        let path = finalize_without_overwrite(&part, &final_path)?;
        sessions.remove(id);
        self.save(&sessions)?;
        Ok(json!({"ok":true,"path":path,"size":length,"sha256":hash}))
    }

    fn cancel(&self, input: &Value) -> Result<Value> {
        let id = transfer_id(input)?;
        let _lock = self.lock.lock().expect("gallery downloads");
        let mut sessions = self.read()?;
        if sessions.contains_key(id) {
            let (part, _) = paths(&sessions, id)?;
            match fs::remove_file(part) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(reason(error)),
            }
            sessions.remove(id);
            self.save(&sessions)?;
        }
        Ok(json!({"ok":true}))
    }
}

fn reason(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn transfer_id(input: &Value) -> Result<&str> {
    input["id"]
        .as_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .ok_or_else(|| "Sesi unduhan tidak valid".into())
}

fn partial_path(target: &Path, id: &str) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(format!(".{id}.kcpart"));
    name.into()
}

fn paths(sessions: &Map<String, Value>, id: &str) -> Result<(PathBuf, PathBuf)> {
    let row = sessions.get(id).ok_or("Sesi unduhan tidak ditemukan")?;
    let target = PathBuf::from(row["final"].as_str().ok_or("Tujuan file tidak valid")?);
    let part = partial_path(&target, id);
    if !target.is_absolute() || row["part"].as_str().map(Path::new) != Some(part.as_path()) {
        return Err("Lokasi file parsial tidak valid".into());
    }
    Ok((part, target))
}

fn resume(sessions: &Map<String, Value>, input: &Value) -> Result<Option<Value>> {
    for (id, row) in sessions {
        if row["fileId"] == input["fileId"]
            && row["expected"] == input["size"]
            && row["mtimeMs"].as_f64() == input["mtimeMs"].as_f64()
        {
            let (part, target) = paths(sessions, id)?;
            let metadata = match fs::metadata(&part) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(reason(error)),
            };
            if metadata.is_file() && metadata.len() <= input["size"].as_u64().unwrap_or_default() {
                return Ok(Some(
                    json!({"ok":true,"id":id,"path":target,"offset":metadata.len(),"resumed":true}),
                ));
            }
        }
    }
    Ok(None)
}

fn finalize_without_overwrite(part: &Path, target: &Path) -> Result<PathBuf> {
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    };
    let source = wide(part);
    let stem = target.file_stem().ok_or("Nama file tidak valid")?;
    for n in 0..10_000 {
        let dest = if n == 0 {
            target.to_owned()
        } else {
            let mut name = stem.to_owned();
            name.push(format!(" ({n})"));
            if let Some(extension) = target.extension() {
                name.push(".");
                name.push(extension);
            }
            target.with_file_name(name)
        };
        // MoveFileExW without REPLACE_EXISTING publishes the result without clobbering a file.
        if unsafe { MoveFileExW(source.as_ptr(), wide(&dest).as_ptr(), 0) } != 0 {
            return Ok(dest);
        }
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(),Some(code) if code==ERROR_ALREADY_EXISTS as i32 || code==ERROR_FILE_EXISTS as i32)
        {
            return Err(reason(error));
        }
    }
    Err("Terlalu banyak file dengan nama yang sama".into())
}

fn owner(window: &WebviewWindow) -> Result<usize> {
    if !matches!(window.label(), "host" | "main") {
        return Err("Galeri hanya tersedia dari jendela chat.".into());
    }
    window
        .hwnd()
        .map(|handle| handle.0 as usize)
        .map_err(reason)
}

#[tauri::command]
pub async fn gallery_choose_download_folder(
    window: WebviewWindow,
    downloads: State<'_, Arc<Downloads>>,
) -> Result<Option<PathBuf>> {
    let owner = owner(&window)?;
    let downloads = downloads.inner().clone();
    tokio::task::spawn_blocking(move || {
        let selected = crate::image_files::choose_folder(owner)?;
        if let Some(path) = &selected {
            downloads.allow(path)?;
        }
        Ok(selected)
    })
    .await
    .map_err(reason)?
}

#[tauri::command]
pub async fn gallery_download(
    window: WebviewWindow,
    downloads: State<'_, Arc<Downloads>>,
    action: String,
    input: Value,
) -> Result<Value> {
    let owner = owner(&window)?;
    let downloads = downloads.inner().clone();
    tokio::task::spawn_blocking(move || match action.as_str() {
        "start" => {
            if input["fileId"]
                .as_str()
                .is_none_or(|id| id.is_empty() || id.len() > 100)
                || input["size"].as_u64().is_none()
                || input["mtimeMs"].as_f64().is_none()
            {
                return Err("Metadata unduhan tidak valid".into());
            }
            if let Some(old) = downloads.resume(&input)? {
                return Ok(old);
            }
            let original = input["name"].as_str().unwrap_or("file");
            let basename = original.rsplit(['\\', '/']).next().unwrap_or("file");
            let name: String = basename
                .chars()
                .take(170)
                .map(|ch| {
                    if ch.is_control() || matches!(ch, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                        '_'
                    } else {
                        ch
                    }
                })
                .collect();
            let name = name.trim_end_matches(['.', ' ']);
            let name = if name.is_empty() { "file" } else { name };
            let path = if let Some(directory) =
                input["directory"].as_str().filter(|dir| !dir.is_empty())
            {
                let directory = Path::new(directory).canonicalize().map_err(reason)?;
                if !downloads
                    .folders
                    .lock()
                    .expect("gallery folders")
                    .contains(&directory)
                {
                    return Err("Pilih folder tujuan melalui dialog Galeri.".into());
                }
                directory.join(name)
            } else {
                let Some(path) = crate::image_files::choose_save_path(
                    owner,
                    name,
                    "Semua file\0*.*\0\0",
                    "",
                    "Simpan file dari Gallery",
                )?
                else {
                    return Ok(json!({"ok":false,"canceled":true}));
                };
                path
            };
            downloads.start(&input, &path)
        }
        "write" => downloads.write(&input),
        "finish" => downloads.finish(&input),
        "cancel" => downloads.cancel(&input),
        _ => Err("Operasi unduhan tidak dikenal".into()),
    })
    .await
    .map_err(reason)?
}
