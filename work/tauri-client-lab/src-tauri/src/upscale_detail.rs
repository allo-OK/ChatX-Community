use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    io::{Read, Seek, SeekFrom},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
    sync::OnceLock,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

type ImagePtr = *mut c_void;
type Transform = unsafe extern "C" fn(ImagePtr, *mut ImagePtr, ...) -> c_int;
type Band = unsafe extern "C" fn(ImagePtr, *mut ImagePtr, c_int, ...) -> c_int;
type GetInt = unsafe extern "C" fn(ImagePtr) -> c_int;
const LAB: c_int = 13;
const RGB: c_int = 17;
const SRGB: c_int = 22;
const END: *const c_char = ptr::null();

struct Vips {
    open: unsafe extern "C" fn(*const c_char, ...) -> ImagePtr,
    error: unsafe extern "C" fn() -> *const c_char,
    clear_error: unsafe extern "C" fn(),
    unref: unsafe extern "C" fn(ImagePtr),
    width: GetInt,
    height: GetInt,
    interpretation: GetInt,
    format: GetInt,
    has_alpha: GetInt,
    bands: GetInt,
    autorot: Transform,
    premultiply: Transform,
    unpremultiply: Transform,
    sharpen: Transform,
    colourspace: Band,
    extract: Band,
    cast: Band,
    percent: unsafe extern "C" fn(ImagePtr, c_double, *mut c_int, ...) -> c_int,
    linear: unsafe extern "C" fn(ImagePtr, *mut ImagePtr, f64, f64, ...) -> c_int,
    join: unsafe extern "C" fn(ImagePtr, ImagePtr, *mut ImagePtr, ...) -> c_int,
    save: unsafe extern "C" fn(ImagePtr, *const c_char, ...) -> c_int,
    thumb: unsafe extern "C" fn(*const c_char, *mut ImagePtr, c_int, ...) -> c_int,
    thumb_buffer: unsafe extern "C" fn(*const c_void, usize, *mut ImagePtr, c_int, ...) -> c_int,
    array_double: unsafe extern "C" fn(*const c_double, c_int) -> *mut c_void,
    area_unref: unsafe extern "C" fn(*mut c_void),
    jpeg: unsafe extern "C" fn(ImagePtr, *mut *mut c_void, *mut usize, ...) -> c_int,
    free: unsafe extern "C" fn(*mut c_void),
}

use std::ffi::c_double;

impl Vips {
    fn load() -> Result<Self, String> {
        let path = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name("libvips-42.dll");
        let path: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        // Load the fixed app dependency using an absolute path and a restricted DLL search.
        let module = unsafe {
            LoadLibraryExW(
                path.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return Err(format!(
                "Libvips tidak tersedia: {}",
                std::io::Error::last_os_error()
            ));
        }
        macro_rules! symbol {
            ($name:literal) => {{
                let address = unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) }
                    .ok_or(concat!("Libvips tidak menyediakan ", $name))?;
                // libvips-42 is the same pinned C ABI shipped with the original app.
                unsafe { std::mem::transmute(address) }
            }};
        }
        let init: unsafe extern "C" fn(*const c_char) -> c_int = symbol!("vips_init");
        let cache_max: unsafe extern "C" fn(c_int) = symbol!("vips_cache_set_max");
        let api = Self {
            open: symbol!("vips_image_new_from_file"),
            error: symbol!("vips_error_buffer"),
            clear_error: symbol!("vips_error_clear"),
            unref: symbol!("g_object_unref"),
            width: symbol!("vips_image_get_width"),
            height: symbol!("vips_image_get_height"),
            interpretation: symbol!("vips_image_get_interpretation"),
            format: symbol!("vips_image_get_format"),
            has_alpha: symbol!("vips_image_hasalpha"),
            bands: symbol!("vips_image_get_bands"),
            autorot: symbol!("vips_autorot"),
            premultiply: symbol!("vips_premultiply"),
            unpremultiply: symbol!("vips_unpremultiply"),
            sharpen: symbol!("vips_sharpen"),
            colourspace: symbol!("vips_colourspace"),
            extract: symbol!("vips_extract_band"),
            cast: symbol!("vips_cast"),
            percent: symbol!("vips_percent"),
            linear: symbol!("vips_linear1"),
            join: symbol!("vips_bandjoin2"),
            save: symbol!("vips_pngsave"),
            thumb: symbol!("vips_thumbnail"),
            thumb_buffer: symbol!("vips_thumbnail_buffer"),
            array_double: symbol!("vips_array_double_new"),
            area_unref: symbol!("vips_area_unref"),
            jpeg: symbol!("vips_jpegsave_buffer"),
            free: symbol!("g_free"),
        };
        api.check(unsafe { init(c"ChatX".as_ptr()) })?;
        // Lazy pipelines must release temporary image files as soon as a job ends.
        unsafe { cache_max(0) };
        Ok(api)
    }

    fn check(&self, code: c_int) -> Result<(), String> {
        if code == 0 {
            return Ok(());
        }
        let message = unsafe { CStr::from_ptr((self.error)()) }
            .to_string_lossy()
            .trim()
            .to_owned();
        unsafe { (self.clear_error)() };
        Err(if message.is_empty() {
            "Operasi libvips gagal.".into()
        } else {
            message
        })
    }

    fn image(&self, path: &Path) -> Result<Image<'_>, String> {
        let path = filename(path)?;
        let raw = unsafe { (self.open)(path.as_ptr(), END) };
        if raw.is_null() {
            self.check(-1)?;
        }
        Ok(Image { raw, api: self })
    }

    fn transform(&self, run: impl FnOnce(*mut ImagePtr) -> c_int) -> Result<Image<'_>, String> {
        let mut raw = ptr::null_mut();
        self.check(run(&mut raw))?;
        Ok(Image { raw, api: self })
    }
}

struct Image<'a> {
    raw: ImagePtr,
    api: &'a Vips,
}

impl Drop for Image<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.unref)(self.raw) };
    }
}

fn filename(path: &Path) -> Result<CString, String> {
    CString::new(
        path.to_str()
            .ok_or("Alamat gambar bukan Unicode yang valid.")?,
    )
    .map_err(|error| error.to_string())
}

fn api() -> Result<&'static Vips, String> {
    static API: OnceLock<Result<Vips, String>> = OnceLock::new();
    API.get_or_init(Vips::load).as_ref().map_err(Clone::clone)
}

pub fn thumbnail(path: &Path) -> Result<Vec<u8>, String> {
    let api = api()?;
    let photoshop = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb"));
    let embedded = if photoshop {
        Some(photoshop_thumbnail(path)?)
    } else {
        let (width, height) = dimensions(path)?;
        if u64::from(width) * u64::from(height) > 268_402_689 {
            return Err("Resolusi terlalu besar untuk preview Galeri.".into());
        }
        None
    };
    let path = filename(path)?;
    // VIPS_SIZE_DOWN fits within the two dimensions while leaving small images at their native size.
    let thumb = api.transform(|out| unsafe {
        if let Some(bytes) = &embedded {
            (api.thumb_buffer)(
                bytes.as_ptr().cast(),
                bytes.len(),
                out,
                260,
                c"height".as_ptr(),
                260,
                c"size".as_ptr(),
                2,
                END,
            )
        } else {
            (api.thumb)(
                path.as_ptr(),
                out,
                260,
                c"height".as_ptr(),
                260,
                c"size".as_ptr(),
                2,
                END,
            )
        }
    })?;
    let color = api.transform(|out| unsafe { (api.colourspace)(thumb.raw, out, SRGB, END) })?;
    let (mut result, mut length) = (ptr::null_mut(), 0);
    let background = unsafe { (api.array_double)([242.0; 3].as_ptr(), 3) };
    if background.is_null() {
        return Err("Warna latar thumbnail tidak tersedia".into());
    }
    let code = unsafe {
        let result = (api.jpeg)(
            color.raw,
            &mut result,
            &mut length,
            c"Q".as_ptr(),
            60,
            c"background".as_ptr(),
            background,
            c"optimize_coding".as_ptr(),
            1,
            c"strip".as_ptr(),
            1,
            END,
        );
        (api.area_unref)(background);
        result
    };
    api.check(code)?;
    if result.is_null() {
        return Err("Thumbnail kosong".into());
    }
    let bytes = unsafe {
        let bytes = std::slice::from_raw_parts(result.cast::<u8>(), length).to_vec();
        (api.free)(result);
        bytes
    };
    Ok(bytes)
}

fn photoshop_thumbnail(path: &Path) -> Result<Vec<u8>, String> {
    fn read<const N: usize>(file: &mut std::fs::File, at: u64) -> std::io::Result<[u8; N]> {
        let mut bytes = [0; N];
        file.seek(SeekFrom::Start(at))?;
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    let unavailable =
        "File Photoshop tidak menyimpan thumbnail. Simpan dengan preview di Photoshop.";
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let total = file.metadata().map_err(|error| error.to_string())?.len();
    let header = read::<30>(&mut file, 0).map_err(|_| unavailable)?;
    if &header[..4] != b"8BPS" || !matches!(u16::from_be_bytes([header[4], header[5]]), 1 | 2) {
        return Err(unavailable.into());
    }
    let mut position =
        30 + u32::from_be_bytes(header[26..30].try_into().expect("PSD header length")) as u64;
    let resource_length =
        u32::from_be_bytes(read::<4>(&mut file, position).map_err(|_| unavailable)?) as u64;
    position += 4;
    let end = position + resource_length;
    if end > total {
        return Err(unavailable.into());
    }
    while position + 12 <= end {
        let resource = read::<7>(&mut file, position).map_err(|_| unavailable)?;
        if &resource[..4] != b"8BIM" {
            break;
        }
        let id = u16::from_be_bytes([resource[4], resource[5]]);
        let name_size = u64::from(resource[6]) + 1;
        position += 6 + name_size + (name_size % 2);
        if position + 4 > end {
            break;
        }
        let length =
            u32::from_be_bytes(read::<4>(&mut file, position).map_err(|_| unavailable)?) as u64;
        position += 4;
        if position + length > end {
            break;
        }
        if matches!(id, 1033 | 1036) && (30..=16 * 1024 * 1024).contains(&length) {
            let header = read::<30>(&mut file, position).map_err(|_| unavailable)?;
            if header[..4] == [0, 0, 0, 1] && header[28..30] == [255, 216] {
                let mut bytes = vec![0; (length - 28) as usize];
                file.seek(SeekFrom::Start(position + 28))
                    .map_err(|_| unavailable)?;
                file.read_exact(&mut bytes).map_err(|_| unavailable)?;
                return Ok(bytes);
            }
        }
        position += length + (length % 2);
    }
    Err(unavailable.into())
}

pub fn dimensions(path: &Path) -> Result<(u32, u32), String> {
    let api = api()?;
    let image = api.image(path)?;
    let dimensions = unsafe { ((api.width)(image.raw), (api.height)(image.raw)) };
    Ok((dimensions.0 as u32, dimensions.1 as u32))
}

pub fn apply(input: &Path, output: &Path, model: &str) -> Result<(), String> {
    let api = api()?;
    let input = api.image(input)?;
    let oriented = api.transform(|out| unsafe { (api.autorot)(input.raw, out, END) })?;
    let interpretation = match unsafe { (api.interpretation)(oriented.raw) } {
        RGB => SRGB,
        other => other,
    };
    let sigma = match model {
        "lite" => 1.15,
        "standard" => 1.25,
        _ => 1.4,
    };
    // Match Sharp's alpha treatment around the sharpen stage.
    let format = unsafe { (api.format)(oriented.raw) };
    let premultiplied = if unsafe { (api.has_alpha)(oriented.raw) } != 0 {
        let premultiplied =
            api.transform(|out| unsafe { (api.premultiply)(oriented.raw, out, END) })?;
        Some(api.transform(|out| unsafe { (api.cast)(premultiplied.raw, out, format, END) })?)
    } else {
        None
    };
    let source = premultiplied.as_ref().unwrap_or(&oriented);
    let sharpened = api.transform(|out| unsafe {
        (api.sharpen)(
            source.raw,
            out,
            c"sigma".as_ptr(),
            sigma,
            c"m1".as_ptr(),
            0.0,
            c"m2".as_ptr(),
            3.0,
            c"x1".as_ptr(),
            2.0,
            c"y2".as_ptr(),
            10.0,
            c"y3".as_ptr(),
            20.0,
            END,
        )
    })?;
    let color =
        api.transform(|out| unsafe { (api.colourspace)(sharpened.raw, out, interpretation, END) })?;
    let color = if premultiplied.is_some() {
        let restored = api.transform(|out| unsafe { (api.unpremultiply)(color.raw, out, END) })?;
        api.transform(|out| unsafe { (api.cast)(restored.raw, out, format, END) })?
    } else {
        color
    };
    let normalized = normalize(api, &color, interpretation)?;
    let image = normalized.as_ref().unwrap_or(&color);
    let srgb = api.transform(|out| unsafe { (api.colourspace)(image.raw, out, SRGB, END) })?;
    let output = filename(output)?;
    api.check(unsafe {
        (api.save)(
            srgb.raw,
            output.as_ptr(),
            c"compression".as_ptr(),
            7,
            c"strip".as_ptr(),
            1,
            END,
        )
    })
}

fn normalize<'a>(
    api: &'a Vips,
    image: &Image<'_>,
    interpretation: c_int,
) -> Result<Option<Image<'a>>, String> {
    let lab = api.transform(|out| unsafe { (api.colourspace)(image.raw, out, LAB, END) })?;
    let luminance = api.transform(|out| unsafe { (api.extract)(lab.raw, out, 0, END) })?;
    let (mut min, mut max) = (0, 0);
    api.check(unsafe { (api.percent)(luminance.raw, 1.0, &mut min, END) })?;
    api.check(unsafe { (api.percent)(luminance.raw, 99.0, &mut max, END) })?;
    if (max - min).abs() <= 1 {
        return Ok(None);
    }
    let chroma =
        api.transform(|out| unsafe { (api.extract)(lab.raw, out, 1, c"n".as_ptr(), 2, END) })?;
    let factor = 100.0 / f64::from(max - min);
    let adjusted = api.transform(|out| unsafe {
        (api.linear)(luminance.raw, out, factor, -f64::from(min) * factor, END)
    })?;
    let joined = api.transform(|out| unsafe { (api.join)(adjusted.raw, chroma.raw, out, END) })?;
    let color =
        api.transform(|out| unsafe { (api.colourspace)(joined.raw, out, interpretation, END) })?;
    if unsafe { (api.has_alpha)(image.raw) } != 0 {
        let bands = unsafe { (api.bands)(image.raw) };
        let alpha =
            api.transform(|out| unsafe { (api.extract)(image.raw, out, bands - 1, END) })?;
        return api
            .transform(|out| unsafe { (api.join)(color.raw, alpha.raw, out, END) })
            .map(Some);
    }
    Ok(Some(color))
}
