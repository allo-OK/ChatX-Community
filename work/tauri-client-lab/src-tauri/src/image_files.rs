use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{collections::HashSet, ffi::{OsStr, OsString}, io::{Cursor, Read, Write}, os::windows::ffi::{OsStrExt, OsStringExt}, path::{Path, PathBuf}, sync::Mutex};
use windows_sys::Win32::{
    Foundation::HWND,
    System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED},
    UI::{
        Controls::Dialogs::{CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OPENFILENAMEW,
            OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST},
        Shell::{SHBrowseForFolderW, SHGetPathFromIDListW, ShellExecuteW, BROWSEINFOW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}

pub(crate) fn move_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source = wide(source.as_os_str());
    let target = wide(target.as_os_str());
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn choose_images(owner: usize, multiple: bool) -> Result<Vec<PathBuf>, String> {
    choose_files(owner, multiple, "Gambar\0*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.tif;*.tiff\0\0", None, "Pilih gambar")
}

pub(crate) fn choose_examples(owner: usize, directory: Option<&Path>, artwork: bool) -> Result<Vec<PathBuf>, String> {
    let filter = if artwork { "File contoh\0*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.tif;*.tiff;*.gif\0\0" }
        else { "File contoh\0*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.tif;*.tiff;*.gif;*.psd;*.psb;*.pdf\0\0" };
    choose_files(owner, true, filter, directory, "Pilih file desain")
}


fn choose_files(owner: usize, multiple: bool, filter: &str, directory: Option<&Path>, title: &str) -> Result<Vec<PathBuf>, String> {
    let filter = wide(OsStr::new(filter));
    let title = wide(OsStr::new(title));
    let directory = directory.map(|path| wide(path.as_os_str()));
    let mut files = vec![0u16; 32768];
    let mut config = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as HWND,
        lpstrFilter: filter.as_ptr(),
        lpstrTitle: title.as_ptr(),
        lpstrInitialDir: directory.as_ref().map_or(std::ptr::null(), |path| path.as_ptr()),
        lpstrFile: files.as_mut_ptr(),
        nMaxFile: files.len() as u32,
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR
            | if multiple { OFN_ALLOWMULTISELECT } else { 0 },
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { GetOpenFileNameW(&mut config) } == 0 {
        let code = unsafe { CommDlgExtendedError() };
        return if code == 0 { Ok(Vec::new()) } else { Err(format!("Pemilih file Windows gagal ({code})")) };
    }
    let mut paths = files.split(|value| *value == 0).take_while(|value| !value.is_empty())
        .map(|part| PathBuf::from(OsString::from_wide(part)));
    let Some(first) = paths.next() else { return Ok(Vec::new()) };
    let following: Vec<_> = paths.map(|part| first.join(part)).collect();
    Ok(if following.is_empty() { vec![first] } else { following })
}

pub fn choose_folder(owner: usize) -> Result<Option<PathBuf>, String> {
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if initialized < 0 {
        return Err("Pemilih folder Windows gagal dibuka".into());
    }
    let mut display = [0u16; 260];
    let title = wide(OsStr::new("Pilih folder output"));
    let config = BROWSEINFOW {
        hwndOwner: owner as HWND, pszDisplayName: display.as_mut_ptr(), lpszTitle: title.as_ptr(),
        ulFlags: 0x0051, ..unsafe { std::mem::zeroed() }
    };
    let item = unsafe { SHBrowseForFolderW(&config) };
    let mut path = [0u16; 260];
    let selected = !item.is_null() && unsafe { SHGetPathFromIDListW(item, path.as_mut_ptr()) } != 0;
    unsafe {
        if !item.is_null() { CoTaskMemFree(item.cast()) }
        CoUninitialize();
    }
    let end = path.iter().position(|unit| *unit == 0).unwrap_or(path.len());
    Ok(selected.then(|| PathBuf::from(OsString::from_wide(&path[..end]))))
}

#[derive(Default)]
pub struct PngOutputs(Mutex<HashSet<PathBuf>>);

impl PngOutputs {
    pub fn allow(&self, folder: &Path) -> Result<(), String> {
        self.0.lock().expect("image output folders").insert(
            folder.canonicalize().map_err(|error| format!("Folder output tidak dapat dibuka: {error}"))?);
        Ok(())
    }

    pub fn write(&self, path: &Path, encoded: &str) -> Result<Value, String> {
        let name = path.file_name().filter(|_| path.is_absolute() && mime(path) == Some("image/png"))
            .ok_or("Pilih lokasi file PNG yang valid.")?;
        let folder = path.parent().and_then(|parent| parent.canonicalize().ok()).ok_or("Folder output tidak ditemukan.")?;
        if !self.0.lock().expect("image output folders").contains(&folder) {
            return Err("Pilih folder output melalui dialog aplikasi terlebih dahulu.".into());
        }
        let bytes = decode_png(encoded)?;
        persist_png(&folder.join(name), &bytes)?;
        Ok(json!({"ok":true,"path":path,"size":bytes.len()}))
    }

    pub fn write_vector(&self, input: &Value) -> Result<Value, String> {
        let source = Path::new(input["input"].as_str().unwrap_or_default());
        let default = source.parent().filter(|_| source.is_absolute() && source.is_file() && mime(source).is_some())
            .ok_or("File gambar tidak ditemukan.")?;
        let requested = input["outputDir"].as_str().filter(|value| !value.is_empty()).map(Path::new);
        let folder = requested.unwrap_or(default).canonicalize().map_err(|_| "Folder output tidak ditemukan.")?;
        if requested.is_some() && !self.0.lock().expect("image output folders").contains(&folder) {
            return Err("Pilih folder output melalui dialog aplikasi terlebih dahulu.".into());
        }
        let name = source.file_stem().and_then(OsStr::to_str).unwrap_or("gambar");
        let name: String = name.chars().take(120).collect();
        let preset = input["preset"].as_str().filter(|preset| matches!(*preset, "logo" | "color" | "detail"))
            .ok_or("Preset vektor tidak valid.")?;
        let svg = input["svg"].as_str().filter(|value| value.starts_with("<?xml") && value.ends_with("</svg>\n")
            && value.len() <= 64 * 1024 * 1024).ok_or("SVG keluaran tidak valid.")?;
        let output_format = input["outputFormat"].as_str().unwrap_or("svg");
        let raster = match output_format {
            "svg" => None,
            "png" => Some(decode_png(input["raster"].as_str().unwrap_or_default())?),
            "webp" | "jpg" => {
                let encoded = input["raster"].as_str().filter(|value| value.len() <= 140 * 1024 * 1024)
                    .ok_or("Data gambar keluaran tidak valid.")?;
                let decoded = STANDARD.decode(encoded).map_err(|_| "Data gambar keluaran tidak valid.")?;
                let valid = if output_format == "jpg" { decoded.starts_with(&[0xff, 0xd8, 0xff]) }
                    else { decoded.starts_with(b"RIFF") && decoded.get(8..12) == Some(b"WEBP") };
                if !valid { return Err("Format gambar keluaran tidak valid.".into()); }
                Some(decoded)
            }
            _ => return Err("Format keluaran tidak dikenal.".into()),
        };
        let output_folder = requested.unwrap_or(default);
        let filename = format!("{name}_KC-vector_{preset}");
        let svg_path = folder.join(format!("{filename}.svg"));
        persist_image(&svg_path, svg.as_bytes())?;
        if let Some(bytes) = &raster {
            persist_image(&folder.join(format!("{filename}.{output_format}")), bytes)?;
        }
        let output = output_folder.join(format!("{filename}.{output_format}"));
        Ok(json!({"ok":true,"output":output,"size":raster.as_ref().map_or(svg.len(), Vec::len)}))
    }
}

pub fn save_png(owner: usize, input: &Value) -> Result<Value, String> {
    let bytes = decode_png(input["base64"].as_str().unwrap_or_default())?;
    let name = input["name"].as_str().unwrap_or("removed-background.png");
    let name: String = name.chars().take(180).map(|ch| {
        if ch.is_control() || matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { ch }
    }).collect();
    let Some(path) = choose_save_path(owner,&name,"PNG Transparan\0*.png\0\0","png","Simpan PNG")? else {
        return Ok(json!({"ok":false,"canceled":true}));
    };
    persist_png(&path, &bytes)?;
    Ok(json!({"ok":true,"path":path}))
}

pub fn choose_save_path(owner: usize, name: &str, filter: &str, extension: &str, title: &str) -> Result<Option<PathBuf>,String> {
    let filter = wide(OsStr::new(filter));
    let extension = wide(OsStr::new(extension));
    let title = wide(OsStr::new(title));
    let mut file = vec![0u16; 32768];
    let default = wide(OsStr::new(name));
    file[..default.len()].copy_from_slice(&default);
    let mut config = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner as HWND, lpstrFilter: filter.as_ptr(), nFilterIndex: 1, lpstrDefExt: extension.as_ptr(),
        lpstrTitle: title.as_ptr(),
        lpstrFile: file.as_mut_ptr(), nMaxFile: file.len() as u32,
        Flags: OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { GetSaveFileNameW(&mut config) } == 0 {
        let code = unsafe { CommDlgExtendedError() };
        return if code == 0 { Ok(None) }
            else { Err(format!("Dialog simpan gagal ({code})")) };
    }
    let end = file.iter().position(|unit| *unit == 0).unwrap_or(file.len());
    Ok(Some(PathBuf::from(OsString::from_wide(&file[..end]))))
}

pub(crate) fn decode_png(encoded: &str) -> Result<Vec<u8>, String> {
    let encoded = encoded.strip_prefix("data:image/png;base64,").unwrap_or(encoded);
    if encoded.is_empty() { return Err("Data PNG kosong".into()); }
    if encoded.len() > 140 * 1024 * 1024 { return Err("Data PNG terlalu besar untuk diekspor.".into()); }
    let bytes = STANDARD.decode(encoded).map_err(|_| "Data PNG tidak valid.")?;
    let reader = png::Decoder::new(Cursor::new(&bytes)).read_info().map_err(|_| "Data PNG tidak valid.")?;
    if reader.info().width == 0 || reader.info().height == 0 {
        return Err("Ukuran PNG tidak valid.".into());
    }
    drop(reader);
    Ok(bytes)
}

fn persist_png(path: &Path, bytes: &[u8]) -> Result<(), String> {
    persist_image(path, bytes).map_err(|error| format!("Gagal menyimpan PNG: {error}"))
}

fn persist_image(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::host::atomic_write(path, |file| file.write_all(bytes))
        .map_err(|error| error.to_string())
}

fn mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"), "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"), "bmp" => Some("image/bmp"), "tif" | "tiff" => Some("image/tiff"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

pub fn read_image(path: &Path, preview: bool) -> Result<Value, String> {
    let kind = mime(path).filter(|_| path.is_absolute()).ok_or("Format gambar tidak didukung")?;
    let limit = 100 * 1024 * 1024;
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err("Gambar kosong atau terlalu besar untuk dimuat".into());
    }
    let encoded = STANDARD.encode(bytes);
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or("gambar");
    if preview {
        Ok(json!({"ok":true,"dataUrl":format!("data:{kind};base64,{encoded}")}))
    } else {
        Ok(json!({"ok":true,"base64":encoded,"name":name,"type":kind,"mimetype":kind}))
    }
}

pub fn open_path(owner: usize, path: &Path) -> Result<Value, String> {
    if !path.is_absolute() || !(path.is_dir() || path.is_file() && mime(path).is_some()) {
        return Err("Gambar atau folder output tidak ditemukan".into());
    }
    open_associated_path(owner, path)
}

pub(crate) fn open_associated_path(owner: usize, path: &Path) -> Result<Value, String> {
    let target = wide(path.as_os_str());
    let result = unsafe {
        ShellExecuteW(owner as HWND, std::ptr::null(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    if result as usize <= 32 {
        return Err("Windows gagal membuka gambar atau folder output".into());
    }
    Ok(json!({"ok":true}))
}
#[tauri::command]
pub fn open_vectorizer_web() -> Result<(), String> {
    let target: Vec<u16> = "https://vectorizer.ai/".encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), std::ptr::null(), target.as_ptr(),
            std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    if result as isize <= 32 {
        return Err("Vectorizer.AI tidak bisa dibuka di browser.".into());
    }
    Ok(())
}
