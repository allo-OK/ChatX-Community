use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::Value;
use std::io::SeekFrom;

const CHUNK: usize = 512 * 1024;
const TTL: Duration = Duration::from_secs(120);
const ACK_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct Transfer {
    requester: SocketRef,
    source: Option<SocketRef>,
    file: Value,
    rooms: Value,
    touched: Instant,
    offset: Option<u64>,
    busy: bool,
}

#[derive(Default)]
pub(super) struct Transfers {
    previews: Mutex<HashMap<String, Transfer>>,
    downloads: Mutex<HashMap<String, Transfer>>,
}

impl Transfers {
    pub(super) fn pending_counts(&self) -> (usize, usize) {
        let mut previews = self.previews.lock().expect("gallery previews");
        prune(&mut previews);
        let preview_count = previews.len();
        drop(previews);
        let mut downloads = self.downloads.lock().expect("gallery downloads");
        prune(&mut downloads);
        (preview_count, downloads.len())
    }
}

pub(super) struct Service {
    pub(super) transfers: Arc<Transfers>,
    pub(super) gallery: Result<Arc<crate::gallery::Gallery>, String>,
    pub(super) devices: Arc<Mutex<DeviceRegistry>>,
    pub(super) conversations: Arc<ConversationDb>,
    pub(super) history: PathBuf,
    pub(super) io: SocketIo,
}

fn error(message: impl AsRef<str>) -> Value {
    json!({"ok":false,"error":message.as_ref()})
}

fn prune(map: &mut HashMap<String, Transfer>) {
    map.retain(|_, transfer| {
        transfer.touched.elapsed() < TTL
            && transfer.requester.connected()
            && transfer
                .source
                .as_ref()
                .is_none_or(|source| source.connected())
    });
}

impl Service {
    pub(super) fn disconnected(&self, socket: &SocketRef) {
        let involved = |transfer: &Transfer| {
            transfer.requester.id == socket.id || transfer.source.as_ref().is_some_and(|source| source.id == socket.id)
        };
        let mut previews = self.transfers.previews.lock().expect("gallery previews");
        previews.retain(|_, transfer| !involved(transfer));
        drop(previews);
        let mut downloads = self.transfers.downloads.lock().expect("gallery downloads");
        downloads.retain(|id, transfer| {
            if involved(transfer) {
                self.notify_cancel(id, transfer, "disconnect", "PC sumber atau tujuan offline");
                false
            } else {
                true
            }
        });
    }

    pub(super) fn register(self: Arc<Self>, socket: &SocketRef) {
        for event in [
            "file-gallery-preview-request",
            "file-gallery-download-request",
            "file-gallery-download-ready",
            "file-gallery-download-chunk",
        ] {
            let service = self.clone();
            socket.on(
                event,
                async move |socket: SocketRef, Data(raw): Data<Value>, ack: AckSender| match event {
                    "file-gallery-preview-request" => service.request(socket, raw, ack, true).await,
                    "file-gallery-download-request" => {
                        service.request(socket, raw, ack, false).await
                    }
                    "file-gallery-download-ready" => service.ready(socket, raw, ack).await,
                    _ => {
                        let result = service.forward(socket, raw).await;
                        let _ = ack.send(&result.unwrap_or_else(error));
                    }
                },
            );
        }
        let service = self.clone();
        socket.on(
            "file-gallery-preview-response",
            async move |socket: SocketRef, Data(raw): Data<Value>| {
                service.preview_response(socket, raw);
            },
        );
        socket.on(
            "file-gallery-transfer-cancel",
            async move |socket: SocketRef, Data(raw): Data<Value>| {
                self.cancel(&socket, raw["requestId"].as_str().unwrap_or_default());
            },
        );
    }

    fn can_download(&self, socket: &SocketRef) -> bool {
        let Some(device) = gallery_device(socket, &self.devices) else {
            return false;
        };
        let Some(session) = socket.extensions.get::<SocketSession>() else {
            return false;
        };
        socket
            .extensions
            .get::<Arc<crate::cohost::Access>>()
            .is_some_and(|access| {
                access.policy(&device.name, &device.id, session.is_host)["permissions"]
                    ["downloadFiles"]
                    != false
            })
    }

    fn allowed(&self, transfer: &Transfer) -> bool {
        self.can_download(&transfer.requester)
            && transfer.requester.connected()
            && (transfer.source.is_some()
                || gallery_rooms(&transfer.requester, &self.devices, &self.conversations)
                    == transfer.rooms)
    }

    async fn request(&self, socket: SocketRef, raw: Value, ack: AckSender, preview: bool) {
        let result = self.lookup(socket, raw, preview).await;
        let (id, transfer) = match result {
            Ok(result) => result,
            Err(message) => {
                let _ = ack.send(&error(message));
                return;
            }
        };
        if !preview {
            let _ = ack.send(&json!({"ok":true,"requestId":id,"file":transfer.file}));
            return;
        }
        let _ = ack.send(&json!({"ok":true,"requestId":id}));
        if let Some(source) = transfer.source {
            if source.emit("file-gallery-preview-source",&json!({"requestId":id,"localId":transfer.file["localId"],"fileId":transfer.file["fileId"]})).is_err() {
                self.transfers.previews.lock().expect("gallery previews").remove(&id);
                let _ = transfer.requester.emit("file-gallery-preview-result",&json!({"requestId":id,"ok":false,"error":"PC sumber sedang offline"}));
            }
        } else {
            let url = transfer.file["previewUrl"].as_str().unwrap_or_default();
            let _ = transfer.requester.emit("file-gallery-preview-result",&json!({"requestId":id,
                "ok":!url.is_empty(),"dataUrl":url,"error":if url.is_empty() {"Preview chat hanya tersedia untuk gambar"} else {""}}));
        }
    }

    async fn lookup(
        &self,
        socket: SocketRef,
        raw: Value,
        preview: bool,
    ) -> Result<(String, Transfer), String> {
        if gallery_device(&socket, &self.devices).is_none()
            || (!preview && !self.can_download(&socket))
        {
            return Err("Akses fitur dibatasi oleh HOST.".into());
        }
        let id = raw["fileId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 100)
            .ok_or("File tidak ada di index")?
            .to_owned();
        let rooms = gallery_rooms(&socket, &self.devices, &self.conversations);
        let query_rooms = rooms.clone();
        let gallery = self.gallery.clone()?;
        let history = self.history.clone();
        let mut file =
            tokio::task::spawn_blocking(move || gallery.find(&id, &history, &query_rooms))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?
                .ok_or("File tidak ada di index")?;
        if gallery_device(&socket, &self.devices).is_none()
            || (!preview && !self.can_download(&socket))
            || (file["sourceType"] == "chat"
                && rooms != gallery_rooms(&socket, &self.devices, &self.conversations))
        {
            return Err("Akses fitur dibatasi oleh HOST.".into());
        }
        let source = if file["sourceType"] == "chat" {
            None
        } else {
            Some(
                self.io
                    .sockets()
                    .into_iter()
                    .find(|peer| {
                        gallery_source(peer, &self.devices)
                            .is_some_and(|device| file["ownerDeviceId"] == device.id)
                    })
                    .ok_or("PC sumber sedang offline")?,
            )
        };
        file["sourceOnline"] = json!(true);
        let request_id = uuid::Uuid::new_v4().to_string();
        let transfer = Transfer {
            requester: socket,
            source,
            file,
            rooms,
            touched: Instant::now(),
            offset: None,
            busy: false,
        };
        if !preview || transfer.source.is_some() {
            let mut map = if preview {
                &self.transfers.previews
            } else {
                &self.transfers.downloads
            }
            .lock()
            .expect("gallery transfers");
            prune(&mut map);
            if map.len() >= if preview { 80 } else { 24 } {
                return Err("Antrean Gallery penuh, coba lagi sebentar.".into());
            }
            map.insert(request_id.clone(), transfer.clone());
        }
        Ok((request_id, transfer))
    }

    fn preview_response(&self, socket: SocketRef, raw: Value) {
        let id = raw["requestId"].as_str().unwrap_or_default();
        let transfer = {
            let mut map = self.transfers.previews.lock().expect("gallery previews");
            prune(&mut map);
            if !map
                .get(id)
                .and_then(|transfer| transfer.source.as_ref())
                .is_some_and(|source| source.id == socket.id)
            {
                return;
            }
            map.remove(id)
        };
        let Some(transfer) = transfer else {
            return;
        };
        if gallery_source(&socket, &self.devices).is_none()
            || gallery_device(&transfer.requester, &self.devices).is_none()
        {
            return;
        }
        let url = raw["dataUrl"].as_str().unwrap_or_default();
        let valid = url.starts_with("data:image/") && url.len() <= 8 * 1024 * 1024;
        let _ = transfer.requester.emit(
            "file-gallery-preview-result",
            &json!({"requestId":id,
            "ok":raw["ok"]!=false && valid,"dataUrl":if valid {url}else{""},
            "error":raw["error"].as_str().unwrap_or(if valid {""}else{"Preview tidak tersedia"})}),
        );
    }

    async fn ready(&self, socket: SocketRef, raw: Value, ack: AckSender) {
        let id = raw["requestId"].as_str().unwrap_or_default();
        let transfer = (|| {
            let mut map = self.transfers.downloads.lock().expect("gallery downloads");
            prune(&mut map);
            let transfer = map
                .get_mut(id)
                .filter(|transfer| transfer.requester.id == socket.id && transfer.offset.is_none())
                .ok_or("Transfer tidak valid")?;
            if !self.allowed(transfer) {
                return Err("Unduh file dibatasi HOST");
            }
            let offset = raw["offset"]
                .as_u64()
                .filter(|offset| *offset <= transfer.file["size"].as_u64().unwrap_or(0))
                .ok_or("Offset resume tidak valid")?;
            transfer.offset = Some(offset);
            transfer.touched = Instant::now();
            Ok(transfer.clone())
        })();
        let transfer = match transfer {
            Ok(transfer) => transfer,
            Err(message) => {
                let _ = ack.send(&error(message));
                return;
            }
        };
        let _ = ack.send(&json!({"ok":true}));
        let offset = transfer.offset.expect("ready offset");
        if let Some(source) = &transfer.source {
            let requester = gallery_device(&socket, &self.devices)
                .map(|device| device.name)
                .unwrap_or_default();
            if source
                .emit(
                    "file-gallery-download-source",
                    &json!({"requestId":id,"localId":transfer.file["localId"],
                "file":transfer.file,"offset":offset,"requester":requester}),
                )
                .is_err()
            {
                self.fail(id, &transfer, "PC sumber offline");
            }
        } else {
            match self.stream(id, &transfer, offset).await {
                Ok(()) => {
                    self.transfers
                        .downloads
                        .lock()
                        .expect("gallery downloads")
                        .remove(id);
                }
                Err(message) => self.fail(id, &transfer, &message),
            }
        }
    }

    async fn stream(&self, id: &str, transfer: &Transfer, mut offset: u64) -> Result<(), String> {
        let stored = transfer.file["chatStored"]
            .as_str()
            .ok_or("File chat tidak tersedia")?;
        let upload_dir = super::storage_location::upload_dir(Path::new(".")).map_err(|error| error.to_string())?;
        let mut file = tokio::fs::File::open(upload_dir.join(stored))
            .await
            .map_err(|error| error.to_string())?;
        let total = file
            .metadata()
            .await
            .map_err(|error| error.to_string())?
            .len();
        if transfer.file["size"].as_u64() != Some(total) {
            return Err("File sumber berubah".into());
        }
        let mut digest = Sha256::new();
        let mut buffer = vec![0; CHUNK];
        loop {
            self.check_live(id, transfer)?;
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        let sha256 = format!("{:x}", digest.finalize());
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|error| error.to_string())?;
        loop {
            self.check_live(id, transfer)?;
            let len = (total - offset).min(CHUNK as u64) as usize;
            file.read_exact(&mut buffer[..len])
                .await
                .map_err(|error| error.to_string())?;
            let eof = offset + len as u64 == total;
            send_chunk(
                &transfer.requester,
                &json!({"requestId":id,"offset":offset,"total":total,
                "base64":STANDARD.encode(&buffer[..len]),"eof":eof,"sha256":sha256}),
            )
            .await?;
            if eof {
                return Ok(());
            }
            offset += len as u64;
        }
    }

    fn check_live(&self, id: &str, transfer: &Transfer) -> Result<(), String> {
        let mut map = self.transfers.downloads.lock().expect("gallery downloads");
        let entry = map.get_mut(id).ok_or("Transfer dibatalkan")?;
        if !self.allowed(transfer) {
            return Err("Unduh file dibatasi HOST".into());
        }
        entry.touched = Instant::now();
        Ok(())
    }

    async fn forward(&self, socket: SocketRef, raw: Value) -> Result<Value, String> {
        let id = raw["requestId"].as_str().ok_or("Transfer tidak valid")?;
        if gallery_source(&socket, &self.devices).is_none() {
            return Err("Transfer tidak valid".into());
        }
        let data = raw["base64"]
            .as_str()
            .filter(|data| data.len() <= 4 * CHUNK.div_ceil(3))
            .ok_or("Chunk tidak valid")?;
        let len = STANDARD
            .decode(data)
            .map_err(|_| "Chunk tidak valid")?
            .len() as u64;
        let total = raw["total"].as_u64().ok_or("Ukuran file tidak valid")?;
        let sha = raw["sha256"]
            .as_str()
            .filter(|sha| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("Checksum tidak valid")?;
        let offset = raw["offset"].as_u64().ok_or("Offset tidak valid")?;
        let next = offset
            .checked_add(len)
            .filter(|next| *next <= total)
            .ok_or("Offset tidak valid")?;
        let eof = raw["eof"] == true;
        if len > CHUNK as u64 || eof != (next == total) || (len == 0 && !eof) {
            return Err("Chunk tidak valid".into());
        }
        let transfer = {
            let mut map = self.transfers.downloads.lock().expect("gallery downloads");
            prune(&mut map);
            let entry = map
                .get_mut(id)
                .filter(|entry| {
                    !entry.busy
                        && entry.offset == Some(offset)
                        && entry.file["size"].as_u64() == Some(total)
                        && entry
                            .source
                            .as_ref()
                            .is_some_and(|source| source.id == socket.id)
                })
                .ok_or("Transfer tidak valid")?;
            if !self.allowed(entry) {
                return Err("Unduh file dibatasi HOST".into());
            }
            entry.busy = true;
            entry.touched = Instant::now();
            entry.clone()
        };
        let result = send_chunk(&transfer.requester,&json!({"requestId":id,"offset":offset,"total":total,"base64":data,"eof":eof,"sha256":sha})).await;
        if let Err(message) = result {
            self.fail(id, &transfer, &message);
            return Err(message);
        }
        let mut map = self.transfers.downloads.lock().expect("gallery downloads");
        let entry = map.get_mut(id).ok_or("Transfer dibatalkan")?;
        if eof {
            map.remove(id);
        } else {
            entry.busy = false;
            entry.offset = Some(next);
            entry.touched = Instant::now();
        }
        Ok(json!({"ok":true}))
    }

    fn cancel(&self, socket: &SocketRef, id: &str) {
        let transfer = {
            let mut map = self.transfers.downloads.lock().expect("gallery downloads");
            if !map.get(id).is_some_and(|transfer| {
                transfer.requester.id == socket.id
                    || transfer
                        .source
                        .as_ref()
                        .is_some_and(|source| source.id == socket.id)
            }) {
                return;
            }
            map.remove(id)
        };
        if let Some(transfer) = transfer {
            self.notify_cancel(id, &transfer, "cancel", "");
        }
    }

    fn fail(&self, id: &str, transfer: &Transfer, message: &str) {
        if self
            .transfers
            .downloads
            .lock()
            .expect("gallery downloads")
            .remove(id)
            .is_some()
        {
            self.notify_cancel(id, transfer, "disconnect", message);
        }
    }

    fn notify_cancel(&self, id: &str, transfer: &Transfer, reason: &str, message: &str) {
        let payload = json!({"requestId":id,"reason":reason,"error":message});
        let _ = transfer
            .requester
            .emit("file-gallery-transfer-cancelled", &payload);
        if let Some(source) = &transfer.source {
            let _ = source.emit("file-gallery-transfer-cancelled", &payload);
        }
    }
}

async fn send_chunk(socket: &SocketRef, payload: &Value) -> Result<(), String> {
    let reply: Value = socket
        .timeout(ACK_TIMEOUT)
        .emit_with_ack("file-gallery-download-chunk", payload)
        .map_err(|error| error.to_string())?
        .await
        .map_err(|error| error.to_string())?;
    if reply["ok"] == true {
        Ok(())
    } else {
        Err(reply["error"]
            .as_str()
            .unwrap_or("PC tujuan menolak chunk")
            .into())
    }
}
