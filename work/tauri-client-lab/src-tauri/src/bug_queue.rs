use serde_json::{json, Value};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{State, WebviewWindow};
use tokio::sync::Notify;

const MAX_PENDING: usize = 200;
const MAX_REPORT_BYTES: usize = 128 * 1024;

pub struct BugQueue {
    path: PathBuf,
    files: Mutex<()>,
    wake: Notify,
}

pub fn read_pending(path: &Path) -> io::Result<Vec<Value>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

impl BugQueue {
    pub fn new(data: &Path) -> Self {
        Self {
            path: data.join("logs/pending-bugs.json"),
            files: Mutex::new(()),
            wake: Notify::new(),
        }
    }

    fn save(&self, rows: &[Value]) -> io::Result<()> {
        fs::create_dir_all(self.path.parent().expect("queue directory"))?;
        crate::host::atomic_write(&self.path, |file| {
            serde_json::to_writer(file, rows).map_err(io::Error::other)
        })
    }

    fn push(&self, mut report: Value) -> io::Result<()> {
        if !report.is_object() {
            return Err(io::Error::other("Data laporan tidak valid."));
        }
        report["queueId"] = json!(uuid::Uuid::new_v4().to_string());
        if serde_json::to_vec(&report).map_err(io::Error::other)?.len() > MAX_REPORT_BYTES {
            return Err(io::Error::other("Laporan terlalu besar."));
        }
        let _guard = self.files.lock().expect("bug queue");
        let mut rows = read_pending(&self.path)?;
        rows.push(report);
        if rows.len() > MAX_PENDING {
            rows.drain(..rows.len() - MAX_PENDING);
        }
        self.save(&rows)?;
        self.wake.notify_one();
        Ok(())
    }

    fn front(&self) -> io::Result<Option<Value>> {
        let _guard = self.files.lock().expect("bug queue");
        let mut rows = read_pending(&self.path)?;
        let Some(first) = rows.first_mut() else {
            return Ok(None);
        };
        if !first.is_object() {
            return Err(io::Error::other("Entri antrean laporan tidak valid."));
        }
        if first["queueId"].as_str().and_then(|id| uuid::Uuid::parse_str(id).ok()).is_none() {
            first["queueId"] = json!(uuid::Uuid::new_v4().to_string());
            self.save(&rows)?;
        }
        Ok(rows.into_iter().next())
    }

    fn acknowledge(&self, report: &Value) -> io::Result<()> {
        let _guard = self.files.lock().expect("bug queue");
        let mut rows = read_pending(&self.path)?;
        if let Some(index) = rows.iter().position(|row| row["queueId"] == report["queueId"]) {
            rows.remove(index);
            self.save(&rows)?;
        }
        Ok(())
    }

    pub async fn run(self: Arc<Self>, status: crate::HostStatus, log: Arc<crate::support_log::SupportLog>) {
        let client = match reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build() {
            Ok(client) => client,
            Err(error) => {
                let _ = log.record("runtime", "bug-queue-unavailable", &json!({"error":error.to_string()}));
                return;
            }
        };
        loop {
            let connection = {
                let status = status.lock().expect("status HOST");
                status["address"].as_str().zip(status["adminToken"].as_str())
                    .map(|(address, token)| (address.to_owned(), token.to_owned()))
            };
            let Some((address, token)) = connection else {
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            };
            let result = async {
                let queue = self.clone();
                let Some(report) = tokio::task::spawn_blocking(move || queue.front()).await?? else {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(false);
                };
                let response = client.post(format!("{address}/bug-report"))
                    .bearer_auth(token)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(report.to_string())
                    .send().await?.error_for_status()?;
                let receipt: Value = serde_json::from_slice(&response.bytes().await?)?;
                if receipt["ok"] != true || receipt["signature"].as_str().is_none() {
                    return Err(io::Error::other("HOST belum mengonfirmasi laporan.").into());
                }
                let queue = self.clone();
                tokio::task::spawn_blocking(move || queue.acknowledge(&report)).await??;
                Ok(true)
            }.await;
            match result {
                Ok(true) => {}
                Ok(false) => self.wake.notified().await,
                Err(error) => {
                    let _ = log.record("runtime", "bug-queue-retry", &json!({"error":error.to_string()}));
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        }
    }
}

#[tauri::command]
pub async fn queue_bug_report(window: WebviewWindow, queue: State<'_, Arc<BugQueue>>, report: Value) -> Result<Value, String> {
    if window.label() != "host" {
        return Err("Jendela ini tidak boleh mengirim laporan HOST.".into());
    }
    let queue = queue.inner().clone();
    tokio::task::spawn_blocking(move || {
        let mut report = report;
        if let Some(fields) = report.as_object_mut() {
            fields.retain(|key, _| matches!(key.as_str(), "type" | "source" | "severity" | "message" | "stack" | "context" | "version" | "role" | "pc" | "user" | "deviceId" | "ts"));
            if let Some(context) = fields.get_mut("context") {
                *context = crate::support_log::redact(context, 8);
            }
        }
        queue.push(report)
    })
        .await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
    Ok(json!({"ok":true,"queued":true}))
}
