use super::*;
use serde_json::Value;

const CONFIG: &str = "storage-location.json";

fn root(data: &Path) -> io::Result<PathBuf> {
    let file = match std::fs::File::open(data.join(CONFIG)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(data.to_owned()),
        Err(error) => return Err(error),
    };
    let config: Value = serde_json::from_reader(file)?;
    match config["storageRoot"].as_str().filter(|path| !path.is_empty()) {
        Some(path) if Path::new(path).is_absolute() => Ok(PathBuf::from(path)),
        Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "Lokasi storage harus absolut")),
        None => Ok(data.to_owned()),
    }
}

pub(crate) fn upload_dir(data: &Path) -> io::Result<PathBuf> {
    Ok(root(data)?.join("uploads"))
}

pub(super) fn location() -> io::Result<Value> {
    let data = std::env::current_dir()?;
    let root = root(&data)?;
    let is_default = root == data || root.canonicalize().is_ok_and(|root| data.canonicalize().is_ok_and(|data| root == data));
    Ok(json!({"ok":true,"mode":"host-shared-storage","dataDir":data,"storageRoot":root,
        "uploadDir":root.join("uploads"),"defaultStorageRoot":data,"isDefault":is_default,
        "host":std::env::var("COMPUTERNAME").unwrap_or_default(),"migrating":false,"freeBytes":free_bytes(&root)}))
}

fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0;
    let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
        wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut(),
    ) };
    (ok != 0).then_some(free)
}

pub(super) fn migrate(input: &Value, progress: impl Fn(Value)) -> io::Result<Value> {
    let data = std::env::current_dir()?;
    let requested = Path::new(input["targetRoot"].as_str().unwrap_or_default().trim());
    if !requested.is_absolute() || !requested.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Pilih folder tujuan absolut yang tersedia"));
    }
    let root = requested.canonicalize()?;
    let source_root = self::root(&data)?;
    let source = source_root.join("uploads");
    let destination = root.join("uploads");
    if source_root == data {
        std::fs::create_dir_all(&source)?;
    }
    let source = source.canonicalize()?;
    if root.starts_with(&source) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Folder tujuan berada di dalam uploads aktif"));
    }
    std::fs::create_dir_all(&destination)?;
    let destination = destination.canonicalize()?;
    if destination == source {
        let mut result = location()?;
        result["unchanged"] = json!(true);
        return Ok(result);
    }
    if destination.parent() != Some(root.as_path()) || source.starts_with(&destination) || destination.starts_with(&source) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Folder uploads tujuan bertumpuk atau dialihkan"));
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&source)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && !entry.file_name().to_string_lossy().ends_with(".migrate-part") {
            files.push(entry.file_name());
        }
    }
    progress(json!({"active":true,"targetRoot":root,"total":files.len()}));
    let mut bytes = 0;
    let mut verified = Vec::with_capacity(files.len());
    for name in &files {
        let from = source.join(name);
        let to = destination.join(name);
        let hash = super::storage::hash(&from)?;
        match std::fs::symlink_metadata(&to) {
            Ok(meta) if meta.file_type().is_file() && super::storage::hash(&to)? == hash => {}
            Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("File tujuan berbeda: {}", name.to_string_lossy()))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => copy_verified(&from, &to, &hash)?,
            Err(error) => return Err(error),
        }
        bytes += to.metadata()?.len();
        verified.push(hash);
        progress(json!({"active":true,"targetRoot":root,"copied":verified.len(),"total":files.len(),"bytes":bytes}));
    }
    let config = json!({"storageRoot":root,"updatedAt":presence_time()});
    atomic_save(&data.join(CONFIG), config.as_object().expect("storage config"))?;
    let mut deleted = 0;
    let mut warnings = Vec::new();
    if input["cleanupSource"] == true {
        for (name, expected) in files.iter().zip(verified) {
            let from = source.join(name);
            let to = destination.join(name);
            let clean = (|| -> io::Result<()> {
                if super::storage::hash(&from)? != expected || super::storage::hash(&to)? != expected {
                    return Err(io::Error::other("Checksum berubah, salinan lama dipertahankan"));
                }
                std::fs::remove_file(from)
            })();
            match clean {
                Ok(()) => deleted += 1,
                Err(error) if warnings.len() < 20 => warnings.push(format!("{}: {error}", name.to_string_lossy())),
                Err(_) => {}
            }
        }
    }
    let mut result = location()?;
    result.as_object_mut().expect("storage result").extend(
        json!({"copied":files.len(),"total":files.len(),"bytes":bytes,"deleted":deleted,"warnings":warnings})
            .as_object().expect("storage counts").clone(),
    );
    Ok(result)
}

pub(crate) fn copy_verified(source: &Path, target: &Path, hash: &str) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let temporary = target.with_file_name(format!(".{}.migrate-part", uuid::Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        let mut input = std::fs::File::open(source)?;
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        if super::storage::hash(&temporary)? != hash {
            return Err(io::Error::other("Checksum salinan berbeda"));
        }
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain([0]).collect();
        let to: Vec<u16> = target.as_os_str().encode_wide().chain([0]).collect();
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
