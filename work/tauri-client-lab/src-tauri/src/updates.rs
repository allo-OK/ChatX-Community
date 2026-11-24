use axum::{
    body::Body,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use tokio::fs;
use tokio_util::io::ReaderStream;

#[path = "update_publish.rs"]
mod publish;
pub(crate) use publish::{checked_installer, verify_manifest, MAX_INSTALLER};
pub(crate) use publish::verify_rust_manifest;
pub use publish::{router, Identity, UpdatePermit};

const INSTALLER: &str = "updates/ChatX-CLIENT-Update.exe";
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Channel { Electron, Rust }

impl Channel {
    fn root(self) -> &'static std::path::Path {
        std::path::Path::new(match self { Self::Electron => "updates", Self::Rust => "updates/rust" })
    }
    fn metadata(self) -> std::path::PathBuf { self.root().join("client-update.json") }
    fn download_url(self) -> &'static str {
        match self { Self::Electron => "/client-update/download", Self::Rust => "/rust-client-update/download" }
    }
}

fn installer(channel: Channel, meta: &Value) -> Option<std::path::PathBuf> {
    match meta.get("installer") {
        None if channel == Channel::Electron => Some(INSTALLER.into()),
        None => None,
        Some(Value::String(name)) => {
            let id = name.strip_prefix("payload-")?.strip_suffix(".exe")?;
            uuid::Uuid::parse_str(id).ok()?;
            Some(channel.root().join(name))
        }
        _ => None,
    }
}

pub async fn status() -> Json<Value> {
    status_for(Channel::Electron).await
}

pub async fn rust_status() -> Json<Value> {
    status_for(Channel::Rust).await
}

async fn status_for(channel: Channel) -> Json<Value> {
    let unavailable = || Json(json!({"ok":true,"available":false}));
    let Ok(bytes) = fs::read(channel.metadata()).await else {
        return unavailable();
    };
    let Ok(meta) = serde_json::from_slice::<Value>(&bytes) else {
        return unavailable();
    };
    let Some(path) = installer(channel, &meta) else {
        return unavailable();
    };
    let Ok(file) = fs::metadata(path).await else {
        return unavailable();
    };
    if !meta.is_object() || !file.is_file() {
        return unavailable();
    }
    let notes = meta["notes"].as_str().unwrap_or("");
    let summary: String = notes.trim().chars().take(400).collect();
    let details: String = meta["changelog"]
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .take(16000)
        .collect();
    let changelog = if !details.is_empty() {
        details
    } else if !summary.is_empty() {
        format!("• {summary}")
    } else {
        "• HOST belum menyertakan rincian perubahan.".into()
    };
    Json(json!({
        "ok":true, "available":true, "version":meta["version"].as_str().unwrap_or(""),
        "originalName":meta["originalName"].as_str().unwrap_or(""),
        "required":meta["manifest"]["required"] == true,
        "notes":if notes.is_empty() { "Versi terbaru tersedia." } else { notes },
        "highlights":[if summary.is_empty() { "Versi terbaru tersedia." } else { &summary }],
        "changelog":changelog, "publishedAt":meta["publishedAt"].as_u64().unwrap_or(0),
        "manifest":meta["manifest"], "size":file.len(), "downloadUrl":channel.download_url()
    }))
}

pub async fn download() -> Response {
    download_for(Channel::Electron).await
}

pub async fn rust_download() -> Response {
    download_for(Channel::Rust).await
}

async fn download_for(channel: Channel) -> Response {
    let path = match fs::read(channel.metadata())
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .and_then(|meta| installer(channel, &meta))
    {
        Some(path) => path,
        None => return (StatusCode::NOT_FOUND, "Update tidak ditemukan").into_response(),
    };
    let Ok(file) = fs::File::open(path).await else {
        return (StatusCode::NOT_FOUND, "Update tidak ditemukan").into_response();
    };
    let Ok(metadata) = file.metadata().await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if !metadata.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                match channel {
                    Channel::Electron => "attachment; filename=\"ChatX CLIENT Setup.exe\"",
                    Channel::Rust => "attachment; filename=\"ChatX CLIENT Rust Setup.exe\"",
                }.into(),
            ),
            (header::CONTENT_LENGTH, metadata.len().to_string()),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response()
}
