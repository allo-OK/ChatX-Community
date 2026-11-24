use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{io::Cursor, sync::{Arc, Mutex}, time::Duration};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HWND},
    Graphics::Gdi::{CreateCompatibleDC, DeleteDC, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData},
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
    },
};

const CF_UNICODETEXT: u32 = 13;
const CF_DIBV5: u32 = 17;
const MAX_PIXELS: u64 = 80_000_000;
const CF_BITMAP: u32 = 2;
const MAX_PNG_BYTES: usize = 128 * 1024 * 1024;

#[tauri::command]
pub async fn read_clipboard_image(window: tauri::WebviewWindow) -> Result<Value, String> {
    if !matches!(window.label(), "main" | "host") {
        return Err("Gambar clipboard hanya tersedia di jendela chat".into());
    }
    let hwnd = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    tokio::task::spawn_blocking(move || read_image(hwnd)
        .unwrap_or_else(|error| json!({"ok":false,"error":error})))
        .await.map_err(|error| error.to_string())
}

fn read_image(window: usize) -> Result<Value, String> {
    let clipboard = ClipboardGuard::open(window as HWND)?;
    let format = unsafe { RegisterClipboardFormatW([80, 78, 71, 0].as_ptr()) };
    if format != 0 {
        let handle = unsafe { GetClipboardData(format) };
        if !handle.is_null() {
            let length = unsafe { GlobalSize(handle) };
            if (1..=MAX_PNG_BYTES).contains(&length) {
                let pointer = unsafe { GlobalLock(handle) };
                if !pointer.is_null() {
                    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) }.to_vec();
                    unsafe { GlobalUnlock(handle); }
                    let metadata = png::Decoder::new(Cursor::new(&bytes)).read_info();
                    if let Ok(reader) = metadata {
                        let (width, height) = reader.info().size();
                        if u64::from(width) * u64::from(height) <= MAX_PIXELS {
                            drop(clipboard);
                            return Ok(image_response(&bytes, width, height));
                        }
                    }
                }
            }
        }
    }
    let (width, height, mut rgba) = read_bitmap()?;
    drop(clipboard);
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.write_header().and_then(|mut writer| writer.write_image_data(&rgba))
            .map_err(|error| error.to_string())?;
    }
    Ok(image_response(&bytes, width, height))
}

fn image_response(bytes: &[u8], width: u32, height: u32) -> Value {
    let base64 = STANDARD.encode(bytes);
    json!({"ok":true,"dataUrl":format!("data:image/png;base64,{base64}"),"base64":base64,"width":width,"height":height})
}

fn read_bitmap() -> Result<(u32, u32, Vec<u8>), String> {
    let bitmap = unsafe { GetClipboardData(CF_BITMAP) };
    if bitmap.is_null() { return Err("Clipboard tidak berisi gambar".into()); }
    let mut data = BITMAP::default();
    if unsafe { GetObjectW(bitmap, std::mem::size_of::<BITMAP>() as i32, (&mut data as *mut BITMAP).cast()) } == 0 {
        return Err("Ukuran bitmap clipboard tidak dapat dibaca".into());
    }
    let (width, height) = (data.bmWidth, data.bmHeight);
    if width <= 0 || height <= 0 || (width as u64) * (height as u64) > MAX_PIXELS {
        return Err("Ukuran gambar clipboard terlalu besar atau tidak valid".into());
    }
    let mut pixels = vec![0; width as usize * height as usize * 4];
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        ..Default::default()
    };
    let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
    if dc.is_null() { return Err("Windows gagal menyiapkan pembaca bitmap".into()); }
    let lines = unsafe { GetDIBits(dc, bitmap, 0, height as u32, pixels.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS) };
    unsafe { DeleteDC(dc); }
    if lines != height { return Err("Bitmap clipboard gagal dibaca".into()); }
    Ok((width as u32, height as u32, pixels))
}

#[derive(Default)]
pub struct Clipboard(Mutex<u64>);

impl Clipboard {
    fn start(&self) -> u64 {
        let mut generation = self.0.lock().expect("clipboard state");
        *generation = generation.wrapping_add(1);
        *generation
    }

    fn write(&self, window: usize, generation: u64, values: &[(u32, &[u8])]) -> Result<(), String> {
        let current = self.0.lock().expect("clipboard state");
        if *current != generation {
            return Err("Salin gambar dibatalkan karena ada salinan clipboard yang lebih baru".into());
        }
        let data = values.iter().map(|(_, bytes)| GlobalBuffer::new(bytes)).collect::<Result<Vec<_>, _>>()?;
        let clipboard = ClipboardGuard::open(window as HWND)?;
        if unsafe { EmptyClipboard() } == 0 {
            return Err("Clipboard Windows gagal dikosongkan".into());
        }
        for ((format, _), buffer) in values.iter().zip(data) {
            buffer.submit(*format)?;
        }
        drop(clipboard);
        Ok(())
    }

    pub async fn copy_text(self: Arc<Self>, window: usize, text: String) -> Result<Value, String> {
        let generation = self.start();
        tokio::task::spawn_blocking(move || {
            let bytes = text_bytes(&text, 2 * 1024 * 1024);
            if bytes.len() == 2 {
                return Err("Teks untuk disalin kosong".into());
            }
            self.write(window, generation, &[(CF_UNICODETEXT, &bytes)])?;
            Ok(json!({"ok":true,"length":bytes.len() / 2 - 1}))
        }).await.map_err(|error| error.to_string())?
    }

    pub async fn copy_mixed(self: Arc<Self>, window: usize, input: Value) -> Result<Value, String> {
        let generation = self.start();
        let (images, text) = tokio::task::spawn_blocking(move || validate_payload(input))
            .await.map_err(|error| error.to_string())??;
        let count = images.len();
        let text_length = text.len() / 2 - 1;
        if text_length > 0 {
            let clipboard = self.clone();
            tokio::task::spawn_blocking(move || clipboard.write(window, generation, &[(CF_UNICODETEXT, &text)]))
                .await.map_err(|error| error.to_string())??;
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        let mut size = (0, 0);
        for (index, bytes) in images.into_iter().rev().enumerate() {
            let clipboard = self.clone();
            size = tokio::task::spawn_blocking(move || {
                let (width, height, dib) = png_to_dib(&bytes)?;
                let format = unsafe { RegisterClipboardFormatW([80, 78, 71, 0].as_ptr()) };
                if format == 0 {
                    return Err("Format PNG clipboard tidak tersedia".into());
                }
                clipboard.write(window, generation, &[(CF_DIBV5, &dib), (format, &bytes)])?;
                Ok::<_, String>((width, height))
            }).await.map_err(|error| error.to_string())??;
            if index + 1 < count {
                tokio::time::sleep(Duration::from_millis(900)).await;
            }
        }
        let mut summary = json!({"ok":true,"width":size.0,"height":size.1,"textLength":text_length});
        if count > 1 {
            summary["previewCount"] = json!(count - 1);
            summary["totalClipboardEntries"] = json!(count + usize::from(text_length > 0));
        }
        Ok(summary)
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open(window: HWND) -> Result<Self, String> {
        for _ in 0..8 {
            if unsafe { OpenClipboard(window) } != 0 {
                return Ok(Self);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        Err("Clipboard Windows sedang dipakai aplikasi lain".into())
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

struct GlobalBuffer(HGLOBAL);

impl GlobalBuffer {
    fn new(bytes: &[u8]) -> Result<Self, String> {
        let buffer = Self(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) });
        if buffer.0.is_null() {
            return Err("Memori clipboard tidak tersedia".into());
        }
        let target = unsafe { GlobalLock(buffer.0) };
        if target.is_null() {
            return Err("Memori clipboard tidak dapat ditulis".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast(), bytes.len());
            GlobalUnlock(buffer.0);
        }
        Ok(buffer)
    }

    fn submit(mut self, format: u32) -> Result<(), String> {
        if unsafe { SetClipboardData(format, self.0) }.is_null() {
            return Err("Clipboard Windows gagal menerima data".into());
        }
        self.0 = std::ptr::null_mut();
        Ok(())
    }
}

impl Drop for GlobalBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { GlobalFree(self.0) };
        }
    }
}

fn text_bytes(text: &str, limit: usize) -> Vec<u8> {
    text.encode_utf16().take_while(|unit| *unit != 0).take(limit).chain([0])
        .flat_map(u16::to_le_bytes).collect()
}

fn validate_payload(input: Value) -> Result<(Vec<Vec<u8>>, Vec<u8>), String> {
    let previews = input["extraImages"].as_array().into_iter().flatten().filter(|value| !value.is_null());
    let images = std::iter::once(&input["png"]).chain(previews.map(|entry| &entry["png"]));
    let mut validated = Vec::new();
    for image in images {
        let raw = image.as_str().ok_or("PNG rincian order kosong")?;
        let base64 = raw.strip_prefix("data:image/png;base64,").unwrap_or(raw);
        let bytes = STANDARD.decode(base64).map_err(|_| "PNG rincian order tidak valid")?;
        png_to_dib(&bytes)?;
        validated.push(bytes);
    }
    let text = text_bytes(input["text"].as_str().unwrap_or_default(), 12000);
    Ok((validated, text))
}

fn png_to_dib(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| "PNG rincian order tidak valid")?;
    let (width, height) = reader.info().size();
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_PIXELS || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err("Satu halaman terlalu besar. Gunakan Simpan PNG untuk customer.".into());
    }
    let mut output = vec![0; reader.output_buffer_size().ok_or("Ukuran PNG tidak valid")?];
    let info = reader.next_frame(&mut output).map_err(|_| "PNG rincian order tidak valid")?;
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        _ => return Err("Warna PNG tidak didukung".into()),
    };
    let mut dib = vec![0; 124];
    for (offset, value) in [(0, 124), (4, width), (8, (-(height as i32)) as u32),
        (16, 3), (20, (pixels * 4) as u32), (40, 0x00ff0000), (44, 0x0000ff00),
        (48, 0x000000ff), (52, 0xff000000), (56, 0x73524742), (108, 4)] {
        dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32u16.to_le_bytes());
    dib.reserve(pixels as usize * 4);
    for pixel in output[..info.buffer_size()].chunks_exact(channels) {
        let color = match pixel {
            [r, g, b, a] => [*b, *g, *r, *a],
            [r, g, b] => [*b, *g, *r, 255],
            [gray, a] => [*gray, *gray, *gray, *a],
            [gray] => [*gray, *gray, *gray, 255],
            _ => unreachable!(),
        };
        dib.extend_from_slice(&color);
    }
    Ok((width, height, dib))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[10, 20, 30, 40, 50, 60, 70, 80]).unwrap();
        }
        bytes
    }

    #[test]
    fn png_becomes_top_down_bgra_with_alpha() {
        let bytes = sample_png();
        let (width, height, dib) = png_to_dib(&bytes).unwrap();
        assert_eq!((width, height), (1, 2));
        assert_eq!(i32::from_le_bytes(dib[8..12].try_into().unwrap()), -2);
        assert_eq!(&dib[124..], &[30, 20, 10, 40, 70, 60, 50, 80]);
        assert!(png_to_dib(b"not a png").is_err());
    }

    #[test]
    fn invalid_secondary_page_prevents_the_whole_copy() {
        let result = validate_payload(json!({"png":STANDARD.encode(sample_png()),"text":"CS","extraImages":[{"png":"missing"}]}));
        assert!(result.is_err());
        assert_eq!(text_bytes("AB", 1), b"A\0\0\0");
    }
}
