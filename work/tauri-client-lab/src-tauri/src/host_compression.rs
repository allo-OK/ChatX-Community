use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::{fs::{MetadataExt, OpenOptionsExt}, io::AsRawHandle},
    path::Path,
    time::Duration,
};
use windows_sys::Win32::System::{IO::DeviceIoControl, Ioctl::FSCTL_SET_COMPRESSION};

const COMPRESSED: u32 = 0x800;
const REPARSE: u32 = 0x400;

pub(crate) fn background_io<T>(run: impl FnOnce() -> T) -> T {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority,
        THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END};
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            if self.0 {
                unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END); }
            }
        }
    }
    let _restore = Restore(unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) != 0
    });
    run()
}

// NTFS keeps the original logical bytes, filename and seek offsets.
pub(super) fn enable(file: &File) -> io::Result<()> {
    let format = 1u16; // COMPRESSION_FORMAT_DEFAULT
    let mut returned = 0;
    let result = unsafe {
        DeviceIoControl(file.as_raw_handle(), FSCTL_SET_COMPRESSION,
            (&format as *const u16).cast(), 2, std::ptr::null_mut(), 0,
            &mut returned, std::ptr::null_mut())
    };
    if result == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

pub(crate) fn archive_directory(path: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
    let directory = OpenOptions::new().read(true).write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path)?;
    enable(&directory)
}

pub(crate) fn compress_idle_upload(root: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(root)?;
    if meta.file_attributes() & REPARSE != 0 {
        return Err(io::Error::other("Folder uploads memakai tautan Windows"));
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext|
            matches!(ext.to_ascii_lowercase().as_str(), "psd" | "psb" | "bmp" | "tif" | "tiff" | "txt" | "csv" | "svg")) {
            continue;
        }
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.file_attributes() & (COMPRESSED | REPARSE) != 0
            || !(64 * 1024..=16 * 1024 * 1024).contains(&meta.len()) {
            continue;
        }
        let old = |time: io::Result<std::time::SystemTime>| time.ok()
            .and_then(|time| time.elapsed().ok()).is_some_and(|age| age >= Duration::from_secs(3 * 86400));
        if !old(meta.modified()) || !old(meta.accessed()) { continue; }
        let file = match OpenOptions::new().read(true).write(true)
            .custom_flags(0x00200000).open(path) {
            Ok(file) => file,
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => continue,
            Err(error) => return Err(error),
        };
        let current = file.metadata()?;
        if current.file_attributes() & (COMPRESSED | REPARSE) != 0 || current.len() != meta.len()
            || !old(current.modified()) || !old(current.accessed()) { continue; }
        return enable(&file);
    }
    Ok(())
}
