use super::*;
use axum::extract::State;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub(super) struct Service {
    access: Attachments,
    gallery: Option<Arc<crate::gallery::Gallery>>,
    transfers: Arc<gallery_transfer::Transfers>,
    health: Value,
    snapshot: SnapshotGate,
    started: Instant,
    scheduler_delay_ms: Arc<AtomicU64>,
}

impl Service {
    pub(super) fn new(access: Attachments, gallery: Option<Arc<crate::gallery::Gallery>>,
        transfers: Arc<gallery_transfer::Transfers>, health: Value, snapshot: SnapshotGate) -> Self {
        Self {
            access, gallery, transfers, health, snapshot,
            started: Instant::now(),
            scheduler_delay_ms: Arc::new(AtomicU64::new(u64::MAX)),
        }
    }

    pub(super) async fn sample_scheduler(self) -> io::Result<()> {
        loop {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
            tokio::time::sleep_until(deadline).await;
            let delay = tokio::time::Instant::now().saturating_duration_since(deadline);
            self.scheduler_delay_ms.store(delay.as_millis() as u64, Ordering::Relaxed);
        }
    }

    fn roster(&self) -> (Vec<Value>, Vec<Value>) {
        let access = &self.access;
        let mut devices = access.devices.lock().expect("device registry");
        let mut live = HashMap::<String, Presence>::new();
        for socket in access.io.sockets() {
            let Some(session) = socket.extensions.get::<SocketSession>() else { continue };
            let Some(device) = devices.resolve_session(&session.token, Instant::now()).map(str::to_owned) else { continue };
            let Some(presence) = socket.extensions.get::<Presence>() else { continue };
            if live.get(&device).is_none_or(|previous| previous.connected < presence.connected) {
                live.insert(device, presence);
            }
        }
        let now = presence_time();
        let mut known = devices.records.iter().map(|(id, record)| {
            let current = live.get(id);
            let field = |name: &str| current.and_then(|presence| presence.public[name].as_str())
                .filter(|value| !value.is_empty()).map_or_else(|| record[name].clone(), |value| json!(value));
            json!({"deviceId":id,"name":field("name"),"pc":field("pc"),"version":field("version"),
                "teamRole":field("teamRole"),"isHost":record["isHost"] == true || current.is_some_and(|p| p.public["isHost"] == true),
                "online":current.is_some(),"revoked":record["revoked"] == true,
                "lastSeen":if current.is_some() {json!(now)} else {record["lastSeen"].clone()}})
        }).collect::<Vec<_>>();
        drop(devices);
        known = access.access.directory(known);
        for row in &mut known {
            row["ageMs"] = json!(now.saturating_sub(row["lastSeen"].as_u64().unwrap_or_default()));
        }
        known.sort_by(|left, right| right["lastSeen"].as_u64().cmp(&left["lastSeen"].as_u64())
            .then_with(|| left["deviceId"].as_str().cmp(&right["deviceId"].as_str())));
        let online = known.iter().filter(|row| row["online"] == true).cloned().collect();
        (online, known)
    }
}

pub(super) fn router(service: Service) -> Router {
    Router::new().route("/admin/system-status", get(status)).with_state(service)
}

async fn status(State(service): State<Service>, headers: HeaderMap, uri: Uri) -> Response {
    if !device_admin::authorized(&service.access, &headers, &uri) {
        return (StatusCode::FORBIDDEN, Json(json!({"ok":false,"error":"Hanya HOST"}))).into_response();
    }
    let path = service.access.history.clone();
    let gallery = service.gallery.clone();
    let counts = tokio::task::spawn_blocking(move || {
        let chat = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|db| db.query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get::<_, i64>(0)));
        let gallery_rows = gallery.and_then(|gallery| gallery.row_count().ok());
        (chat, gallery_rows)
    }).await;
    let (chat, gallery_rows) = match counts {
        Ok((chat, gallery_rows)) => (chat.ok(), gallery_rows),
        Err(_) => (None, None),
    };
    let (online, known) = service.roster();
    let (previews, downloads) = service.transfers.pending_counts();
    let mut health = service.health.clone();
    health["maintenance"] = json!(service.snapshot.try_read().is_err() || storage::pending());
    health["uptimeSec"] = json!(service.started.elapsed().as_secs());
    let delay = service.scheduler_delay_ms.load(Ordering::Relaxed);
    health["eventLoopDelayMs"] = json!((delay != u64::MAX).then_some(delay));
    let memory = process_memory();
    health["rssBytes"] = json!(memory.map(|(rss, _)| rss));
    health["privateBytes"] = json!(memory.map(|(_, private)| private));
    health["chatDb"] = json!(chat.is_some() && health["database"] == true);
    health["chatMessages"] = json!(chat);
    health["galleryRows"] = json!(gallery_rows);
    health["pendingGalleryPreviews"] = json!(previews);
    health["pendingGalleryDownloads"] = json!(downloads);
    health["onlineClients"] = json!(online.len());
    if chat.is_none() {
        health["databaseWarning"] = json!("Statistik database chat tidak dapat dibaca.");
    }
    Json(json!({"ok":true,"serverVersion":service.health["version"],"online":online,"known":known,"health":health})).into_response()
}

pub(crate) fn process_memory() -> Option<(usize, usize)> {
    use windows_sys::Win32::System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::GetCurrentProcess,
    };
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX { cb: size, ..Default::default() };
    // The EX structure begins with the fields of PROCESS_MEMORY_COUNTERS.
    // Passing its full size asks Windows to populate PrivateUsage as well.
    let success = unsafe {
        K32GetProcessMemoryInfo(GetCurrentProcess(), std::ptr::from_mut(&mut counters).cast(), size)
    };
    (success != 0).then_some((counters.WorkingSetSize, counters.PrivateUsage))
}
