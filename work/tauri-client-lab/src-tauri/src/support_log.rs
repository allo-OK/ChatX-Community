use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, WebviewWindow};

const ROTATE_BYTES: u64 = 2 * 1024 * 1024;
const TAIL_BYTES: u64 = 350_000;

pub struct SupportLog {
    data: PathBuf,
    since: Instant,
    files: Mutex<()>,
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn redact(value: &Value, depth: usize) -> Value {
    if depth == 0 {
        return Value::Null;
    }
    match value {
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .filter_map(|(key, value)| {
                    let normalized: String = key
                        .to_ascii_lowercase()
                        .chars()
                        .filter(char::is_ascii_alphanumeric)
                        .collect();
                    if [
                        "token",
                        "secret",
                        "authorization",
                        "apikey",
                        "password",
                        "cookie",
                        "base64",
                        "chatcontent",
                        "messagebody",
                        "filecontent",
                        "pairingcode",
                    ]
                    .iter()
                    .any(|s| normalized.contains(s))
                        || matches!(normalized.as_str(), "text" | "body" | "clipboard")
                    {
                        return None;
                    }
                    Some((key.chars().take(80).collect(), redact(value, depth - 1)))
                })
                .take(30)
                .collect(),
        ),
        Value::Array(entries) => Value::Array(
            entries
                .iter()
                .take(12)
                .map(|value| redact(value, depth - 1))
                .collect(),
        ),
        Value::String(value) => json!(value.chars().take(600).collect::<String>()),
        _ => value.clone(),
    }
}

impl SupportLog {
    pub fn new(data: PathBuf) -> Self {
        Self {
            data,
            since: Instant::now(),
            files: Mutex::new(()),
        }
    }

    fn directory(&self) -> PathBuf {
        self.data.join("logs")
    }

    pub fn record(&self, source: &str, kind: &str, detail: &Value) -> io::Result<()> {
        let _guard = self.files.lock().expect("support log");
        let dir = self.directory();
        fs::create_dir_all(&dir)?;
        let path = dir.join("timeline.jsonl");
        let entry = json!({"ts":epoch_ms(),"source":source,"type":kind.chars().take(120).collect::<String>(),"detail":redact(detail,3)});
        let mut line = serde_json::to_vec(&entry)?;
        if line.len() > 16 * 1024 {
            line = serde_json::to_vec(
                &json!({"ts":entry["ts"],"source":source,"type":entry["type"],"detail":{"truncated":true}}),
            )?;
        }
        line.push(b'\n');
        if fs::metadata(&path).is_ok_and(|meta| meta.len() + line.len() as u64 > ROTATE_BYTES) {
            let previous = dir.join("timeline.jsonl.previous");
            match fs::remove_file(&previous) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::rename(&path, &previous)?;
        }
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)?
            .write_all(&line)
    }

    fn export(&self, path: &Path, details: Value) -> io::Result<()> {
        let _guard = self.files.lock().expect("support log");
        let dir = self.directory();
        let mut data = details;
        for (key, file) in [
            ("timelineLog", "timeline.jsonl"),
            ("previousTimelineLog", "timeline.jsonl.previous"),
            ("stdioErrors", "stdio-errors.jsonl"),
            ("runtimeLog", "runtime.log"),
            ("rendererLog", "renderer.log"),
        ] {
            data[key] = json!(tail(&dir.join(file))?);
        }
        data["applicationDiagnostics"]["uptimeSeconds"] = json!(self.since.elapsed().as_secs());
        data["applicationDiagnostics"]["pendingBugReports"] =
            match crate::bug_queue::read_pending(&dir.join("pending-bugs.json")) {
                Ok(rows) => json!(rows.len()),
                Err(error) => json!({"error":error.to_string()}),
            };
        data["hostDiagnostics"] = match fs::read(self.data.join("diagnostics.json")) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Value::Null,
            Err(error) => return Err(error),
        };
        let mut recent = std::collections::BTreeMap::<String, usize>::new();
        for event in data["timelineLog"].as_str().unwrap_or_default().lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
            if let Some(kind) = event["type"].as_str() {
                if kind.contains("failed") || kind.contains("error") {
                    *recent.entry(kind.to_owned()).or_default() += 1;
                }
            }
        }
        data["applicationDiagnostics"]["recentErrorsByType"] = json!(recent);
        let version = data["version"].as_str().unwrap_or_default();
        let mut reports_by_version = std::collections::BTreeMap::<String, u64>::new();
        for group in data["hostDiagnostics"]["groups"].as_object().into_iter().flatten().map(|(_, group)| group) {
            let reported_version = group["version"].as_str().unwrap_or("unknown");
            *reports_by_version.entry(reported_version.to_owned()).or_default() += group["count"].as_u64().unwrap_or(0);
        }
        data["applicationDiagnostics"]["hostReportCounts"] = json!({
            "currentVersion": version, "byVersion": reports_by_version,
        });
        crate::host::atomic_write(path, |file| {
            serde_json::to_writer_pretty(file, &data).map_err(io::Error::other)
        })
    }
}

fn tail(path: &Path) -> io::Result<String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error),
    };
    let start = file.metadata()?.len().saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[tauri::command]
pub async fn support_event(
    window: WebviewWindow,
    log: tauri::State<'_, std::sync::Arc<SupportLog>>,
    input: Value,
) -> Result<(), String> {
    let source = match window.label() {
        "host" => "renderer",
        _ => return Err("Sumber log tidak valid.".into()),
    };
    let kind = input["type"]
        .as_str()
        .unwrap_or("renderer-event")
        .to_owned();
    let detail = input
        .get("detail")
        .or_else(|| input.get("context"))
        .cloned()
        .unwrap_or(Value::Null);
    let log = log.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        log.record(source, &kind, &detail)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_support_folder(
    window: WebviewWindow,
    log: tauri::State<'_, std::sync::Arc<SupportLog>>,
) -> Result<Value, String> {
    if window.label() != "host" {
        return Err("Buka folder log dari HOST.".into());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    let path = log.directory();
    tauri::async_runtime::spawn_blocking(move || {
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        crate::image_files::open_path(owner, &path)?;
        Ok(json!({"ok":true,"path":path}))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn export_support_log(
    window: WebviewWindow,
    log: tauri::State<'_, std::sync::Arc<SupportLog>>,
    status: tauri::State<'_, crate::HostStatus>,
    config: Value,
    exported_at: String,
) -> Result<Value, String> {
    if window.label() != "host" {
        return Err("Ekspor log dari HOST.".into());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    let app = window.app_handle();
    let windows: Vec<_> = app
        .webview_windows()
        .into_iter()
        .map(|(id, w)| {
            json!({"id":id,"visible":w.is_visible().unwrap_or(false),
        "minimized":w.is_minimized().unwrap_or(false),"destroyed":false})
        })
        .collect();
    let host = status.lock().expect("status HOST").clone();
    let log = log.inner().clone();
    let memory = crate::host::system_status::process_memory();
    let details = json!({"exportedAt":exported_at.chars().take(40).collect::<String>(),"version":"1.1.43","platform":"win32",
        "arch":std::env::consts::ARCH,"isPackaged":!cfg!(debug_assertions),"role":"host","roleSource":"Rust",
        "exe":std::env::current_exe().ok(),"appName":"ChatX HOST","userData":log.data,
        "serverAddress":host["address"],"serverDataDir":host["dataPath"],"config":redact(&config,3),
        "applicationDiagnostics":{"windows":windows,"processes":[{"type":"host","pid":std::process::id(),
            "rssBytes":memory.map(|(rss, _)| rss),"privateBytes":memory.map(|(_, private)| private)}]}});
    tauri::async_runtime::spawn_blocking(move || {
        let name = format!("ChatX-Support-Log-v1.1.43-{}.json", epoch_ms());
        let path = if std::env::args_os()
            .nth(1)
            .is_some_and(|arg| arg == "--smoke-gui")
        {
            log.data.join("support-export-smoke.json")
        } else {
            let Some(path) = crate::image_files::choose_save_path(
                owner,
                &name,
                "JSON Log\0*.json\0\0",
                "json",
                "Export ChatX Support Log",
            )?
            else {
                return Ok(json!({"ok":false,"canceled":true}));
            };
            path
        };
        log.export(&path, details)
            .map_err(|error| error.to_string())?;
        let _ = log.record("runtime", "support-log-exported", &json!({}));
        Ok(json!({"ok":true,"path":path}))
    })
    .await
    .map_err(|error| error.to_string())?
}
