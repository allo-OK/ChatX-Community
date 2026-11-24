use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use windows::{
    Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap},
    Media::FaceAnalysis::FaceDetector,
    Security::Cryptography::CryptographicBuffer,
    Storage::Streams::InMemoryRandomAccessStream,
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};

struct Runtime;

impl Runtime {
    fn new() -> windows::core::Result<Self> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED)?; }
        Ok(Self)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { RoUninitialize(); }
    }
}

pub fn detect(encoded: &str) -> Result<Vec<Value>, String> {
    let encoded = encoded.strip_prefix("data:image/png;base64,").ok_or("Preview deteksi harus berupa PNG.")?;
    if encoded.len() > 16 * 1024 * 1024 {
        return Err("Preview deteksi wajah terlalu besar.".into());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| "Preview deteksi wajah tidak valid.")?;
    let reader = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info()
        .map_err(|_| "Preview deteksi wajah tidak valid.")?;
    let dimensions = reader.info().size();
    if !(1..=1600).contains(&dimensions.0) || !(1..=1600).contains(&dimensions.1) {
        return Err("Ukuran preview deteksi wajah tidak valid.".into());
    }
    detect_png(&bytes)
}

fn stage<T>(name: &str, result: windows::core::Result<T>) -> Result<T, String> {
    result.map_err(|error| format!("Deteksi wajah Windows gagal saat {name} (0x{:08X}).", error.code().0 as u32))
}

fn detect_png(bytes: &[u8]) -> Result<Vec<Value>, String> {
    let _runtime = stage("memulai WinRT", Runtime::new())?;
    let stream = stage("menyiapkan stream", InMemoryRandomAccessStream::new())?;
    let buffer = stage("menyiapkan PNG", CryptographicBuffer::CreateFromByteArray(bytes))?;
    stage("menyalin PNG", stream.WriteAsync(&buffer).and_then(|task| task.join()))?;
    stage("membuka stream PNG", stream.Seek(0))?;
    let decoder = stage("membaca PNG", BitmapDecoder::CreateAsync(&stream).and_then(|task| task.join()))?;
    let source = stage("mendekode PNG", decoder.GetSoftwareBitmapAsync().and_then(|task| task.join()))?;
    let bitmap = stage("mengubah format gambar", SoftwareBitmap::Convert(&source, BitmapPixelFormat::Gray8))?;
    let detector = stage("membuka detektor", FaceDetector::CreateAsync().and_then(|task| task.join()))?;
    let faces = stage("mencari wajah", detector.DetectFacesAsync(&bitmap).and_then(|task| task.join()))?;
    let count = stage("membaca jumlah wajah", faces.Size())?;
    let mut bounds = stage("membaca posisi wajah", (0..count).map(|index| faces.GetAt(index)?.FaceBox())
        .collect::<windows::core::Result<Vec<_>>>())?;
    bounds.sort_by_key(|face| (face.Y, face.X));
    Ok(bounds.into_iter().take(12).map(|face| json!({
        "x":face.X, "y":face.Y, "width":face.Width, "height":face.Height,
    })).collect())
}
