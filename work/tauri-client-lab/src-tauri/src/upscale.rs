use serde_json::{json, Value};
use std::{io::Read, os::windows::process::CommandExt, path::{Path, PathBuf}, process::{Child, Command, Stdio}, sync::mpsc, time::{Duration, Instant, SystemTime}};

#[path = "upscale_detail.rs"]
mod detail;

pub use detail::thumbnail as gallery_thumbnail;

pub struct Engine {
    binary: PathBuf,
    models: PathBuf,
}

pub fn detect() -> Option<Engine> {
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    std::iter::once(program_files.join("Upscayl/resources"))
        .chain(local.iter().flat_map(|base| [base.join("Programs/Upscayl/resources"), base.join("upscayl/resources")]))
        .find_map(|root| {
            let models = root.join("models");
            let binary = root.join("bin/upscayl-bin.exe");
            (binary.is_file() && models.is_dir()).then_some(Engine { binary, models })
        })
}

fn model_names(key: &str) -> (&'static str, &'static [&'static str]) {
    match key {
        "lite" => ("lite", &["upscayl-lite-4x"]),
        "fidelity" => ("fidelity", &["high-fidelity-4x"]),
        "remacri" => ("remacri", &["remacri-4x"]),
        "ultramix" => ("ultramix", &["ultramix-balanced-4x"]),
        "ultrasharp" => ("ultrasharp", &["ultrasharp-4x", "ultrasharp"]),
        "digital" => ("digital", &["digital-art-4x"]),
        _ => ("standard", &["upscayl-standard-4x"]),
    }
}

impl Engine {
    pub fn status(&self) -> Value {
        json!({"ok":true,"engine":"Upscayl","modelsPath":self.models})
    }

    pub fn run(self, request: Value, emit: impl Fn(u8, &str)) -> Result<Value, String> {
        if request["computeMode"] == "cpu" {
            return Err("Upscale AI memakai Vulkan GPU. Mode CPU tidak tersedia.".into());
        }
        let input = PathBuf::from(request["input"].as_str().unwrap_or_default());
        if !input.is_absolute() || !input.is_file() {
            return Err("Gambar input tidak ditemukan".into());
        }
        let (key, names) = model_names(request["model"].as_str().unwrap_or_default());
        let model = names.iter().copied().find(|name| {
            self.models.join(format!("{name}.param")).is_file() && self.models.join(format!("{name}.bin")).is_file()
        }).ok_or("Model Upscayl tidak tersedia")?;
        let scale = match request["scale"].as_str().or_else(|| request["scale"].as_u64().map(|n| match n {
            3 => "3", 4 => "4", _ => "2",
        })) {
            Some("3") => "3", Some("4") => "4", _ => "2",
        };
        let folder = request["outputDir"].as_str().filter(|dir| !dir.is_empty()).map(PathBuf::from)
            .or_else(|| input.parent().map(Path::to_path_buf)).ok_or("Folder output belum tersedia")?;
        if !folder.is_absolute() {
            return Err("Folder output harus berupa alamat absolut".into());
        }
        std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let stem: String = input.file_stem().and_then(|stem| stem.to_str()).unwrap_or("gambar").chars()
            .take(120).map(|character| if matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { character }).collect();
        let output = folder.join(format!("{stem}_KC-upscale_{scale}x_{model}.png"));
        let temp = TemporaryOutput(folder.join(format!(".kc-upscale-{}.png", uuid::Uuid::new_v4())));
        let input_dimensions = detail::dimensions(&input).ok();
        let mut command = Command::new(&self.binary);
        command.arg("-i").arg(&input).arg("-o").arg(&temp.0).args(["-s", scale, "-m"]).arg(&self.models)
            .args(["-n", model, "-f", "png", "-c", "0"])
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        command.creation_flags(0x08000000);
        let mut child = ChildGuard(command.spawn().map_err(|error| format!("Gagal menjalankan Upscayl. {error}"))?);
        let (sender, receiver) = mpsc::sync_channel(16);
        drain_pipe(child.0.stdout.take().ok_or("Output Upscayl tidak tersedia")?, sender.clone());
        drain_pipe(child.0.stderr.take().ok_or("Output Upscayl tidak tersedia")?, sender);
        let mut percent = 2;
        let start = Instant::now();
        let mut last_activity = start;
        emit(percent, "Menyiapkan engine Upscayl…");
        let exit = loop {
            if let Some(exit) = child.0.try_wait().map_err(|error| error.to_string())? {
                break exit;
            }
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(next) => {
                    last_activity = Instant::now();
                    if let Some(next) = next {
                        percent = percent.max(next);
                        emit(percent, "Upscaling AI…");
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(200)),
            }
            if start.elapsed() >= Duration::from_secs(20 * 60) {
                return Err("Upscale melewati batas 20 menit dan dihentikan otomatis.".into());
            }
            if last_activity.elapsed() >= Duration::from_secs(90) {
                let writing = std::fs::metadata(&temp.0).ok().and_then(|info| info.modified().ok())
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age < Duration::from_secs(90));
                if !writing {
                    return Err("Upscale berhenti merespons lebih dari 90 detik dan dihentikan otomatis.".into());
                }
            }
        };
        if !exit.success() {
            return Err(format!("Upscale gagal. Periksa GPU Vulkan dan model Upscayl ({exit})."));
        }
        emit(96, "Memvalidasi resolusi dan merapikan detail…");
        let (width, height) = png_dimensions(&temp.0)?;
        if let Some((w, h)) = input_dimensions {
            let factor = scale.parse::<f64>().expect("validated scale");
            let minimum = |size: u32| ((f64::from(size) * factor - 4.0).max(f64::from(size) + 1.0)) * 0.92;
            if f64::from(width) < minimum(w) || f64::from(height) < minimum(h) {
                return Err("Resolusi hasil Upscayl tidak sesuai skala yang dipilih".into());
            }
        }
        let detailed = TemporaryOutput(folder.join(format!(".kc-upscale-detail-{}.png", uuid::Uuid::new_v4())));
        detail::apply(&temp.0, &detailed.0, key)
            .map_err(|error| format!("Upscale selesai tetapi detail pass gagal: {error}"))?;
        let size = std::fs::metadata(&detailed.0).map_err(|error| error.to_string())?.len();
        std::fs::rename(&detailed.0, &output).map_err(|error| error.to_string())?;
        emit(100, "Upscale selesai");
        Ok(json!({"ok":true,"output":output,"model":key,"engineModel":model,"scale":scale,"size":size,
            "width":width,"height":height,"inputWidth":input_dimensions.map(|size| size.0),
            "inputHeight":input_dimensions.map(|size| size.1),"backend":"GPU · Vulkan / Upscayl",
            "detailPass":"enabled"}))
    }
}

fn png_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let reader = png::Decoder::new(std::io::BufReader::new(file)).read_info()
        .map_err(|_| "Hasil upscale bukan PNG yang valid")?;
    Ok(reader.info().size())
}

fn progress(text: &str) -> Option<u8> {
    text.split_inclusive('%').filter_map(|part| {
        let value = part.strip_suffix('%')?.rsplit(|character: char| !character.is_ascii_digit() && character != '.').next()?;
        let percent = value.parse::<f64>().ok()?;
        Some(percent.round().clamp(0.0, 94.0) as u8)
    }).last()
}

fn drain_pipe(mut pipe: impl Read + Send + 'static, sender: mpsc::SyncSender<Option<u8>>) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(count) = pipe.read(&mut buffer) {
            if count == 0 || sender.send(progress(&String::from_utf8_lossy(&buffer[..count]))).is_err() {
                break;
            }
        }
    });
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

struct TemporaryOutput(PathBuf);

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_progress_until_output_is_validated() {
        assert_eq!(progress("GPU ready"), None);
        assert_eq!(progress("3.4%\r27.8%"), Some(28));
        assert_eq!(progress("100.00%\n"), Some(94));
        assert_eq!(model_names("ultrasharp").1, &["ultrasharp-4x", "ultrasharp"]);
    }
}
