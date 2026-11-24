use axum::{body::Body, extract::{Path as RoutePath, Request}, http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri}, middleware::{self, Next}, response::{IntoResponse, Response}, routing::{get, post}, Json, Router};
use serde_json::json;
use sha2::{Digest, Sha256};
use socketioxide::{extract::{AckSender, Data, SocketRef}, SocketIo};
use std::{collections::HashMap, io::{self, Write}, net::Ipv4Addr, path::{Path, PathBuf}, sync::{Arc, Mutex, RwLock}, time::{Duration, Instant}};
use tokio::net::{TcpListener, UdpSocket};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[path = "gallery_transfer.rs"]
mod gallery_transfer;
#[path = "storage.rs"]
mod storage;
#[path = "storage_location.rs"]
pub(crate) mod storage_location;
#[path = "host_compression.rs"]
pub(crate) mod host_compression;

pub async fn serve() -> io::Result<()> {
    serve_with_ready(|port, discovery_port| {
        println!("{}", json!({"port":port,"discoveryPort":discovery_port}));
        io::stdout().flush()
    }).await
}

pub async fn serve_with_ready(ready: impl FnOnce(u16, u16) -> io::Result<()>) -> io::Result<()> {
    let admin = std::env::var("KC_TEST_ADMIN_TOKEN").ok().filter(|token| valid_admin_token(token));
    serve_listener(|port, discovery_port, _| ready(port, discovery_port),
        None, admin, SnapshotGate::default(), Arc::default(), None).await
}

pub type SnapshotGate = Arc<tokio::sync::RwLock<()>>;

#[path = "statuses.rs"]
mod statuses;
#[path = "device_admin.rs"]
mod device_admin;
#[path = "system_status.rs"]
pub(crate) mod system_status;

macro_rules! snapshot_write {
    ($socket:expr,$ack:ident) => {{
        match $socket.extensions.get::<SnapshotGate>().and_then(|gate|gate.try_read_owned().ok()) {
            Some(guard)=>{
                if storage::pending() {
                    let _=$ack.send(&json!({"ok":false,"retryable":true,"error":"Pemulihan storage belum selesai. Coba ulang operasi storage atau restart HOST."}));
                    return;
                }
                guard
            },
            None=>{
                let _=$ack.send(&json!({"ok":false,"retryable":true,"error":"HOST sedang membuat backup. Coba lagi sebentar."}));
                return;
            }
        }
    }};
}

pub async fn serve_with_provider(
    ready: impl FnOnce(u16, u16, Option<std::net::SocketAddr>) -> io::Result<()>,
    admin: String,
    snapshot: SnapshotGate,
    update_permit: Arc<crate::updates::UpdatePermit>,
    lan: Option<LanBinding>,
    reserved: Option<HostListeners>,
) -> io::Result<()> {
    serve_listener(ready, lan, Some(admin), snapshot, update_permit, reserved).await
}

#[derive(Clone, Copy)]
pub struct LanBinding {
    pub address: Ipv4Addr,
    pub ephemeral: bool,
}

pub async fn serve_lan_lab(address: Ipv4Addr) -> io::Result<()> {
    if !address.is_private() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Alamat HOST lab harus IPv4 LAN privat"));
    }
    serve_listener(|_, discovery_port, lan| {
        let port = lan.expect("LAN listener").port();
        println!("{}", json!({"ip":address,"port":port,"discoveryPort":discovery_port}));
        io::stdout().flush()
    }, Some(LanBinding { address, ephemeral: false }), None, SnapshotGate::default(), Arc::default(), None).await
}

pub(crate) struct HostListeners {
    lan_tcp: Option<TcpListener>,
    local_tcp: Option<TcpListener>,
    udp: UdpSocket,
    discovery_reply: Option<UdpSocket>,
    address: Ipv4Addr,
}

impl HostListeners {
    pub(crate) async fn bind(lan: Option<LanBinding>, with_local_control: bool) -> io::Result<Self> {
        let address = lan.map_or(Ipv4Addr::LOCALHOST, |binding| binding.address);
        if lan.is_some() && !address.is_private() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Alamat LAN harus IPv4 privat"));
        }
        let (lan_tcp, discovery_addr, discovery_port) = match lan {
            Some(binding) => {
                let (port, discovery_port) = if binding.ephemeral { (0, 0) } else { (45820, 45821) };
                (Some(TcpListener::bind((address, port)).await?), Ipv4Addr::UNSPECIFIED, discovery_port)
            }
            None => (None, Ipv4Addr::LOCALHOST, 0),
        };
        let local_tcp = if lan.is_none() || with_local_control {
            let local_port = if lan.is_some_and(|binding| !binding.ephemeral) { 45820 } else { 0 };
            Some(TcpListener::bind((Ipv4Addr::LOCALHOST, local_port)).await?)
        } else { None };
        let udp = UdpSocket::bind((discovery_addr, discovery_port)).await?;
        let discovery_reply = if lan.is_some() {
            Some(UdpSocket::bind((address, 0)).await?)
        } else {
            None
        };
        Ok(Self {lan_tcp, local_tcp, udp, discovery_reply, address})
    }
}

async fn serve_listener(
    ready: impl FnOnce(u16, u16, Option<std::net::SocketAddr>) -> io::Result<()>,
    lan: Option<LanBinding>,
    admin: Option<String>,
    snapshot: SnapshotGate,
    update_permit: Arc<crate::updates::UpdatePermit>,
    reserved: Option<HostListeners>,
) -> io::Result<()> {
    let HostListeners {lan_tcp, local_tcp, udp, discovery_reply, address} =
        match reserved {
            Some(listeners) => listeners,
            None => HostListeners::bind(lan, admin.is_some()).await?,
        };
    let lan_address = lan_tcp.as_ref().map(TcpListener::local_addr).transpose()?;
    let local_port = local_tcp.as_ref().map(TcpListener::local_addr).transpose()?.map_or(0, |address| address.port());
    let port = lan_address.map_or(local_port, |address| address.port());
    let pc = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "ChatX".into());
    let strict_auth = lan.is_some();
    let mut health = json!({
        "ok": true, "service": "CHATX_COMMUNITY", "ip": address,
        "pcName": pc, "port": port, "version": "8.1.4",
        "maxFileBytes": 2147483648_u64, "chunkBytes": 4194304,
        "maintenance": false, "protocol": 2, "temporary": !strict_auth,
        "database": false, "databaseIntegrity": "unavailable", "databaseWarning": ""
    });
    let (socket_layer, socket_io) = SocketIo::builder()
        .max_payload(16 * 1024 * 1024)
        .build_layer();
    let update_identity=Arc::new(crate::updates::Identity::default());
    let update_routes=crate::updates::router(admin.clone(),update_identity.clone(),update_permit,socket_io.clone());
    let devices = Arc::new(Mutex::new(DeviceRegistry::open(PathBuf::from("approved-devices.json"))?));
    let history_path = crate::history::host_history_path(Path::new("."))?;
    crate::history::initialize(&history_path).map_err(io::Error::other)?;
    recover_group_deletion(&history_path)?;
    storage::recover(&history_path)?;
    let integrity_path = history_path.clone();
    let integrity = tokio::task::spawn_blocking(move || {
        rusqlite::Connection::open_with_flags(integrity_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?
            .query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
    }).await.map_err(io::Error::other)?;
    health["database"] = json!(integrity.as_ref().is_ok_and(|status| status == "ok"));
    health["databaseIntegrity"] = json!(match &integrity {
        Ok(status) if status == "ok" => "ok",
        Ok(_) => "failed",
        Err(_) => "unavailable",
    });
    if health["database"] == true && history_path.ends_with(crate::history::ELECTRON_HISTORY) {
        crate::history::import_read_markers(Path::new("."), &history_path)
            .map_err(io::Error::other)?;
    }
    if health["database"] != true {
        health["databaseWarning"] = json!("Pemeriksaan integritas database chat gagal.");
    }
    let conversations = Arc::new(RwLock::new(load_conversations(Path::new("conversations.json"))?));
    let assets = Arc::new(crate::asset_library::Library::open(PathBuf::from("asset-library.json"),&history_path)?);
    let status_store = Arc::new(statuses::Statuses::open(PathBuf::from("statuses.json"))?);
    let cohost_access = Arc::new(crate::cohost::Access::open(PathBuf::from("moderation.json"))?);
    let attachments = Attachments {
        devices: devices.clone(),
        conversations: conversations.clone(),
        history: history_path.clone(),
        admin: admin.clone(), io: socket_io.clone(),
        access: cohost_access.clone(),
        assets: assets.clone(),
    };
    let upload_devices = devices.clone();
    let upload_admin = admin.clone();
    let upload_io = socket_io.clone();
    let bledar_devices = devices.clone();
    let bledar_admin = admin.clone();
    let bledar_host_access = admin.clone();
    let video_access = cohost_access.clone();
    let video_devices = devices.clone();
    let identity_devices = devices.clone();
    let identity_admin = admin.clone();
    let cohost_users = devices.clone();
    let cohost_user_io = socket_io.clone();
    let cohost_io = socket_io.clone();
    let policy_events = socket_io.clone();
    let policy_devices = devices.clone();
    let moderation_events_access = cohost_access.clone();
    let login_access = cohost_access.clone();
    let route_access = cohost_access.clone();
    let route_devices = devices.clone();
    let route_admin = admin.clone();
    let profiles = Arc::new(crate::profiles::Profiles::open(std::env::current_dir()?)?);
    let cohost_profiles = profiles.clone();
    let profile_devices = devices.clone();
    let profile_admin = admin.clone();
    let profile_io = socket_io.clone();
    let profile_events = socket_io.clone();
    let profile_event_devices = devices.clone();
    let presence_profiles = profiles.clone();
    let diagnostics = Arc::new(crate::diagnostics::Diagnostics::open(PathBuf::from("diagnostics.json"))?);
    let socket_diagnostics = diagnostics.clone();
    let gallery = crate::gallery::Gallery::open(std::env::current_dir()?)
        .map(Arc::new).map_err(|error| error.to_string());
    let gallery_transfers = Arc::new(gallery_transfer::Transfers::default());
    let system = system_status::Service::new(attachments.clone(), gallery.clone().ok(), gallery_transfers.clone(), health.clone(), snapshot.clone());
    let runtime_sampler = system.clone();
    let diagnostics_devices = devices.clone();
    let diagnostics_admin = admin.clone();
    let diagnostics_access = cohost_access.clone();
    let diagnostics_io = socket_io.clone();
    let game_server = Arc::new(crate::games::Server::open(PathBuf::from("games.json"), profiles.clone())?);
    let game_order = Arc::new(tokio::sync::Mutex::new(()));
    let game_tick = game_server.clone();
    let game_timer_changed = Arc::new(tokio::sync::Notify::new());
    let game_timer_wakeup = game_timer_changed.clone();
    let game_tick_order = game_order.clone();
    let game_tick_devices = devices.clone();
    let game_tick_io = socket_io.clone();
    let presence_io = socket_io.clone();
    let bledar_library = crate::bledar::Library::open(PathBuf::from("bledar-videos"), move |headers, uri| {
        let token = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| uri.query()?.split('&').find_map(|part| part.strip_prefix("kc_token=")));
        token.is_some_and(|token| {
            let host = bledar_host_access.as_deref().is_some_and(|expected| valid_admin_token(token)
                && expected.bytes().zip(token.bytes()).fold(0, |difference, (a, b)| difference | (a ^ b)) == 0);
            host || bledar_devices.lock().expect("device registry").resolve_session(token, Instant::now()).is_some()
        })
    }, move |headers| {
        let supplied = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        let host = bledar_admin.as_deref().zip(supplied).is_some_and(|(expected, supplied)| valid_admin_token(supplied)
            && expected.bytes().zip(supplied.bytes()).fold(0, |difference, (a, b)| difference | (a ^ b)) == 0);
        if host { return true; }
        let Some(token) = supplied else { return false };
        let mut registry = video_devices.lock().expect("device registry");
        let Some(id) = registry.resolve_session(token, Instant::now()).map(str::to_owned) else { return false };
        registry.records.get(&id).and_then(|record| record["name"].as_str())
            .is_some_and(|name| video_access.policy(name,&id,false)["permissions"]["banned"] != true && video_access.scopes(name, false)["manageVideos"] == true)
    })?;
    let message_videos = bledar_library.clone();
    let message_order = Arc::new(tokio::sync::Mutex::new(()));
    let tag_guard = Arc::new(crate::message_tags::Guard::default());
    let reaction_order = Arc::new(tokio::sync::Mutex::new(()));
    let socket_snapshot = snapshot.clone();
    let http_snapshot = snapshot.clone();
    socket_io.ns("/", move |socket: SocketRef| {
        let update_identity=update_identity.clone();
        socket.extensions.insert(socket_snapshot.clone());
        socket.extensions.insert(login_access.clone());
        socket.extensions.insert(presence_profiles.clone());
        let devices = devices.clone();
        let history_path = history_path.clone();
        let conversations = conversations.clone();
        let presence_io = presence_io.clone();
        let reaction_order = reaction_order.clone();
        let message_order = message_order.clone();
        let message_videos = message_videos.clone();
        let tag_guard = tag_guard.clone();
        let message_assets = assets.clone();
        let status_store = status_store.clone();
        let game_server = game_server.clone();
        let socket_diagnostics = socket_diagnostics.clone();
        let gallery = gallery.clone();
        let gallery_transfers = gallery_transfers.clone();
        let game_timer_changed = game_timer_changed.clone();
        let game_order = game_order.clone();
        let admin = admin.clone();
        async move {
            socket.on("peer-handover-request", |socket: SocketRef, Data(_): Data<serde_json::Value>, ack: AckSender| async move {
                let error = if socket.extensions.get::<SocketSession>().is_some() {
                    "Server ini bukan temporary peer"
                } else {
                    "Perangkat belum disetujui"
                };
                let _ = ack.send(&json!({"ok":false,"error":error}));
            });
            for event in ["status-list-request", "status-create", "status-delete"] {
                let store = status_store.clone();
                let devices = devices.clone();
                let conversations = conversations.clone();
                let io = presence_io.clone();
                socket.on(event, async move |socket: SocketRef, Data(input): Data<serde_json::Value>, ack: AckSender| {
                    let _snapshot = snapshot_write!(socket, ack);
                    let Some((device, name, _)) = conversation_for_socket(&socket, &devices, &conversations, "general") else {
                        let _ = ack.send(&json!({"ok":false,"error":"Belum login"})); return;
                    };
                    let host = socket.extensions.get::<SocketSession>().is_some_and(|session| session.is_host);
                    let access = socket.extensions.get::<Arc<crate::cohost::Access>>().expect("status access");
                    if access.policy(&name, &device, host)["permissions"]["banned"] == true {
                        let _ = ack.send(&json!({"ok":false,"code":"FEATURE_DENIED","error":"Akses dibatasi HOST"})); return;
                    }
                    let profiles = socket.extensions.get::<Arc<crate::profiles::Profiles>>().expect("status profiles");
                    let result = tokio::task::spawn_blocking(move || -> Result<_,String> {
                        let now = presence_time();
                        let mut reply = json!({"ok":true});
                        match event {
                            "status-create" => reply["item"] = store.create(&name, &device, &input, now)?,
                            "status-delete" => store.delete(&name, host, input["id"].as_str().unwrap_or_default())?,
                            _ => {}
                        }
                        let items = store.list(now, &profiles)?;
                        if event == "status-list-request" {reply["items"] = items.clone();}
                        Ok((reply, items))
                    }).await;
                    match result {
                        Ok(Ok((reply,items))) => {
                            let _ = ack.send(&reply);
                            if event != "status-list-request" {
                                for peer in io.sockets() {
                                    if conversation_for_socket(&peer, &devices, &conversations, "general").is_some() {
                                        let _ = peer.emit("status-updated", &json!({"items":items}));
                                    }
                                }
                            }
                        }
                        Ok(Err(error)) => {let _ = ack.send(&json!({"ok":false,"error":error}));}
                        Err(_) => {let _ = ack.send(&json!({"ok":false,"error":"Penyimpanan status berhenti"}));}
                    }
                });
            }
            let gallery_service = Arc::new(gallery_transfer::Service {
                transfers: gallery_transfers, gallery: gallery.clone(), devices: devices.clone(),
                conversations: conversations.clone(), history: history_path.clone(), io: presence_io.clone(),
            });
            gallery_service.clone().register(&socket);
            let gallery_devices = devices.clone();
            let gallery_io = presence_io.clone();
            for event in ["file-gallery-index-status", "file-gallery-delta", "file-gallery-query", "file-gallery-register"] {
                let gallery = gallery.clone();
                let devices = gallery_devices.clone();
                let io = gallery_io.clone();
                let history = history_path.clone();
                let conversations = conversations.clone();
                socket.on(event, async move |socket: SocketRef, Data(input): Data<serde_json::Value>, ack: AckSender| {
                    let _snapshot = if matches!(event,"file-gallery-delta"|"file-gallery-register") {
                        Some(snapshot_write!(socket,ack))
                    } else {None};
                    let Some(device) = gallery_device(&socket, &devices) else {
                        let _ = ack.send(&json!({"ok":false,"error":"Belum login / akses ditolak"}));
                        return;
                    };
                    if event == "file-gallery-register" && !gallery_legacy_allowed(&socket,&devices,&device.id) {
                        let _ = ack.send(&json!({"ok":false,"error":"CLIENT lama tidak didukung untuk Gallery."}));
                        return;
                    }
                    if event == "file-gallery-delta" && !gallery_can_write(&socket,&devices,&device.id) {
                        let _ = ack.send(&json!({"ok":false,"error":"Identitas perangkat Galeri tidak cocok."}));
                        return;
                    }
                    let gallery = match gallery {
                        Ok(gallery) => gallery,
                        Err(error) => {
                            let _ = ack.send(&json!({"ok":false,"error":format!("SQLite Gallery tidak tersedia: {error}")}));
                            return;
                        }
                    };
                    let online = if event == "file-gallery-query" {
                        io.sockets().iter().filter_map(|peer| gallery_source(peer,&devices).map(|device|device.id)).collect()
                    } else { Default::default() };
                    let rooms = if event == "file-gallery-query" {
                        gallery_rooms(&socket,&devices,&conversations)
                    } else { serde_json::Value::Null };
                    let query_rooms = rooms.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        let reply = match event {
                            "file-gallery-delta" => gallery.delta(&device, &input),
                            "file-gallery-register" => gallery.register(&device,&input),
                            "file-gallery-query" => gallery.query(&input, &online, &history, &query_rooms),
                            _ => gallery.status(&device.id),
                        };
                        (device, reply)
                    }).await;
                    match result {
                        Ok((device, Ok(reply))) if gallery_device(&socket, &devices).is_some() &&
                            (event != "file-gallery-query" || gallery_rooms(&socket,&devices,&conversations) == rooms) => {
                            let _ = ack.send(&reply);
                            if matches!(event,"file-gallery-delta"|"file-gallery-register") && reply["staged"] != true && reply["duplicate"] != true {
                                let update = json!({"deviceId":device.id,"name":device.name,"pc":device.pc,"count":reply["count"]});
                                for peer in io.sockets().iter().filter(|peer| gallery_device(peer, &devices).is_some()) {
                                    let _ = peer.emit("file-gallery-index-updated", &update);
                                }
                            }
                        }
                        Ok((_, Err(error))) => {
                            let error = error.to_string();
                            let _ = ack.send(&json!({"ok":false,"resync":error=="RESYNC_REQUIRED","error":error}));
                        }
                        _ => { let _ = ack.send(&json!({"ok":false,"error":"Index Gallery tidak tersedia / sesi berakhir."})); }
                    }
                });
            }
            let report_io = presence_io.clone();
            let report_devices = devices.clone();
            socket.on("bug-report", async move |socket: SocketRef, Data(input): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot=snapshot_write!(socket,ack);
                if !input.is_object() || input.to_string().len() > 128 * 1024 {
                    let _ = ack.send(&json!({"ok":false,"error":"Data laporan tidak valid."}));
                    return;
                }
                let Some(name) = game_identity(&socket,&report_devices) else {
                    let _ = ack.send(&json!({"ok":false,"error":"Sesi tidak tersedia."}));
                    return;
                };
                let mut identity = socket.extensions.get::<Presence>().map(|presence|presence.public).unwrap_or_else(||json!({}));
                identity["name"] = json!(name);
                let store = socket_diagnostics.clone();
                match tokio::task::spawn_blocking(move || store.record(&input,&identity)).await {
                    Ok(Ok(result)) => {
                        emit_diagnostics_update(&report_io,&result);
                        let _ = ack.send(&json!({"ok":true,"signature":result["signature"],"count":result["count"]}));
                    }
                    _ => { let _ = ack.send(&json!({"ok":false,"error":"Gagal menyimpan laporan."})); }
                }
            });
            for &event in crate::games::EVENTS {
                let server = game_server.clone();
                let timer = game_timer_changed.clone();
                let order = game_order.clone();
                let devices = devices.clone();
                let io = presence_io.clone();
                socket.on(event, async move |socket: SocketRef, Data(input): Data<serde_json::Value>, ack: AckSender| {
                    let _snapshot=snapshot_write!(socket,ack);
                    let _ordered = order.lock().await;
                    let Some(actor) = game_identity(&socket, &devices) else {
                        let _ = ack.send(&json!({"ok":false,"error":"Belum login"}));
                        return;
                    };
                    let online: Vec<_> = io.sockets().iter().filter_map(|peer| game_identity(peer, &devices)).collect();
                    let result = tokio::task::spawn_blocking(move || server.handle(event, &actor, &input, &online)).await;
                    let Ok((reply, updates)) = result else {
                        let _ = ack.send(&json!({"ok":false,"error":"Pemrosesan pertandingan berhenti."}));
                        return;
                    };
                    if !updates.is_empty() { timer.notify_one(); }
                    emit_game_updates(&io, &devices, updates);
                    if event == "game-state-request" && reply["ok"] == true {
                        let _ = socket.emit("game-state", &reply["state"]);
                    }
                    let _ = ack.send(&reply);
                });
            }
            let disconnect_devices = devices.clone();
            let disconnect_io = presence_io.clone();
            socket.on_disconnect(async move |socket: SocketRef| {
                gallery_service.disconnected(&socket);
                socket.extensions.remove::<Presence>();
                emit_presence(&disconnect_io, &disconnect_devices, true);
            });
            let activity_devices = devices.clone();
            let activity_io = presence_io.clone();
            socket.on("activity-update", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let updated = {
                    let mut registry = activity_devices.lock().expect("device registry");
                    let session = socket.extensions.get::<SocketSession>();
                    let active = session.as_ref().is_some_and(|session| registry.resolve_session(&session.token, Instant::now()).is_some());
                    if active {
                        if let Some(mut presence) = socket.extensions.get::<Presence>() {
                            presence.update_activity(&raw);
                            socket.extensions.insert(presence);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if updated {
                    emit_presence(&activity_io, &activity_devices, false);
                    let _ = ack.send(&json!({"ok":true}));
                } else {
                    let _ = ack.send(&json!({"ok":false,"error":"Belum login"}));
                }
            });
            let login_devices = devices.clone();
            let login_conversations = conversations.clone();
            let login_path = history_path.clone();
            let login_io = presence_io.clone();
            let login_games = game_server.clone();
            let login_game_order = game_order.clone();
            let login_statuses = status_store.clone();
            socket.on("hello", async move |socket: SocketRef, Data(mut raw): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot=snapshot_write!(socket,ack);
                if let Some(access) = socket.extensions.get::<Arc<crate::cohost::Access>>() {
                    let role = raw["teamRole"].as_str().unwrap_or("EDITOR").trim().to_uppercase();
                    raw["teamRole"] = json!(if access.jobs().contains(&role) { role } else { "EDITOR".into() });
                }
                let mut reply = {
                    let mut registry = login_devices.lock().expect("device registry");
                    let host_authorized = is_admin(&raw, admin.as_deref());
                    let protected_lan = strict_auth && !host_authorized;
                    let registration = if protected_lan && admin.is_some() {
                        register_lan_device(&mut registry, &raw, &login_conversations, &socket)
                    } else {
                        Ok(())
                    };
                    let mut reply = match registration {
                        Ok(()) => login_lab_device(&mut registry, &raw, protected_lan),
                        Err(error) => error,
                    };
                    if reply["ok"] == true {
                        let device_id = raw["deviceId"].as_str().unwrap_or_default().to_owned();
                        let id = device_id.as_str();
                        if let Some(saved) = registry.records.get(id) {
                            let mut record = saved.clone();
                            // Electron registries do not carry the Rust lab approval field.
                            // Only a successful keyed desktop LAN login can supply this default.
                            if protected_lan && admin.is_some() && record.get("labPrivateHistoryApproved").is_none() {
                                record["labPrivateHistoryApproved"] = json!(true);
                            }
                            let assigned = socket.extensions.get::<Arc<crate::cohost::Access>>()
                                .and_then(|access| access.assigned_job(record["name"].as_str().unwrap_or_default(), id));
                            if let Some(role) = assigned { raw["teamRole"] = json!(role); }
                            record["teamRole"] = raw["teamRole"].clone();
                            record["lastSeen"] = json!(presence_time());
                            for (field, limit) in [("pc", 80), ("version", 40)] {
                                let value = clip_presence(&raw[field], limit);
                                if !value.is_empty() {
                                    record[field] = json!(value);
                                }
                            }
                            if is_admin(&raw, admin.as_deref()) { record["isHost"] = json!(true); }
                            if registry.save_record(id.to_owned(), record).is_err() {
                                if let Some(token) = reply["token"].as_str() {
                                    registry.sessions.remove(token);
                                }
                                reply = json!({"ok":false,"error":"Identitas perangkat gagal disimpan.","code":"DEVICE_STORAGE_FAILED"});
                            }
                        }
                    }
                    socket.extensions.remove::<SocketSession>();
                    socket.extensions.remove::<Presence>();
                    if let Some(token) = reply["token"].as_str() {
                        let key = raw["deviceKey"].as_str().unwrap_or_default();
                        socket.extensions.insert(SocketSession {
                            token: token.to_owned(),
                            key_hash: format!("{:x}", Sha256::digest(key.as_bytes())),
                            is_host: is_admin(&raw, admin.as_deref()),
                        });
                        socket.extensions.insert(Presence::from_login(&reply, &raw, is_admin(&raw, admin.as_deref())));
                    }
                    reply
                };
                if reply["ok"]==true {
                    if let Ok((key,fingerprint))=update_identity.public() {
                        reply["hostPublicKey"]=json!(key);
                        reply["hostFingerprint"]=json!(fingerprint);
                    }
                }
                let _ = socket.emit("session-auth", &reply);
                let _ = ack.send(&reply);
                if reply["ok"] == true {
                    let _ordered = login_game_order.lock().await;
                    if let Some(actor) = game_identity(&socket, &login_devices) {
                        for (event, data) in login_games.reconnect(&actor) { let _ = socket.emit(event, &data); }
                    }
                }
                emit_presence(&login_io, &login_devices, true);
                if let Some(token) = reply["token"].as_str() {
                    let items = conversation_list(&socket, &login_devices, &login_conversations, login_path.clone()).await;
                    let path = login_path.clone();
                    let history = tokio::task::spawn_blocking(move || crate::history::page(&path, "general", 120, None, "")).await;
                    let store = login_statuses.clone();
                    let profiles = socket.extensions.get::<Arc<crate::profiles::Profiles>>().expect("status profiles");
                    let statuses = tokio::task::spawn_blocking(move || store.list(presence_time(), &profiles)).await;
                    let mut registry = login_devices.lock().expect("device registry");
                    let same_session = socket.extensions.get::<SocketSession>().is_some_and(|session| session.token == token);
                    if same_session && registry.resolve_session(token, Instant::now()).is_some() {
                        if let Ok(items) = items {
                            let _ = socket.emit("conversation-list", &items);
                        }
                        if let Ok(Ok(history)) = history {
                            let _ = socket.emit("history", &history["items"]);
                        }
                        match statuses {
                            Ok(Ok(items)) => {let _ = socket.emit("status-list", &items);}
                            _ => {let _ = socket.emit("status-error", &json!({"error":"Daftar Status belum dapat dibaca"}));}
                        }
                    }
                }
            });
            for (event, group) in [("conversation-create-dm", false), ("conversation-create-group", true)] {
                let devices = devices.clone();
                let conversations = conversations.clone();
                let history_path = history_path.clone();
                let io = presence_io.clone();
                socket.on(event, async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                    let _snapshot=snapshot_write!(socket,ack);
                    let Some((actor, names)) = conversation_creator(&socket, &devices) else {
                        let _ = ack.send(&json!({"ok":false,"error":"Akses percakapan privat belum disetujui HOST"}));
                        return;
                    };
                    let writable = conversations.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        create_conversation(&writable, &actor, &names, &raw, group)
                    }).await;
                    match result {
                        Ok(Ok(item)) => {
                            let id = item["id"].as_str().unwrap_or_default();
                            let _ = ack.send(&json!({"ok":true,"conversation":item}));
                            for peer in io.sockets() {
                                if conversation_for_socket(&peer, &devices, &conversations, id).is_some() {
                                    if let Ok(items) = conversation_list(&peer, &devices, &conversations, history_path.clone()).await {
                                        if conversation_for_socket(&peer, &devices, &conversations, id).is_some() {
                                            let _ = peer.emit("conversation-list", &items);
                                        }
                                    }
                                }
                            }
                        }
                        Ok(Err(error)) => { let _ = ack.send(&json!({"ok":false,"error":error})); }
                        Err(_) => { let _ = ack.send(&json!({"ok":false,"error":"Percakapan belum tersimpan"})); }
                    }
                });
            }
            for (event, action) in [
                ("conversation-general-update", GroupChange::General),
                ("conversation-group-update", GroupChange::Edit),
                ("conversation-leave", GroupChange::Leave),
            ] {
                let devices = devices.clone();
                let conversations = conversations.clone();
                let path = history_path.clone();
                let io = presence_io.clone();
                let order = message_order.clone();
                let updates = reaction_order.clone();
                socket.on(event, async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                    let _snapshot = snapshot_write!(socket, ack);
                    let _order = order.lock().await;
                    let _updates = updates.lock().await;
                    let id = if action == GroupChange::General { "general" } else { raw["id"].as_str().unwrap_or_default() };
                    let host = socket.extensions.get::<SocketSession>().is_some_and(|session| session.is_host);
                    if action == GroupChange::General && !host {
                        let _ = ack.send(&json!({"ok":false,"error":"HOST only"}));
                        return;
                    }
                    let access_id = if host && action == GroupChange::Edit { "general" } else { id };
                    let Some((device, actor, _)) = conversation_for_socket(&socket, &devices, &conversations, access_id) else {
                        let _ = ack.send(&json!({"ok":false,"error":"Grup tidak ditemukan"}));
                        return;
                    };
                    let access = socket.extensions.get::<Arc<crate::cohost::Access>>().expect("message access policy");
                    if access.policy(&actor, &device, host)["permissions"]["banned"] == true {
                        let _ = ack.send(&json!({"ok":false,"error":"Akses dibatasi oleh HOST","code":"FEATURE_DENIED"}));
                        return;
                    }
                    let peers: Vec<_> = io.sockets().into_iter().filter(|peer| {
                        conversation_for_socket(peer, &devices, &conversations, id).is_some()
                    }).collect();
                    let known = devices.lock().expect("device registry").records.values()
                        .filter(|record| record["revoked"] != true)
                        .filter_map(|record| record["name"].as_str().map(str::to_owned))
                        .collect::<Vec<_>>();
                    let id = id.to_owned();
                    let update_id = id.clone();
                    let writable = conversations.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        change_group(&writable, &update_id, &actor, host, &known, action, &raw)
                    }).await;
                    match result {
                        Ok(Ok(())) => { let _ = ack.send(&json!({"ok":true})); }
                        Ok(Err(error)) => { let _ = ack.send(&json!({"ok":false,"error":error})); return; }
                        Err(_) => { let _ = ack.send(&json!({"ok":false,"retryable":true,"error":"Perubahan grup belum tersimpan"})); return; }
                    }
                    for peer in peers {
                        if conversation_for_socket(&peer, &devices, &conversations, &id).is_none() {
                            let _ = peer.emit("conversation-removed", &json!({"id":id}));
                        }
                    }
                    let general_history = if action == GroupChange::General {
                        let path = path.clone();
                        tokio::task::spawn_blocking(move || crate::history::page(&path, "general", 120, None, "")).await.ok().and_then(Result::ok)
                    } else { None };
                    for peer in io.sockets() {
                        if let Ok(items) = conversation_list(&peer, &devices, &conversations, path.clone()).await {
                            let _ = peer.emit("conversation-list", &items);
                            if let Some(history) = &general_history {
                                let _ = peer.emit("conversation-refresh", &json!({"id":"general","items":history["items"]}));
                            }
                        }
                    }
                });
            }
            let list_devices = devices.clone();
            let delete_devices = devices.clone();
            let delete_conversations = conversations.clone();
            let delete_path = history_path.clone();
            let delete_io = presence_io.clone();
            let delete_order = message_order.clone();
            let delete_updates = reaction_order.clone();
            socket.on("conversation-delete-group", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot = snapshot_write!(socket, ack);
                let _order = delete_order.lock().await;
                let _updates = delete_updates.lock().await;
                let authorized = socket.extensions.get::<SocketSession>().is_some_and(|session| session.is_host)
                    && conversation_for_socket(&socket, &delete_devices, &delete_conversations, "general").is_some();
                if !authorized { let _ = ack.send(&json!({"ok":false,"error":"HOST only"})); return; }
                let id = raw["id"].as_str().unwrap_or_default().to_owned();
                let path = delete_path.clone();
                let writable = delete_conversations.clone();
                let delete_id = id.clone();
                let result = tokio::task::spawn_blocking(move || delete_group(&writable, &path, &delete_id)).await;
                let reply = match result {
                    Ok(Ok(())) => json!({"ok":true}),
                    Ok(Err(error)) => json!({"ok":false,"retryable":true,"error":error}),
                    Err(_) => json!({"ok":false,"retryable":true,"error":"Penghapusan grup belum selesai"}),
                };
                let _ = ack.send(&reply);
                if reply["ok"] != true { return; }
                for peer in delete_io.sockets() {
                    if let Ok(items) = conversation_list(&peer, &delete_devices, &delete_conversations, delete_path.clone()).await {
                        let _ = peer.emit("conversation-removed", &json!({"id":id}));
                        let _ = peer.emit("conversation-list", &items);
                        let _ = peer.emit("file-gallery-index-updated", &json!({"deviceId":"__chat__","name":"ChatX","pc":"HOST · Chat"}));
                    }
                }
            });
            let list_conversations = conversations.clone();
            let list_path = history_path.clone();
            let admin_list_devices = devices.clone();
            let admin_list_conversations = conversations.clone();
            let admin_list_path = history_path.clone();
            socket.on("host-conversation-list", async move |socket: SocketRef, ack: AckSender| {
                if !socket.extensions.get::<SocketSession>().is_some_and(|session| session.is_host) {
                    let _ = ack.send(&json!({"ok":false,"error":"HOST only"}));
                    return;
                }
                let result = conversation_list_scope(&socket, &admin_list_devices, &admin_list_conversations, admin_list_path.clone(), true).await;
                let reply = match result {
                    Ok(items) => json!({"ok":true,"items":items}),
                    Err(error) => json!({"ok":false,"error":error}),
                };
                let _ = ack.send(&reply);
            });
            socket.on("conversation-list-request", async move |socket: SocketRef, ack: AckSender| {
                let reply = match conversation_list(&socket, &list_devices, &list_conversations, list_path.clone()).await {
                    Ok(items) => json!({"ok":true,"items":items}),
                    Err(error) => json!({"ok":false,"error":error}),
                };
                let _ = ack.send(&reply);
            });
            let open_devices = devices.clone();
            let open_conversations = conversations.clone();
            let open_path = history_path.clone();
            let open_order = reaction_order.clone();
            socket.on("conversation-open", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot=snapshot_write!(socket,ack);
                let _order = open_order.lock().await;
                let id = raw["id"].as_str().filter(|id| !id.is_empty()).unwrap_or("general");
                let Some((_, name, conversation)) = conversation_for_socket(&socket, &open_devices, &open_conversations, id) else {
                    let _ = ack.send(&json!({"ok":false,"error":"Chat tidak ditemukan"}));
                    return;
                };
                let path = open_path.clone();
                let id = id.to_owned();
                let dm = conversation["type"] == "dm";
                let read_id = id.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::history::mark_read(&path, &read_id, &name, dm, presence_time() as i64)
                }).await;
                let reply = match result {
                    Ok(Ok(items)) => {
                        for peer in socket.broadcast().sockets() {
                            let Some((device, _, _)) = conversation_for_socket(&peer, &open_devices, &open_conversations, &id) else { continue };
                            for item in items.iter().filter(|item| item["senderDeviceId"].as_str() == Some(device.as_str())) {
                                let receipt = item.get("receipt").filter(|value| value.is_object()).cloned()
                                    .unwrap_or_else(|| json!({"deliveredAt":0,"readAt":0}));
                                let _ = peer.emit("message-receipt", &json!({
                                    "id":item["id"],"conversationId":id,"receipt":receipt,"readBy":item["readBy"].as_object().cloned().unwrap_or_default()
                                }));
                            }
                        }
                        json!({"ok":true})
                    }
                    _ => json!({"ok":false,"error":"Status baca tidak dapat disimpan"}),
                };
                let _ = ack.send(&reply);
            });
            let typing_devices = devices.clone();
            let typing_conversations = conversations.clone();
            socket.on("typing-state", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>| {
                let Some(id) = raw["id"].as_str() else { return };
                let Some((_, name, _)) = conversation_for_socket(&socket, &typing_devices, &typing_conversations, id) else { return };
                let state = json!({"conversationId":id,"name":name,"typing":raw["typing"] == true,"ts":presence_time()});
                for peer in socket.broadcast().sockets() {
                    if conversation_member_for_socket(&peer, &typing_devices, &typing_conversations, id) {
                        let _ = peer.emit("typing-state", &state);
                    }
                }
            });
            let details_devices = devices.clone();
            let details_conversations = conversations.clone();
            let details_path = history_path.clone();
            socket.on("message-details", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let missing = json!({"ok":false,"error":"Pesan tidak ditemukan"});
                let Some(id) = raw["id"].as_str().filter(|id| !id.is_empty()).map(str::to_owned) else {
                    let _ = ack.send(&missing);
                    return;
                };
                let path = details_path.clone();
                let Ok(Ok(Some(item))) = tokio::task::spawn_blocking(move || crate::history::message(&path, &id)).await else {
                    let _ = ack.send(&missing);
                    return;
                };
                let id = item["conversationId"].as_str().or_else(||item["room"].as_str()).unwrap_or("general");
                let Some((_, _, conversation)) = conversation_for_socket(&socket, &details_devices, &details_conversations, id) else {
                    let _ = ack.send(&missing);
                    return;
                };
                let sender = item["sender"].as_str().unwrap_or_default().trim().to_lowercase();
                let members: Vec<_> = conversation["members"].as_array().into_iter().flatten().filter_map(|name| {
                    let name = name.as_str()?;
                    let key = name.trim().to_lowercase();
                    (key != sender).then(|| json!({"name":name,"readAt":item["readBy"][crate::profiles::key(name)]["at"].as_i64().unwrap_or_default()}))
                }).collect();
                let _ = ack.send(&json!({"ok":true,"conversationType":conversation["type"],"members":members,"receipt":item["receipt"]}));
            });
            let send_devices = devices.clone();
            let reaction_devices = devices.clone();
            let reaction_conversations = conversations.clone();
            let reaction_path = history_path.clone();
            let edit_order = reaction_order.clone();
            let forward_source_order = reaction_order.clone();
            socket.on("message-react", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot=snapshot_write!(socket,ack);
                let Some(id) = raw["id"].as_str().filter(|id| !id.is_empty()).map(str::to_owned) else {
                    let _ = ack.send(&json!({"ok":false,"error":"Pesan tidak tersedia"}));
                    return;
                };
                let path = reaction_path.clone();
                let lookup = id.clone();
                let found = tokio::task::spawn_blocking(move || crate::history::message(&path, &lookup)).await;
                let Ok(Ok(Some(message))) = found else {
                    let _ = ack.send(&json!({"ok":false,"error":"Pesan tidak tersedia"}));
                    return;
                };
                let conversation = message["conversationId"].as_str().unwrap_or_default();
                let _order = reaction_order.lock().await;
                let access = conversation_for_socket(&socket, &reaction_devices, &reaction_conversations, conversation);
                let Some((_, name, _)) = access.filter(|_| message["deleted"] != true) else {
                    let _ = ack.send(&json!({"ok":false,"error":"Pesan tidak tersedia"}));
                    return;
                };
                let emoji = raw["emoji"].as_str().unwrap_or_default();
                if !matches!(emoji, "👍" | "❤️" | "😂" | "😮" | "😢" | "🙏" | "👎" | "🔥" | "🎉" | "✅" |
                    "💯" | "😍" | "😎" | "😊" | "🤔" | "😅" | "👏" | "🥰" | "😭" | "😡" | "👀" | "💪" | "✨" | "🤣") {
                    let _ = ack.send(&json!({"ok":false,"error":"Emoji tidak valid"}));
                    return;
                }
                let path = reaction_path.clone();
                let conversation_id = conversation.to_owned();
                let emoji = emoji.to_owned();
                let result = tokio::task::spawn_blocking(move || {
                    crate::history::toggle_reaction(&path, &id, &conversation_id, &name, &emoji, presence_time() as i64)
                }).await;
                use crate::history::ReactionUpdate;
                let reply = match result {
                    Ok(Ok(ReactionUpdate::Applied(item))) => {
                        let event = json!({"id":conversation,"item":item});
                        for recipient in std::iter::once(socket.clone()).chain(socket.broadcast().sockets()) {
                            if conversation_member_for_socket(&recipient, &reaction_devices, &reaction_conversations, conversation) {
                                let _ = recipient.emit("message-reactions", &event);
                            }
                        }
                        json!({"ok":true,"item":item})
                    }
                    Ok(Ok(ReactionUpdate::Missing)) => json!({"ok":false,"error":"Pesan tidak tersedia"}),
                    Ok(Ok(ReactionUpdate::Full)) => json!({"ok":false,"error":"Reaksi pesan sudah penuh"}),
                    _ => json!({"ok":false,"retryable":true,"error":"Reaksi belum tersimpan"}),
                };
                let _ = ack.send(&reply);
            });
            for (event, delete) in [("edit-message", false), ("delete-message", true)] {
                let devices = devices.clone();
                let conversations = conversations.clone();
                let path = history_path.clone();
                let order = edit_order.clone();
                socket.on(event, async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                    let _snapshot=snapshot_write!(socket,ack);
                    let _order = order.lock().await;
                    let reply = change_chat_message(&socket, &devices, &conversations, path, raw, delete).await;
                    let _ = ack.send(&reply);
                });
            }
            let receipt_devices = devices.clone();
            let receipt_conversations = conversations.clone();
            let receipt_path = history_path.clone();
            socket.on("message-receipt", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot=snapshot_write!(socket,ack);
                let Some(id) = raw["id"].as_str().filter(|id| !id.is_empty()).map(str::to_owned) else {
                    let _ = ack.send(&json!({"ok":false,"error":"Receipt tidak valid"}));
                    return;
                };
                let read = match raw["stage"].as_str() {
                    Some("read") => true,
                    Some("delivered") => false,
                    _ => {
                        let _ = ack.send(&json!({"ok":false,"error":"Receipt tidak valid"}));
                        return;
                    }
                };
                let path = receipt_path.clone();
                let message_id = id.clone();
                let stored = tokio::task::spawn_blocking(move || crate::history::message(&path, &message_id)).await;
                let Ok(Ok(Some(stored))) = stored else {
                    let _ = ack.send(&json!({"ok":false,"error":"Receipt tidak valid"}));
                    return;
                };
                let conversation = stored["conversationId"].as_str().unwrap_or_default();
                let access = conversation_for_socket(&socket, &receipt_devices, &receipt_conversations, conversation);
                let Some((_, name, _)) = access.filter(|(_, _, metadata)| metadata["type"] == "dm") else {
                    let _ = ack.send(&json!({"ok":false,"error":"Receipt tidak valid"}));
                    return;
                };
                let path = receipt_path.clone();
                let conversation_id = conversation.to_owned();
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
                let result = tokio::task::spawn_blocking(move || {
                    crate::history::update_receipt(&path, &id, &conversation_id, &name, read, now)
                }).await;
                let reply = match result {
                    Ok(Ok(Some(item))) => {
                        for peer in socket.broadcast().sockets() {
                            let access = conversation_for_socket(&peer, &receipt_devices, &receipt_conversations, conversation);
                            if access.is_some_and(|(device, _, _)| item["senderDeviceId"].as_str() == Some(device.as_str())) {
                                let _ = peer.emit("message-receipt", &json!({
                                    "id":item["id"],"conversationId":conversation,"receipt":item["receipt"],"readBy":{}
                                }));
                            }
                        }
                        json!({"ok":true,"receipt":item["receipt"]})
                    }
                    Ok(Ok(None)) => json!({"ok":false,"error":"Receipt tidak valid"}),
                    _ => json!({"ok":false,"error":"Receipt tidak tersimpan"}),
                };
                let _ = ack.send(&reply);
            });
            let send_path = history_path.clone();
            let send_conversations = conversations.clone();
            let message_statuses = status_store.clone();
            let forward_devices = devices.clone();
            let forward_conversations = conversations.clone();
            let forward_path = history_path.clone();
            let forward_order = message_order.clone();
            socket.on("message-forward", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot = snapshot_write!(socket, ack);
                let _order = forward_order.lock().await;
                let _source_order = forward_source_order.lock().await;
                let unavailable = json!({"ok":false,"error":"Pesan/tujuan tidak tersedia"});
                let target = raw["conversationId"].as_str().unwrap_or_default();
                let Some((device, name, metadata)) = conversation_for_socket(&socket, &forward_devices, &forward_conversations, target) else {
                    let _ = ack.send(&unavailable);
                    return;
                };
                let Some(source_id) = raw["id"].as_str().filter(|id| !id.is_empty()).map(str::to_owned) else {
                    let _ = ack.send(&unavailable);
                    return;
                };
                let path = forward_path.clone();
                let source = tokio::task::spawn_blocking(move || crate::history::message(&path, &source_id)).await;
                let source = match source {
                    Ok(Ok(Some(source))) => source,
                    Ok(Ok(None)) => { let _ = ack.send(&unavailable); return; }
                    _ => { let _ = ack.send(&json!({"ok":false,"retryable":true,"error":"Pesan asal belum dapat dibaca."})); return; }
                };
                let origin = source["conversationId"].as_str().or_else(|| source["room"].as_str()).unwrap_or("general");
                if source["deleted"] == true || conversation_for_socket(&socket, &forward_devices, &forward_conversations, origin).is_none() {
                    let _ = ack.send(&unavailable);
                    return;
                }
                let file = if source["attachmentRemoved"] == true { serde_json::Value::Null } else { source["file"].clone() };
                if file.is_null() && source["text"].as_str().unwrap_or_default().trim().is_empty() {
                    let _ = ack.send(&unavailable);
                    return;
                }
                let host = socket.extensions.get::<SocketSession>().is_some_and(|session| session.is_host);
                let access = socket.extensions.get::<Arc<crate::cohost::Access>>().expect("message access policy");
                let permissions = access.policy(&name, &device, host)["permissions"].clone();
                if permissions["banned"] == true || permissions["sendChat"] == false
                    || (file.is_object() && permissions["sendFiles"] == false) {
                    let _ = ack.send(&json!({"ok":false,"error":"Kirim chat atau file dibatasi oleh HOST.","code":"FEATURE_DENIED"}));
                    return;
                }
                let client_id = raw["clientMessageId"].as_str().filter(|id| !id.is_empty())
                    .map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if client_id.len() > 100 || !client_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) {
                    let _ = ack.send(&json!({"ok":false,"error":"ID pesan tidak valid"}));
                    return;
                }
                let id = format!("m-{:x}", Sha256::digest(format!("{device}|{client_id}").as_bytes()));
                let avatar = socket.extensions.get::<Arc<crate::profiles::Profiles>>()
                    .map(|profiles| profiles.get(&name)["avatarUrl"].clone());
                let ts = presence_time() as i64;
                let mut item = json!({
                    "id":id,"ts":ts,"sender":name,"senderDeviceId":device,"senderAvatar":avatar,
                    "text":source["text"],"file":file,"sticker":source["sticker"] == true && !file.is_null(),
                    "conversationId":target,"room":target,"conversationType":metadata["type"],
                    "reply":null,"statusReply":null,"workTag":null,"workTagTarget":null,"tagScope":null,"reactions":[],
                    "tagVideoEnabled":false,"mentionLevel":"standard",
                    "forwarded":true,"forwardedFrom":{"sender":source["sender"],"ts":source["ts"]}
                });
                if metadata["type"] == "dm" {
                    item["receipt"] = json!({"deliveredAt":0,"readAt":0});
                } else {
                    item["readBy"] = json!({});
                }
                let path = forward_path.clone();
                let saved = item.clone();
                let result = tokio::task::spawn_blocking(move || crate::history::insert_message(&path, &saved, ts)).await;
                let reply = match result {
                    Ok(Ok(true)) => {
                        for peer in std::iter::once(socket.clone()).chain(socket.broadcast().sockets()) {
                            if conversation_member_for_socket(&peer, &forward_devices, &forward_conversations, target) {
                                let _ = peer.emit("message", &item);
                            }
                        }
                        json!({"ok":true,"id":id})
                    }
                    Ok(Ok(false)) => json!({"ok":true,"id":id,"duplicate":true}),
                    _ => json!({"ok":false,"retryable":true,"error":"Forward belum tersimpan"}),
                };
                let _ = ack.send(&reply);
            });
            socket.on("message", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let _snapshot=snapshot_write!(socket,ack);
                let _message_guard = message_order.lock().await;
                let conversation = raw["conversationId"].as_str().filter(|id| !id.is_empty()).unwrap_or("general");
                let Some((device, name, metadata)) = conversation_for_socket(&socket, &send_devices, &send_conversations, conversation) else {
                    let _ = ack.send(&json!({"ok":false,"error":"Chat tidak ditemukan / akses ditolak"}));
                    return;
                };
                if let Some(access) = socket.extensions.get::<Arc<crate::cohost::Access>>() {
                    let host = socket.extensions.get::<SocketSession>().is_some_and(|session| session.is_host);
                    let permissions = access.policy(&name, &device, host)["permissions"].clone();
                    let denied = permissions["banned"] == true || permissions["sendChat"] == false
                        || (raw["file"].is_object() && permissions["sendFiles"] == false);
                    if denied {
                        let _ = ack.send(&json!({"ok":false,"error":"Kirim chat atau file dibatasi oleh HOST.","code":"FEATURE_DENIED"}));
                        return;
                    }
                }
                let client_id = raw["clientMessageId"].as_str().unwrap_or_default();
                if !(12..=100).contains(&client_id.len()) || !client_id.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
                }) {
                    let _ = ack.send(&json!({"ok":false,"error":"CLIENT perlu diperbarui: ID pesan tidak valid"}));
                    return;
                }
                let id = format!("m-{:x}", Sha256::digest(format!("{device}|{client_id}").as_bytes()));
                let duplicate_path = send_path.clone();
                let duplicate_id = id.clone();
                match tokio::task::spawn_blocking(move || crate::history::message(&duplicate_path, &duplicate_id)).await {
                    Ok(Ok(Some(existing))) => {
                        if existing["conversationId"] == "general" {
                            let assets = message_assets.clone();
                            match tokio::task::spawn_blocking(move || assets.register(&existing)).await {
                                Ok(Ok(Some(kind))) => {for peer in socket.broadcast().sockets() {let _=peer.emit("asset-library-updated",&json!({"kind":kind}));}}
                                Ok(Ok(None)) => {}
                                _ => {let _=ack.send(&json!({"ok":false,"retryable":true,"id":id,"error":"Pesan tersimpan, tetapi pustaka lampiran belum dapat disimpan."}));return;}
                            }
                        }
                        let _ = ack.send(&json!({"ok":true,"id":id,"duplicate":true})); return;
                    }
                    Ok(Ok(None)) => {}
                    _ => { let _ = ack.send(&json!({"ok":false,"retryable":true,"error":"Status pesan belum dapat dibaca dari storage HOST."})); return; }
                }
                let text: String = raw["text"].as_str().unwrap_or_default().chars().take(8000).collect();
                if text.trim().is_empty() && raw["file"].is_null() {
                    let _ = ack.send(&json!({"ok":false,"error":"Format pesan belum didukung di lab"}));
                    return;
                }
                let file = if raw["file"].is_null() {
                    serde_json::Value::Null
                } else {
                    let stored = raw["file"]["stored"].as_str().unwrap_or_default();
                    let session = socket.extensions.get::<SocketSession>();
                    let file = match session {
                        Some(session) if send_devices.lock().expect("device registry")
                            .records.get(&device).and_then(|record| record["keyHash"].as_str())
                            == Some(session.key_hash.as_str()) =>
                            crate::uploads::chat_attachment(stored, &device, &session.key_hash).await,
                        _ => Ok(None),
                    };
                    let file = match file {
                        Ok(Some(file)) => file,
                        Ok(None) => {
                            let _ = ack.send(&json!({"ok":false,"code":"ATTACHMENT_UNAVAILABLE","error":"Lampiran belum terdaftar untuk perangkat ini di HOST. Coba lagi dari antrean atau unggah ulang file asli."}));
                            return;
                        }
                        Err(_) => {
                            let _ = ack.send(&json!({"ok":false,"code":"ATTACHMENT_INDEX_UNAVAILABLE","retryable":true,"error":"HOST belum bisa membaca indeks lampiran. Pesan akan dicoba lagi otomatis."}));
                            return;
                        }
                    };
                    serde_json::Value::Object(file)
                };
                let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
                let status_store = message_statuses.clone();
                let reply_id = raw["statusReplyToId"].as_str().unwrap_or_default().to_owned();
                let status_conversation = metadata.clone();
                let status_actor = name.clone();
                let status_reply = match tokio::task::spawn_blocking(move ||
                    status_store.reply(&reply_id, &status_conversation, &status_actor, ts as u64)).await {
                    Ok(Ok(reply)) => reply,
                    _ => {let _ = ack.send(&json!({"ok":false,"retryable":true,"error":"Status asal belum dapat dibaca"}));return;}
                };
                let host = socket.extensions.get::<SocketSession>().is_some_and(|session| session.is_host);
                let access = socket.extensions.get::<Arc<crate::cohost::Access>>().expect("message access policy");
                let policy = access.policy(&name, &device, host);
                let known: Vec<_> = send_devices.lock().expect("device registry").records.values()
                    .filter_map(|record| record["name"].as_str().map(str::to_owned)).collect();
                let aliases = socket.extensions.get::<Arc<crate::profiles::Profiles>>().map(|profiles|
                    known.iter().filter_map(|name| profiles.get(name)["displayName"].as_str()
                        .map(|display| (name.to_lowercase(),display.to_owned()))).collect()).unwrap_or_default();
                let mut tags = match crate::message_tags::prepare_with_profiles(&text, &raw, &name, host, &known, &metadata, &policy, ts as u64, &aliases) {
                    Ok(tags) => tags,
                    Err(error) => { let _ = ack.send(&error); return; }
                };
                if tags.everyone && !host {
                    match access.consume_everyone(&device, ts as u64) {
                        Ok(None) => {}
                        Ok(Some(wait)) => { let _ = ack.send(&json!({"ok":false,"error":format!("@everyone cooldown. Coba lagi sekitar {} menit.",wait.div_ceil(60000))})); return; }
                        Err(error) => { let _ = ack.send(&json!({"ok":false,"retryable":true,"error":error})); return; }
                    }
                }
                if let Err(error) = tag_guard.check(&device, host, &tags, &access.tag_spam_policy(), ts as u64) {
                    let _ = ack.send(&error); return;
                }
                if tags.fields["tagVideoEnabled"] == true {
                    match message_videos.pick_video().await {
                        Ok(video) => tags.fields["bledarVideo"] = video,
                        Err(error) => { let _ = ack.send(&json!({"ok":false,"retryable":true,"error":error})); return; }
                    }
                }
                let reply_id = raw["replyToId"].as_str().filter(|id| !id.is_empty()).map(str::to_owned);
                let reply_path = send_path.clone();
                let reply_conversation = conversation.to_owned();
                let quote = match reply_id {
                    Some(reply_id) => match tokio::task::spawn_blocking(move || {
                        crate::history::message(&reply_path, &reply_id)
                    }).await {
                        Ok(Ok(Some(message))) => reply_snapshot(&message, &reply_conversation),
                        Ok(Ok(None)) => serde_json::Value::Null,
                        _ => {
                            let _ = ack.send(&json!({"ok":false,"retryable":true,"error":"Pesan yang dibalas belum dapat dibaca"}));
                            return;
                        }
                    },
                    None => serde_json::Value::Null,
                };
                let mut item = json!({
                    "id": id, "ts": ts, "sender": name, "senderDeviceId": device, "senderAvatar": null,
                    "text": text, "file": file, "sticker": raw["sticker"] == true, "reply": quote, "room": conversation,
                    "conversationId": conversation, "conversationType": metadata["type"],
                    "mentionLevel": "standard"
                });
                if let Some(profiles) = socket.extensions.get::<Arc<crate::profiles::Profiles>>() {
                    item["senderAvatar"] = profiles.get(&name)["avatarUrl"].clone();
                }
                item["statusReply"] = status_reply;
                item.as_object_mut().expect("chat message").extend(tags.fields.as_object().expect("tag metadata").clone());
                if metadata["type"] == "dm" {
                    item["receipt"] = json!({"deliveredAt": 0, "readAt": 0});
                } else {
                    item["readBy"] = json!({});
                }
                let path = send_path.clone();
                let stored_item = item.clone();
                let assets = message_assets.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let inserted = crate::history::insert_message(&path, &stored_item, ts).map_err(|error|error.to_string())?;
                    let kind = if stored_item["conversationId"] == "general" {assets.register(&stored_item)?} else {None};
                    Ok::<_,String>((inserted,kind))
                }).await;
                let reply = match result {
                    Ok(Ok((true,kind))) => {
                        for recipient in std::iter::once(socket.clone()).chain(socket.broadcast().sockets()) {
                            if conversation_member_for_socket(&recipient, &send_devices, &send_conversations, conversation) {
                                let _ = recipient.emit("message", &item);
                            }
                        }
                        if let Some(kind) = kind {for peer in std::iter::once(socket.clone()).chain(socket.broadcast().sockets()) {let _=peer.emit("asset-library-updated",&json!({"kind":kind}));}}
                        json!({"ok":true,"id":id})
                    }
                    Ok(Ok((false,_))) => json!({"ok":true,"id":id,"duplicate":true}),
                    Ok(Err(error)) => json!({"ok":false,"retryable":true,"id":id,"error":format!("Pesan atau pustaka lampiran belum selesai disimpan: {error}")}),
                    _ => json!({"ok":false,"retryable":true,"error":"Penyimpanan pesan berhenti. Periksa storage HOST."}),
                };
                let _ = ack.send(&reply);
            });
            socket.on("conversation-history", async move |socket: SocketRef, Data(raw): Data<serde_json::Value>, ack: AckSender| {
                let id = raw["id"].as_str().filter(|id| !id.is_empty()).unwrap_or("general");
                let Some((_, _, conversation)) = conversation_for_socket(&socket, &devices, &conversations, id) else {
                    let _ = ack.send(&json!({"ok": false, "error": "Chat tidak ditemukan / akses ditolak"}));
                    return;
                };
                let path = history_path.clone();
                let id = id.to_owned();
                let read_id = id.clone();
                let page = tokio::task::spawn_blocking(move || {
                    if let Some(anchor) = raw["aroundId"].as_str().filter(|anchor| !anchor.is_empty()) {
                        return crate::history::around(&path, &read_id, anchor);
                    }
                    crate::history::page(
                        &path, &read_id, raw["limit"].as_i64().unwrap_or(0),
                        raw["beforeTs"].as_f64(), raw["beforeId"].as_str().unwrap_or_default(),
                    ).map(Some)
                }).await;
                if conversation_for_socket(&socket, &devices, &conversations, &id).is_none() {
                    let _ = ack.send(&json!({"ok":false,"error":"Akses chat ditolak"}));
                    return;
                }
                let reply = match page {
                    Ok(Ok(Some(mut page))) => {
                        page["ok"] = json!(true);
                        page["conversation"] = conversation;
                        page
                    }
                    Ok(Ok(None)) => json!({"ok":false,"error":"Pesan tidak ditemukan di percakapan ini"}),
                    _ => json!({"ok": false, "error": "Riwayat chat tidak dapat dibaca"}),
                };
                let _ = ack.send(&reply);
            });
        }
    });
    let app = Router::new()
        .merge(device_admin::router(attachments.clone()))
        .merge(system_status::router(system))
        .merge(storage::router(attachments.clone(),snapshot.clone()))
        .merge(statuses::router(attachments.clone()))
        .merge(crate::profiles::router(profiles, move |headers, uri| {
            let token = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .or_else(|| uri.query()?.split('&').find_map(|part| part.strip_prefix("kc_token=")))?;
            let host = profile_admin.as_deref().is_some_and(|expected| valid_admin_token(token)
                && expected.bytes().zip(token.bytes()).fold(0, |diff,(a,b)| diff | (a^b)) == 0);
            if host {
                return profile_io.sockets().into_iter().find_map(|socket| {
                    if !socket.extensions.get::<SocketSession>()?.is_host { return None; }
                    socket.extensions.get::<Presence>()?.public["name"].as_str().map(str::to_owned)
                });
            }
            let mut registry = profile_devices.lock().expect("device registry");
            let id = registry.resolve_session(token, Instant::now())?.to_owned();
            registry.records.get(&id)?["name"].as_str().map(str::to_owned)
        }, move |profile| {
            for socket in profile_events.sockets() { let _ = socket.emit("profile-updated", &profile); }
            emit_presence(&profile_events, &profile_event_devices, true);
        }))
        .merge(crate::cohost::router(cohost_access.clone(), move |headers| {
            let token = headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")?;
            if identity_admin.as_deref().is_some_and(|expected| valid_admin_token(token)
                && expected.bytes().zip(token.bytes()).fold(0, |diff, (a,b)| diff | (a^b)) == 0) {
                return Some(("HOST".into(), true, String::new()));
            }
            let mut registry = identity_devices.lock().expect("device registry");
            let id = registry.resolve_session(token, Instant::now())?.to_owned();
            Some((registry.records.get(&id)?["name"].as_str()?.to_owned(), false, id))
        }, move || {
            let live: HashMap<String,serde_json::Value> = cohost_user_io.sockets().into_iter().filter_map(|socket| {
                let presence = socket.extensions.get::<Presence>()?;
                Some((presence.public["deviceId"].as_str()?.to_owned(),presence.public.clone()))
            }).collect();
            cohost_users.lock().expect("device registry").records.iter().map(|(id, record)| json!({
                "name":record["name"],"deviceId":id,"isHost":record["isHost"] == true || live.get(id).is_some_and(|row| row["isHost"] == true),
                "teamRole":record["teamRole"].as_str().unwrap_or("EDITOR"),
                "pc":record["pc"],"version":live.get(id).map(|row| row["version"].clone()).unwrap_or_else(||record["version"].clone()),
                "online":live.contains_key(id),"lastSeen":if live.contains_key(id){presence_time()}else{record["lastSeen"].as_u64().unwrap_or_default()},
                "avatarUrl":cohost_profiles.get(record["name"].as_str().unwrap_or_default())["avatarUrl"],
                "permissions":{"sendChat":true,"sendFiles":true,"designAI":true,"banned":false},
                "tagVideoEnabled":record["tagVideoEnabled"] == true,
            })).collect()
        }, move |name, scopes| {
            if name.is_empty() && scopes["refreshModeration"] == true {
                for socket in cohost_io.sockets() {
                    if let (Some(presence),Some(session)) = (socket.extensions.get::<Presence>(),socket.extensions.get::<SocketSession>()) {
                        let name = presence.public["name"].as_str().unwrap_or_default();
                        let mut policy = moderation_events_access.policy(name,presence.public["deviceId"].as_str().unwrap_or_default(),session.is_host);
                        policy["cohost"] = moderation_events_access.scopes(name,session.is_host);
                        let _ = socket.emit("moderation-updated",&json!({"deviceId":presence.public["deviceId"],"profileKey":crate::profiles::key(name),"policy":policy}));
                    }
                }
                emit_presence(&policy_events,&policy_devices,true);
                return;
            }
            let event = if name.is_empty() { scopes } else { json!({"name":name,"scopes":scopes}) };
            for socket in cohost_io.sockets() {
                let _ = socket.emit(if name.is_empty() { "job-desk-updated" } else { "cohost-updated" }, &event);
                if !name.is_empty() {
                    if let (Some(presence),Some(session)) = (socket.extensions.get::<Presence>(),socket.extensions.get::<SocketSession>()) {
                        let recipient = presence.public["name"].as_str().unwrap_or_default();
                        if recipient.trim().eq_ignore_ascii_case(name.trim()) {
                            let mut policy = moderation_events_access.policy(recipient,presence.public["deviceId"].as_str().unwrap_or_default(),session.is_host);
                            policy["cohost"] = moderation_events_access.scopes(recipient,session.is_host);
                            let _ = socket.emit("my-policy", &policy);
                        }
                    }
                }
            }
            emit_presence(&policy_events, &policy_devices, true);
        }))
        .merge(bledar_library.router())
        .merge(crate::diagnostics::router(diagnostics,move |headers| {
            let token = headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")?;
            if diagnostics_admin.as_deref().is_some_and(|expected| valid_admin_token(token)
                && expected.bytes().zip(token.bytes()).fold(0, |diff,(a,b)|diff|(a^b))==0) {
                return Some(json!({"isHost":true,"pc":std::env::var("COMPUTERNAME").unwrap_or_default()}));
            }
            let mut registry = diagnostics_devices.lock().expect("device registry");
            let device = registry.resolve_session(token,Instant::now())?.to_owned();
            let record = registry.records.get(&device)?;
            let name = record["name"].as_str()?;
            if diagnostics_access.policy(name,&device,false)["permissions"]["banned"] == true { return None; }
            let can_view = diagnostics_access.scopes(name,false)["viewDiagnostics"] == true;
            Some(json!({"name":name,"deviceId":device,"pc":record["pc"],"viewDiagnostics":can_view}))
        },move |result| emit_diagnostics_update(&diagnostics_io,result)))
        .merge(crate::uploads::router({
            let devices = upload_devices;
            move |headers: &HeaderMap| {
                let token = headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")?;
                let mut devices = devices.lock().expect("device registry");
                let host_token = upload_admin.as_deref().is_some_and(|expected| valid_admin_token(token)
                    && expected.bytes().zip(token.bytes()).fold(0, |diff,(a,b)| diff | (a^b)) == 0);
                let session_token = if host_token {
                    upload_io.sockets().into_iter().find_map(|socket| {
                        socket.extensions.get::<SocketSession>().filter(|session| session.is_host).map(|session| session.token.clone())
                    })?
                } else { token.to_owned() };
                let device = devices.resolve_session(&session_token, Instant::now())?.to_owned();
                let key = devices.sessions.get(&session_token)?.key_hash.clone()?;
                (devices.records.get(&device)?["keyHash"].as_str() == Some(key.as_str())).then_some((device, key))
            }
        }))
        .route("/peer-sync/manifest", get(|| async {
            Json(json!({"ok":true,"temporary":false,"messages":[],"conversations":[],"files":[]}))
        }))
        .route("/peer-sync/file/{stored}", get(reject_peer_handover))
        .route("/peer-sync/avatar/{stored}", get(reject_peer_handover))
        .route("/peer-sync/host-merge", post(reject_peer_handover))
        .route("/peer-sync/host-avatar/{stored}", post(reject_peer_handover))
        .route("/peer-sync/host-file/{stored}", post(reject_peer_handover))
        .route("/health", get({
            let gate=snapshot.clone();
            move || async move {
                let mut health=health;
                health["maintenance"]=json!(gate.try_read().is_err() || storage::pending());
                if storage::pending() {health["databaseWarning"]=json!("Pemulihan storage belum selesai");}
                Json(health)
            }
        }))
        .route("/client-update/status", get(crate::updates::status))
        .route("/client-update/download", get(crate::updates::download))
        .route("/rust-client-update/status", get(crate::updates::rust_status))
        .route("/rust-client-update/download", get(crate::updates::rust_download))
        .merge(update_routes)
        .route("/permissions", get({
            let attachments = attachments.clone();
            move |headers: HeaderMap, uri: Uri| async move { attachments.permissions(headers,uri) }
        }))
        .route("/asset-library/{kind}", get({
            let attachments = attachments.clone();
            move |RoutePath(kind):RoutePath<String>,headers:HeaderMap,uri:Uri| async move {
                let auth = attachments.permissions(headers,uri);
                if !auth.status().is_success() {return auth}
                match attachments.assets.list(&kind) {
                    Ok(value)=>Json(value).into_response(),
                    Err(error)=>(StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":error}))).into_response(),
                }
            }
        }))
        .route("/search", get({
            let attachments = attachments.clone();
            move |headers: HeaderMap, uri: Uri| attachments.clone().search(headers, uri)
        }))
        .route("/history", get({
            let attachments = attachments.clone();
            move |headers: HeaderMap, uri: Uri| attachments.clone().legacy_history(headers, uri)
        }))
        .route("/uploads/{stored}", get({
            let attachments = attachments.clone();
            move |RoutePath(stored): RoutePath<String>, headers: HeaderMap, uri: Uri| {
                attachments.clone().serve(stored, headers, uri, false)
            }
        }))
        .route("/download/{stored}", get(
            move |RoutePath(stored): RoutePath<String>, headers: HeaderMap, uri: Uri| {
                attachments.clone().serve(stored, headers, uri, true)
            }
        ))
        .layer(middleware::from_fn(move |request:Request,next:Next| {
            let gate=http_snapshot.clone();
            async move {
                let _snapshot=if !matches!(*request.method(),Method::GET|Method::HEAD|Method::OPTIONS)
                    && !request.uri().path().starts_with("/socket.io")
                    && !request.uri().path().starts_with("/peer-sync/") {
                    if request.uri().path().starts_with("/storage/") || request.uri().path()=="/admin/storage/migrate" {
                        return next.run(request).await;
                    }
                    let Ok(guard)=gate.try_read_owned() else {
                        return (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"ok":false,"retryable":true,
                            "error":"HOST sedang membuat backup. Coba lagi sebentar."}))).into_response();
                    };
                    if storage::pending() {
                        return (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"ok":false,"retryable":true,
                            "error":"Pemulihan storage belum selesai. Coba ulang operasi storage atau restart HOST."}))).into_response();
                    }
                    Some(guard)
                } else {None};
                next.run(request).await
            }
        }))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let access = route_access.clone();
            let devices = route_devices.clone();
            let admin = route_admin.clone();
            async move {
                let path = request.uri().path();
                let permission = if (path == "/upload" || path.starts_with("/upload/")) && path != "/upload/cancel" { Some("sendFiles") }
                    else if path.starts_with("/download/") { Some("downloadFiles") }
                    else { None };
                if request.method() != Method::OPTIONS {
                    if let Some(permission) = permission {
                        let token = request.headers().get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())
                            .and_then(|value| value.strip_prefix("Bearer "))
                            .or_else(|| request.uri().query()?.split('&').find_map(|part| part.strip_prefix("kc_token=")));
                        if let Some(token) = token {
                            let host = admin.as_deref().is_some_and(|expected| valid_admin_token(token)
                                && expected.bytes().zip(token.bytes()).fold(0, |diff,(a,b)| diff | (a^b)) == 0);
                            if !host {
                                let member = {
                                    let mut registry = devices.lock().expect("device registry");
                                    registry.resolve_session(token, Instant::now()).map(str::to_owned)
                                        .and_then(|id| registry.records.get(&id)?["name"].as_str().map(|name| (id,name.to_owned())))
                                };
                                if let Some((device,name)) = member {
                                    let policy = access.policy(&name, &device, false)["permissions"].clone();
                                    if policy["banned"] == true || policy[permission] == false {
                                        return (StatusCode::FORBIDDEN, Json(json!({
                                            "ok":false,"code":"FEATURE_DENIED","error":"Akses fitur dibatasi oleh HOST."
                                        }))).into_response();
                                    }
                                }
                            }
                        }
                    }
                }
                next.run(request).await
            }
        }))
        .layer(middleware::from_fn(local_renderer_origin));
    let reply = json!({"service": "CHATX_COMMUNITY", "ip": address, "port": port}).to_string();
    ready(local_port, udp.local_addr()?.port(), lan_address)?;
    let lan_app = app.clone().layer(socket_layer.clone());
    let app = app.layer(socket_layer);
    tokio::select! {
        result = async {
            match local_tcp {
                Some(listener) => axum::serve(listener, app).await,
                None => std::future::pending().await,
            }
        } => result,
        result = async {
            match lan_tcp {
                Some(listener) => axum::serve(listener, lan_app).await,
                None => std::future::pending().await,
            }
        } => result,
        result = runtime_sampler.sample_scheduler() => result,
        result = async {
            loop {
                let delay = {
                    let _ordered = game_tick_order.lock().await;
                    game_tick.timer_delay()
                };
                let Some(delay) = delay else {
                    game_timer_wakeup.notified().await;
                    continue;
                };
                tokio::select! {
                    _ = game_timer_wakeup.notified() => continue,
                    _ = tokio::time::sleep(delay) => {}
                }
                let _snapshot=snapshot.read().await;
                let _ordered = game_tick_order.lock().await;
                let server = game_tick.clone();
                match tokio::task::spawn_blocking(move || server.tick()).await {
                    Ok(Ok(updates)) => emit_game_updates(&game_tick_io, &game_tick_devices, updates),
                    failure => {
                        let error = match failure {
                            Ok(Err(error)) => error,
                            Err(error) => error.to_string(),
                            _ => unreachable!(),
                        };
                        eprintln!("Timer game gagal: {error}");
                        drop(_ordered);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        } => result,
        result = async {
            let mut buffer = [0; 4096];
            loop {
                let (length, peer) = udp.recv_from(&mut buffer).await?;
                let private_peer = match peer.ip() {
                    std::net::IpAddr::V4(ip) => ip.is_private(),
                    _ => false,
                };
                if (!strict_auth || private_peer) && &buffer[..length] == b"CHATX_COMMUNITY_DISCOVER" {
                    let sender = discovery_reply.as_ref().unwrap_or(&udp);
                    let _ = sender.send_to(reply.as_bytes(), peer).await;
                }
            }
        } => result,
    }
}


type Records = serde_json::Map<String, serde_json::Value>;
type ConversationDb = RwLock<Vec<serde_json::Value>>;

async fn reject_peer_handover() -> Response {
    (StatusCode::FORBIDDEN, Json(json!({"ok":false,"error":"HOST handover token ditolak"}))).into_response()
}

async fn local_renderer_origin(request: Request, next: Next) -> Response {
    let origin = request.headers().get(header::ORIGIN).filter(|value| {
        let Ok(origin) = value.to_str() else { return false };
        if matches!(origin, "null" | "http://tauri.localhost") {
            return true;
        }
        origin.parse::<Uri>().is_ok_and(|uri| {
            uri.scheme_str() == Some("http") && matches!(uri.host(), Some("localhost" | "127.0.0.1"))
                && uri.path() == "/" && uri.query().is_none()
        })
    }).cloned();
    let mut response = if request.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    if let Some(origin) = origin {
        let headers = response.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Authorization, Content-Type, X-Upload-Offset, Range, X-KC-Bridge-Token"));
        headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, HEAD, POST, PUT, DELETE, OPTIONS"));
    }
    response.headers_mut().append(header::VARY, HeaderValue::from_static("Origin"));
    response
}

#[derive(Clone)]
struct Attachments {
    devices: Arc<Mutex<DeviceRegistry>>,
    conversations: Arc<ConversationDb>,
    history: PathBuf,
    admin: Option<String>,
    io: SocketIo,
    access: Arc<crate::cohost::Access>,
    assets: Arc<crate::asset_library::Library>,
}

impl Attachments {
    async fn legacy_history(self, headers: HeaderMap, uri: Uri) -> Response {
        if let Err(status) = self.search_identity(&headers, &uri, "general") {
            return status.into_response();
        }
        let path = self.history.clone();
        let result = tokio::task::spawn_blocking(move || crate::history::legacy_general(&path)).await;
        if let Err(status) = self.search_identity(&headers, &uri, "general") {
            return status.into_response();
        }
        match result {
            Ok(Ok(items)) => Json(items).into_response(),
            _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
    fn permissions(&self, headers: HeaderMap, uri: Uri) -> Response {
        let token = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| uri.query()?.split('&').find_map(|part| part.strip_prefix("kc_token=")));
        let Some(token) = token else {return (StatusCode::UNAUTHORIZED,Json(json!({"ok":false,"error":"Sesi berakhir."}))).into_response()};
        let host = self.admin.as_deref().is_some_and(|expected| valid_admin_token(token)
            && expected.bytes().zip(token.bytes()).fold(0, |diff,(a,b)| diff | (a^b)) == 0);
        let identity = if host {
            self.io.sockets().into_iter().find_map(|socket| {
                if !socket.extensions.get::<SocketSession>()?.is_host {return None}
                let presence = socket.extensions.get::<Presence>()?;
                Some((presence.public["deviceId"].as_str()?.to_owned(),presence.public["name"].as_str()?.to_owned()))
            })
        } else {
            let mut registry = self.devices.lock().expect("device registry");
            registry.resolve_session(token,Instant::now()).map(str::to_owned).and_then(|device| {
                registry.records.get(&device)?["name"].as_str().map(|name| (device,name.to_owned()))
            })
        };
        let Some((device,name)) = identity else {return (StatusCode::UNAUTHORIZED,Json(json!({"ok":false,"error":"Sesi berakhir."}))).into_response()};
        let mut policy = self.access.policy(&name,&device,host);
        policy["cohost"] = self.access.scopes(&name,host);
        Json(json!({"ok":true,"profileName":name,"policy":policy})).into_response()
    }
    fn search_identity(&self, headers: &HeaderMap, uri: &Uri, conversation: &str) -> Result<(), StatusCode> {
        let token = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| uri.query()?.split('&').find_map(|part| part.strip_prefix("kc_token=")));
        let Some(token) = token else { return Err(StatusCode::UNAUTHORIZED) };
        let host = self.admin.as_deref().is_some_and(|expected| valid_admin_token(token)
            && expected.bytes().zip(token.bytes()).fold(0, |diff,(a,b)| diff | (a^b)) == 0);
        if host {
            return self.io.sockets().iter().any(|socket| socket.extensions.get::<SocketSession>()
                .is_some_and(|session| session.is_host) && conversation_for_socket(socket, &self.devices, &self.conversations, conversation).is_some())
                .then_some(()).ok_or(StatusCode::FORBIDDEN);
        }
        let mut registry = self.devices.lock().expect("device registry");
        let Some(device) = registry.resolve_session(token, Instant::now()).map(str::to_owned) else { return Err(StatusCode::UNAUTHORIZED) };
        let Some(record) = registry.records.get(&device) else { return Err(StatusCode::UNAUTHORIZED) };
        let verified = registry.sessions.get(token).and_then(|session| session.key_hash.as_deref())
            .is_some_and(|hash| record["keyHash"].as_str() == Some(hash));
        let Some(name) = record["name"].as_str() else { return Err(StatusCode::UNAUTHORIZED) };
        if self.access.policy(name, &device, false)["permissions"]["banned"] == true { return Err(StatusCode::FORBIDDEN) }
        (conversation == "general" || (verified && record["labPrivateHistoryApproved"] == true))
            .then_some(()).filter(|_| accessible_conversation(&self.conversations, conversation, name, false).is_some())
            .ok_or(StatusCode::FORBIDDEN)
    }

    async fn search(self, headers: HeaderMap, uri: Uri) -> Response {
        if uri.to_string().len() > 16384 { return StatusCode::URI_TOO_LONG.into_response() }
        let Ok(url) = reqwest::Url::parse(&format!("http://localhost{uri}")) else { return StatusCode::BAD_REQUEST.into_response() };
        let parameters: HashMap<String, String> = url.query_pairs().map(|(key,value)| (key.into_owned(),value.into_owned())).collect();
        let value = |key: &str| parameters.get(key).map(String::as_str).unwrap_or_default();
        let conversation = parameters.get("conversationId").cloned().unwrap_or_else(|| "general".into());
        if let Err(status) = self.search_identity(&headers, &uri, &conversation) {
            return (status, Json(json!({"ok":false,"error":"Akses chat ditolak / sesi berakhir"}))).into_response();
        }
        let number = |key: &str| value(key).parse::<f64>().ok().filter(|value| value.is_finite()).unwrap_or_default();
        let filter = crate::history::Search {
            text: value("q").trim().to_lowercase().chars().take(200).collect(),
            kind: value("type").into(), sender: value("sender").trim().to_lowercase(),
            tag: value("tag").trim().to_lowercase(), from: number("from"), to: number("to"),
        };
        if filter.empty() { return Json(json!({"ok":true,"results":[]})).into_response() }
        let path = self.history.clone();
        let id = conversation.clone();
        let result = tokio::task::spawn_blocking(move || crate::history::search(&path, &id, &filter)).await;
        if let Err(status) = self.search_identity(&headers, &uri, &conversation) {
            return (status, Json(json!({"ok":false,"error":"Akses chat ditolak / sesi berakhir"}))).into_response();
        }
        match result {
            Ok(Ok(results)) => Json(json!({"ok":true,"results":results})).into_response(),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":"Pencarian pesan gagal dibaca dari database."}))).into_response(),
        }
    }

    async fn serve(self, stored: String, headers: HeaderMap, uri: Uri, download: bool) -> Response {
        let token = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .or_else(|| uri.query()?.split('&').find_map(|part| part.strip_prefix("kc_token=")));
        let mapped_token = token.and_then(|token| {
            let host = self.admin.as_deref().is_some_and(|expected| valid_admin_token(token)
                && expected.bytes().zip(token.bytes()).fold(0, |diff,(a,b)| diff | (a^b)) == 0);
            if host {
                self.io.sockets().into_iter().find_map(|socket| {
                    socket.extensions.get::<SocketSession>().filter(|session| session.is_host).map(|session| session.token.clone())
                })
            } else { Some(token.to_owned()) }
        });
        let token = mapped_token.as_deref();
        let host_session = token.is_some_and(|token| self.io.sockets().iter().any(|socket| {
            socket.extensions.get::<SocketSession>()
                .is_some_and(|session| session.is_host && session.token == token)
        }));
        let Some(device) = token.and_then(|token| {
            self.devices.lock().expect("device registry").resolve_session(token, Instant::now()).map(str::to_owned)
        }) else {
            return (StatusCode::UNAUTHORIZED, Json(json!({"ok":false,"error":"Perangkat belum disetujui / sesi berakhir"}))).into_response();
        };
        let valid = stored.len() <= 80 && stored.as_bytes().get(..64).is_some_and(|hash| hash.iter().all(u8::is_ascii_hexdigit))
            && stored.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            && !stored.contains("..") && !stored.ends_with('.');
        if !valid {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let index = match tokio::fs::read("file-index.json").await {
            Ok(bytes) => serde_json::from_slice::<Records>(&bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Records::new()),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let Ok(index) = index else { return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
        let file = index.values().find(|entry| entry["stored"].as_str() == Some(stored.as_str()));
        let owns = file.and_then(|entry| entry["owners"].as_array()).is_some_and(|owners| {
            owners.iter().any(|owner| owner.as_str() == Some(device.as_str()))
        });
        let referenced = {
            let path = self.history.clone();
            let stored = stored.clone();
            tokio::task::spawn_blocking(move || crate::history::attachment_conversations(&path, &stored)).await
        };
        let Ok(Ok(referenced)) = referenced else { return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
        let authorized = {
            let mut devices = self.devices.lock().expect("device registry");
            let current = token.filter(|token| devices.resolve_session(token, Instant::now()).is_some_and(|current| current == device));
            current.and_then(|token| devices.sessions.get(token)).is_some_and(|session| {
                devices.records.get(&device).is_some_and(|record| {
                    let verified = record["labPrivateHistoryApproved"] == true
                        && session.key_hash.as_deref().is_some_and(|hash| record["keyHash"].as_str() == Some(hash));
                    let uploaded_owner = session.key_hash.as_deref().is_some_and(|hash| {
                        record["keyHash"].as_str() == Some(hash)
                            && file.and_then(serde_json::Value::as_object)
                                .is_some_and(|entry| crate::uploads::owns_attachment(entry, &device, hash))
                    });
                    host_session || uploaded_owner || (owns && verified) || referenced.iter().any(|id| {
                        id == "general" || (verified
                            && record["name"].as_str().is_some_and(|name| accessible_conversation(&self.conversations, id, name, false).is_some()))
                    })
                })
            })
        };
        if !authorized {
            return (StatusCode::FORBIDDEN, Json(json!({"ok":false,"error":"Akses lampiran ditolak"}))).into_response();
        }
        let Ok(upload_dir) = storage_location::upload_dir(Path::new(".")) else { return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
        let Ok(root) = tokio::fs::canonicalize(upload_dir).await else { return StatusCode::NOT_FOUND.into_response() };
        let Ok(path) = tokio::fs::canonicalize(root.join(&stored)).await else { return StatusCode::NOT_FOUND.into_response() };
        if path.parent() != Some(root.as_path()) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let Ok(mut handle) = tokio::fs::File::open(path).await else { return StatusCode::NOT_FOUND.into_response() };
        let Ok(metadata) = handle.metadata().await else { return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
        if !metadata.is_file() { return StatusCode::NOT_FOUND.into_response() }
        let mime = match Path::new(&stored).extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
            "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif",
            "webp" => "image/webp", "mp4" => "video/mp4", "webm" => "video/webm",
            _ => "application/octet-stream",
        };
        let name = file.and_then(|entry| entry["name"].as_str()).unwrap_or(&stored);
        let encoded: String = name.as_bytes().iter().map(|byte| format!("%{byte:02X}")).collect();
        let disposition = if download || mime == "application/octet-stream" {
            format!("attachment; filename*=UTF-8''{encoded}")
        } else {
            "inline".into()
        };
        let size = metadata.len();
        let range = match headers.get(header::RANGE) {
            Some(value) => match value.to_str().ok().and_then(|value| byte_range(value, size)) {
                Some(range) => Some(range),
                None => return (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{size}"))]).into_response(),
            },
            None => None,
        };
        let (status, length) = match range {
            Some((start, end)) => {
                if handle.seek(io::SeekFrom::Start(start)).await.is_err() {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                (StatusCode::PARTIAL_CONTENT, end - start + 1)
            }
            None => (StatusCode::OK, size),
        };
        let mut response = (status, [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::CONTENT_LENGTH, length.to_string()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            (header::ACCEPT_RANGES, "bytes".into()),
            (header::CACHE_CONTROL, "private, no-store".into()),
        ], Body::from_stream(tokio_util::io::ReaderStream::new(handle.take(length)))).into_response();
        if let Some((start, end)) = range {
            response.headers_mut().insert(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}").parse().expect("valid byte range"));
        }
        response
    }
}

pub(crate) fn byte_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let last = size.checked_sub(1)?;
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.checked_sub(1)?;
        return Some((last.saturating_sub(suffix), last));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() { last } else { end.parse::<u64>().ok()?.min(last) };
    (start <= end).then_some((start, end))
}

#[derive(Clone)]
struct SocketSession {
    token: String,
    key_hash: String,
    is_host: bool,
}

fn valid_admin_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_admin(raw: &serde_json::Value, token: Option<&str>) -> bool {
    let Some((expected, supplied)) = token.zip(raw["adminToken"].as_str()).filter(|(a, b)| valid_admin_token(a) && valid_admin_token(b)) else {
        return false;
    };
    expected.bytes().zip(supplied.bytes()).fold(0, |diff, (a, b)| diff | (a ^ b)) == 0
}

#[derive(Clone)]
struct Presence {
    profile: String,
    connected: Instant,
    public: serde_json::Value,
    activity: serde_json::Value,
}

impl Presence {
    fn from_login(reply: &serde_json::Value, raw: &serde_json::Value, is_host: bool) -> Self {
        let name = reply["name"].as_str().unwrap_or_default();
        let role = raw["teamRole"].as_str().filter(|role| !role.is_empty() && role.chars().count() <= 40).unwrap_or("EDITOR");
        let mut value = Self {
            profile: name.trim().to_lowercase(),
            connected: Instant::now(),
            activity: serde_json::Value::Null,
            public: json!({
                "name":name,"displayName":name,"pc":clip_presence(&raw["pc"],80),
                "deviceId":clip_presence(&raw["deviceId"],120),"version":clip_presence(&raw["version"],40),
                "isHost":is_host,"teamRole":role,"connectedAt":presence_time(),"avatarUrl":null,
                "cohost":false,"activityAccessOverride":false,
                "workTagPrefs":{
                    "print-test":raw["workTagPrefs"]["print-test"] != false,
                    "design-revision":raw["workTagPrefs"]["design-revision"] != false,
                    "design-request":raw["workTagPrefs"]["design-request"] != false
                }
            }),
        };
        value.update_activity(&json!({"enabled": raw["activitySharing"], "activity": raw["activity"]}));
        value
    }

    fn update_activity(&mut self, raw: &serde_json::Value) {
        let enabled = raw["enabled"] != false;
        let activity = &raw["activity"];
        let kind = match activity["kind"].as_str() {
            Some(kind @ ("music" | "video" | "app" | "chat" | "idle")) => kind,
            _ => "chat",
        };
        self.public["activitySharing"] = json!(enabled);
        self.public["activityUpdatedAt"] = json!(presence_time());
        self.activity = {
            let app = clip_presence(&activity["app"], 80);
            let title = clip_presence(&activity["title"], 220);
            let state = clip_presence(&activity["state"], 80);
            json!({
                "kind":kind,
                "app":if app.is_empty() { "ChatX" } else { &app },
                "title":if title.is_empty() { "Online di ChatX" } else { &title },
                "state":if state.is_empty() { "active" } else { &state },
                "artist":clip_presence(&activity["artist"],140),"source":clip_presence(&activity["source"],120)
            })
        };
        self.public["activity"] = if enabled {self.activity.clone()} else {serde_json::Value::Null};
    }
}

fn clip_presence(value: &serde_json::Value, limit: usize) -> String {
    value.as_str().unwrap_or_default().trim().chars()
        .map(|character| if character.is_control() { ' ' } else { character })
        .take(limit).collect()
}

fn presence_time() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn reply_snapshot(message: &serde_json::Value, conversation: &str) -> serde_json::Value {
    if message["deleted"] == true || message["conversationId"].as_str() != Some(conversation) {
        return serde_json::Value::Null;
    }
    let file = &message["file"];
    let extension = file["name"].as_str().and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()));
    let is_image = file["mimetype"].as_str().is_some_and(|mime| mime.to_ascii_lowercase().starts_with("image/"))
        || matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "svg"));
    json!({
        "id":message["id"],"sender":message["sender"],
        "text":message["text"].as_str().unwrap_or_default().chars().take(180).collect::<String>(),
        "fileName":file["name"],"isImage":is_image,"imageUrl":file["url"],
    })
}

async fn change_chat_message(
    socket: &SocketRef, devices: &Mutex<DeviceRegistry>, conversations: &ConversationDb,
    path: PathBuf, raw: serde_json::Value, delete: bool,
) -> serde_json::Value {
    let actor = {
        let mut registry = devices.lock().expect("device registry");
        socket.extensions.get::<SocketSession>().and_then(|session| {
            let device = registry.resolve_session(&session.token, Instant::now())?.to_owned();
            let name = registry.records.get(&device)?["name"].as_str()?.to_owned();
            Some((name, session.is_host))
        })
    };
    let Some((name, is_host)) = actor else { return json!({"ok":false,"error":"Belum login"}) };
    if delete && !is_host {
        return json!({"ok":false,"error":"Hapus untuk semua orang hanya tersedia untuk HOST"});
    }
    if delete && raw["scope"] != "everyone" {
        return json!({"ok":false,"error":"Scope hapus tidak valid"});
    }
    let text: String = raw["text"].as_str().unwrap_or_default().trim().chars().take(4000).collect();
    if !delete && text.is_empty() {
        return json!({"ok":false,"error":"Pesan tidak boleh kosong"});
    }
    let Some(id) = raw["id"].as_str().filter(|id| !id.is_empty()).map(str::to_owned) else {
        return json!({"ok":false,"error":"Pesan tidak ditemukan"});
    };
    let lookup = id.clone();
    let read_path = path.clone();
    let original = tokio::task::spawn_blocking(move || crate::history::message(&read_path, &lookup)).await;
    let Ok(Ok(Some(original))) = original else { return json!({"ok":false,"error":"Pesan tidak ditemukan"}) };
    let conversation = original["conversationId"].as_str().unwrap_or_default();
    if !is_host && conversation_for_socket(socket, devices, conversations, conversation).is_none() {
        return json!({"ok":false,"error":"Pesan tidak ditemukan"});
    }
    let conversation_id = conversation.to_owned();
    let now = presence_time() as i64;
    let result = tokio::task::spawn_blocking(move || {
        use crate::history::MessageChange;
        let change = if delete {
            MessageChange::Delete { actor: &name }
        } else {
            MessageChange::Edit { text: &text, actor: &name, is_host }
        };
        crate::history::change_message(&path, &id, &conversation_id, change, now)
    }).await;
    let item = match result {
        Ok(Ok(Ok(item))) => item,
        Ok(Ok(Err(error))) => return json!({"ok":false,"error":error}),
        _ => return json!({"ok":false,"retryable":true,"error":if delete { "Hapus belum tersimpan" } else { "Edit belum tersimpan" }}),
    };
    let payload = json!({"id":conversation,"item":item});
    let event = if delete { "message-deleted" } else { "message-edited" };
    for recipient in std::iter::once(socket.clone()).chain(socket.broadcast().sockets()) {
        if conversation_member_for_socket(&recipient, devices, conversations, conversation) {
            let _ = recipient.emit(event, &payload);
        }
    }
    if delete { json!({"ok":true,"deletedAt":now}) } else { json!({"ok":true,"editedAt":now}) }
}

fn emit_presence(io: &SocketIo, devices: &Mutex<DeviceRegistry>, directory_changed: bool) {
    let mut registry = devices.lock().expect("device registry");
    let mut recipients = Vec::new();
    let mut profiles = std::collections::BTreeMap::<String, Presence>::new();
    for socket in io.sockets() {
        let Some(session) = socket.extensions.get::<SocketSession>() else { continue };
        if registry.resolve_session(&session.token, Instant::now()).is_none() { continue }
        let Some(presence) = socket.extensions.get::<Presence>() else { continue };
        recipients.push(socket);
        if profiles.get(&presence.profile).is_none_or(|current| presence.connected > current.connected) {
            profiles.insert(presence.profile.clone(), presence);
        }
    }
    let directory = directory_changed.then(|| registry.records.iter().filter_map(|(id, record)| {
        if record["revoked"] == true { return None; }
        let name = record["name"].as_str()?;
        let profile = profiles.get(&name.trim().to_lowercase());
        let team_role = profile.and_then(|profile| profile.public["teamRole"].as_str())
            .or_else(|| record["teamRole"].as_str()).unwrap_or("EDITOR");
        Some(json!({
            "name":name,"displayName":name,"deviceId":id,"pc":record["pc"],"online":profile.is_some(),
            "isHost":profile.is_some_and(|profile| profile.public["isHost"] == true),
            "cohost":false,"teamRole":team_role,"avatarUrl":null,
        }))
    }).collect::<Vec<_>>());
    let access = recipients.iter().find_map(|socket| socket.extensions.get::<Arc<crate::cohost::Access>>());
    let saved_profiles = recipients.iter().find_map(|socket| socket.extensions.get::<Arc<crate::profiles::Profiles>>());
    let decorate = |mut row: serde_json::Value| {
        if let Some(profiles) = &saved_profiles {
            let name = row["name"].as_str().unwrap_or_default().to_owned();
            let profile = profiles.get(&name);
            row["displayName"] = json!(profile["displayName"].as_str().unwrap_or(&name));
            row["avatarUrl"] = profile["avatarUrl"].clone();
        }
        if let Some(access) = &access {
            let host = row["isHost"] == true;
            if let Some(role) = access.assigned_job(row["name"].as_str().unwrap_or_default(), row["deviceId"].as_str().unwrap_or_default()) {
                row["teamRole"] = json!(role);
            }
            let scopes = access.scopes(row["name"].as_str().unwrap_or_default(), host);
            row["cohost"] = json!(!host && scopes.as_object().is_some_and(|scopes| scopes.values().any(|scope| scope == true)));
            row["policy"] = access.policy(row["name"].as_str().unwrap_or_default(), row["deviceId"].as_str().unwrap_or_default(), host);
            row["policy"]["cohost"] = scopes;
        }
        row
    };
    let subjects = profiles.into_values().collect::<Vec<_>>();
    let directory = directory.map(|rows| rows.into_iter().map(decorate).collect::<Vec<_>>());
    for recipient in recipients {
        let viewer=recipient.extensions.get::<Presence>();
        let session=recipient.extensions.get::<SocketSession>();
        let payload=subjects.iter().map(|subject| {
            let mut row=decorate(subject.public.clone());
            if row["activitySharing"]==false {
                let override_access=match (&access,&viewer,&session) {
                    (Some(access),Some(viewer),Some(session))=>access.activity_override(
                        row["deviceId"].as_str().unwrap_or_default(),viewer.public["deviceId"].as_str().unwrap_or_default(),
                        viewer.public["name"].as_str().unwrap_or_default(),session.is_host),
                    _=>false,
                };
                if override_access {
                    row["activity"]=subject.activity.clone();
                    row["activitySharing"]=json!(true);
                    row["activityAccessOverride"]=json!(true);
                }
            }
            row
        }).collect::<Vec<_>>();
        if directory_changed {
            if let (Some(access),Some(presence),Some(session)) = (&access, recipient.extensions.get::<Presence>(), recipient.extensions.get::<SocketSession>()) {
                let name = presence.public["name"].as_str().unwrap_or_default();
                let mut policy = access.policy(name, presence.public["deviceId"].as_str().unwrap_or_default(), session.is_host);
                policy["cohost"] = access.scopes(name, session.is_host);
                let _ = recipient.emit("my-policy", &policy);
            }
        }
        let _ = recipient.emit("presence", &payload);
        if let Some(directory) = &directory {
            let _ = recipient.emit("editor-directory", directory);
        }
    }
}

async fn conversation_list(
    socket: &SocketRef, devices: &Mutex<DeviceRegistry>, conversations: &ConversationDb, path: PathBuf,
) -> Result<Vec<serde_json::Value>, &'static str> {
    conversation_list_scope(socket, devices, conversations, path, false).await
}

async fn conversation_list_scope(
    socket: &SocketRef, devices: &Mutex<DeviceRegistry>, conversations: &ConversationDb, path: PathBuf, all: bool,
) -> Result<Vec<serde_json::Value>, &'static str> {
    let (_, name, general) = conversation_for_socket(socket, devices, conversations, "general").ok_or("Belum login")?;
    let ids = conversations.read().expect("conversations").iter().filter_map(|entry| {
        entry["id"].as_str().filter(|id| *id != "general").map(str::to_owned)
    }).collect::<Vec<_>>();
    let items = std::iter::once(general).chain(ids.iter().filter_map(|id| {
        let (_, _, mut conversation) = conversation_for_socket(socket, devices, conversations, id)?;
        let member = names(&conversation["members"]).any(|member| member.trim().eq_ignore_ascii_case(name.trim()));
        if all { conversation["hostListed"] = json!(member); }
        (all || member).then_some(conversation)
    })).filter(|entry| !all || entry["type"] != "general").collect::<Vec<_>>();
    let result = tokio::task::spawn_blocking(move || -> rusqlite::Result<_> {
        items.into_iter().map(|mut entry| {
            let id = entry["id"].as_str().unwrap_or_default();
            let last = crate::history::latest(&path, id)?.unwrap_or_default();
            let since = crate::history::read_marker(&path, id, &name)?;
            let unread = crate::history::unread(&path, id, &name, since)?;
            let text = if last["deleted"] == true {
                "Pesan dihapus"
            } else {
                last["text"].as_str().filter(|text| !text.is_empty()).or_else(|| last["file"]["name"].as_str()).unwrap_or_default()
            };
            entry["lastTs"] = json!(last["ts"].as_i64().or_else(|| entry["createdAt"].as_i64()).unwrap_or(0));
            entry["lastText"] = json!(text);
            if last["deleted"] != true && last["text"].as_str().is_some_and(|text| !text.is_empty()) {
                entry["lastMentionRecipients"] = last["mentionRecipients"].clone();
            }
            entry["lastSender"] = json!(last["sender"].as_str().unwrap_or_default());
            entry["lastFile"] = json!(last["file"].is_object());
            entry["unread"] = json!(unread);
            Ok(entry)
        }).collect::<rusqlite::Result<Vec<_>>>()
    }).await;
    match result {
        Ok(Ok(items)) => Ok(items),
        _ => Err("Daftar chat tidak dapat dibaca"),
    }
}

fn emit_diagnostics_update(io: &SocketIo, result: &serde_json::Value) {
    let update = json!({"signature":result["signature"],"status":result["status"],"count":result["count"]});
    for socket in io.sockets() {
        if socket.extensions.get::<SocketSession>().is_some() { let _ = socket.emit("diagnostics-updated",&update); }
    }
}

fn game_identity(socket: &SocketRef, devices: &Mutex<DeviceRegistry>) -> Option<String> {
    let session = socket.extensions.get::<SocketSession>()?;
    let (device, name) = {
        let mut registry = devices.lock().expect("device registry");
        let device = registry.resolve_session(&session.token, Instant::now())?.to_owned();
        let name = registry.records.get(&device)?["name"].as_str()?.to_owned();
        (device, name)
    };
    let access = socket.extensions.get::<Arc<crate::cohost::Access>>()?;
    (access.policy(&name, &device, session.is_host)["permissions"]["banned"] != true).then_some(name)
}

fn gallery_device(socket: &SocketRef, devices: &Mutex<DeviceRegistry>) -> Option<crate::gallery::Device> {
    let name = game_identity(socket, devices)?;
    let session = socket.extensions.get::<SocketSession>()?;
    let presence = socket.extensions.get::<Presence>()?;
    let id = devices.lock().expect("device registry").resolve_session(&session.token, Instant::now())?.to_owned();
    Some(crate::gallery::Device { id, name, pc: presence.public["pc"].as_str().unwrap_or_default().to_owned() })
}

fn gallery_legacy_allowed(socket:&SocketRef,devices:&Mutex<DeviceRegistry>,id:&str) -> bool {
    let Some(session)=socket.extensions.get::<SocketSession>() else {return false;};
    if session.is_host {return false;}
    let registry=devices.lock().expect("device registry");
    // A keyless client may update only an existing keyless registry entry.
    registry.sessions.get(&session.token).is_some_and(|session|session.key_hash.is_none()) &&
        registry.records.get(id).is_some_and(|record|record["keyHash"].as_str().is_none_or(str::is_empty))
}

fn gallery_can_write(socket:&SocketRef,devices:&Mutex<DeviceRegistry>,id:&str) -> bool {
    let Some(session)=socket.extensions.get::<SocketSession>() else {return false;};
    if session.is_host {return true;}
    let registry=devices.lock().expect("device registry");
    let Some(stored)=registry.records.get(id) else {return false;};
    let Some(current)=registry.sessions.get(&session.token) else {return false;};
    current.device_id==id && current.key_hash.as_deref()==stored["keyHash"].as_str().filter(|hash|!hash.is_empty())
}

fn gallery_source(socket:&SocketRef,devices:&Mutex<DeviceRegistry>) -> Option<crate::gallery::Device> {
    let device=gallery_device(socket,devices)?;
    gallery_can_write(socket,devices,&device.id).then_some(device)
}

fn gallery_rooms(socket: &SocketRef, devices: &Mutex<DeviceRegistry>, conversations: &ConversationDb) -> serde_json::Value {
    let ids: Vec<_> = conversations.read().expect("conversations").iter()
        .filter_map(|row| row["id"].as_str().map(str::to_owned)).collect();
    let mut rooms = serde_json::Map::new();
    for id in std::iter::once("general").chain(ids.iter().map(String::as_str)) {
        if let Some((_,_,room)) = conversation_for_socket(socket, devices, conversations,id) {
            rooms.insert(id.to_owned(), if room["type"] == "dm" { json!("DM") } else { room["name"].clone() });
        }
    }
    serde_json::Value::Object(rooms)
}

fn emit_game_updates(io: &SocketIo, devices: &Mutex<DeviceRegistry>, updates: Vec<crate::games::Event>) {
    if updates.is_empty() { return; }
    let peers: Vec<_> = io.sockets().into_iter().filter_map(|socket| {
        let name = game_identity(&socket, devices)?;
        Some((crate::profiles::key(&name), socket))
    }).collect();
    for update in updates {
        let targets: std::collections::HashSet<_> = update.names.iter().map(|name| crate::profiles::key(name)).collect();
        for (name, socket) in &peers {
            if targets.contains(name) { let _ = socket.emit(update.event, &update.payload); }
        }
    }
}

fn conversation_for_socket(
    socket: &SocketRef, devices: &Mutex<DeviceRegistry>, conversations: &ConversationDb, id: &str,
) -> Option<(String, String, serde_json::Value)> {
    let mut registry = devices.lock().expect("device registry");
    let session = socket.extensions.get::<SocketSession>()?;
    let device = registry.resolve_session(&session.token, Instant::now())?.to_owned();
    let record = registry.records.get(&device)?;
    if id != "general" && !session.is_host && (record["labPrivateHistoryApproved"] != true
        || record["keyHash"].as_str() != Some(session.key_hash.as_str())) {
        return None;
    }
    let name = record["name"].as_str()?.to_owned();
    let conversation = accessible_conversation(conversations, id, &name, session.is_host)?;
    Some((device, name, conversation))
}

fn conversation_member_for_socket(
    socket: &SocketRef, devices: &Mutex<DeviceRegistry>, conversations: &ConversationDb, id: &str,
) -> bool {
    conversation_for_socket(socket, devices, conversations, id).is_some_and(|(_, name, conversation)| {
        conversation["type"] == "general" || names(&conversation["members"])
            .any(|member| member.trim().eq_ignore_ascii_case(name.trim()))
    })
}

fn load_conversations(path: &Path) -> io::Result<Vec<serde_json::Value>> {
    match std::fs::File::open(path) {
        Ok(file) => serde_json::from_reader(file).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn conversation_creator(socket: &SocketRef, devices: &Mutex<DeviceRegistry>) -> Option<(String, Vec<String>)> {
    let session = socket.extensions.get::<SocketSession>()?;
    let mut registry = devices.lock().expect("device registry");
    let device = registry.resolve_session(&session.token, Instant::now())?.to_owned();
    let record = registry.records.get(&device)?;
    if !session.is_host && (record["labPrivateHistoryApproved"] != true || record["keyHash"].as_str() != Some(session.key_hash.as_str())) {
        return None;
    }
    let actor = record["name"].as_str()?.to_owned();
    let names = registry.records.values().filter(|record| record["revoked"] != true && record["labPrivateHistoryApproved"] == true)
        .filter_map(|record| record["name"].as_str().map(str::to_owned)).collect();
    Some((actor, names))
}

fn create_conversation(
    conversations: &ConversationDb, actor: &str, known: &[String], raw: &serde_json::Value, group: bool,
) -> Result<serde_json::Value, String> {
    let profile = |name: &str| name.trim().to_lowercase();
    let known_name = |name: &str| known.iter().find(|candidate| profile(candidate) == profile(name));
    let name: String = raw["name"].as_str().unwrap_or_default().trim().chars().take(if group { 80 } else { 40 }).collect();
    let mut members = vec![actor.to_owned()];
    if group {
        if name.is_empty() {
            return Err("Nama grup wajib diisi".into());
        }
        if let Some(requested) = raw["members"].as_array() {
            for requested in requested.iter().filter_map(serde_json::Value::as_str) {
                if let Some(member) = known_name(requested) {
                    if !members.iter().any(|existing| profile(existing) == profile(member)) {
                        members.push(member.clone());
                    }
                }
            }
        }
        if members.len() < 2 {
            return Err("Pilih minimal 1 anggota lain".into());
        }
    } else {
        if name.is_empty() || profile(&name) == profile(actor) {
            return Err("Pilih editor lain".into());
        }
        members.push(known_name(&name).ok_or("Editor belum terdaftar di HOST")?.clone());
        members.sort_by_key(|member| profile(member));
    }
    let mut saved = conversations.write().expect("conversations");
    if !group {
        if let Some(existing) = saved.iter().find(|entry| {
            entry["type"] == "dm" && entry["members"].as_array().is_some_and(|list| {
                list.len() == 2 && members.iter().all(|member| list.iter().any(|existing| {
                    existing.as_str().is_some_and(|existing| profile(existing) == profile(member))
                }))
            })
        }) {
            return Ok(existing.clone());
        }
    }
    let id = format!("{}-{}", if group { "grp" } else { "dm" }, uuid::Uuid::new_v4().simple());
    let description: String = raw["description"].as_str().unwrap_or_default().trim().chars().take(200).collect();
    let entry = json!({
        "id":id,"type":if group { "group" } else { "dm" },
        "name":if group { name.as_str() } else { "" },"description":if group { description.as_str() } else { "" },
        "members":members,"admins":if group { vec![actor] } else { Vec::new() },
        "createdAt":presence_time(),"createdBy":actor,
    });
    saved.push(entry.clone());
    if let Err(error) = atomic_write(Path::new("conversations.json"), |file| {
        serde_json::to_writer(file, &*saved).map_err(io::Error::other)
    }) {
        saved.pop();
        return Err(format!("Percakapan belum tersimpan. {error}"));
    }
    Ok(entry)
}

fn accessible_conversation(
    conversations: &ConversationDb, id: &str, name: &str, host: bool,
) -> Option<serde_json::Value> {
    if id == "general" {
        let entries = conversations.read().expect("conversations");
        return Some(general_conversation(&entries));
    }
    let name = name.trim().to_lowercase();
    let entries = conversations.read().expect("conversations");
    let conversation = entries.iter().find(|entry| entry["id"].as_str() == Some(id))?;
    let is_member = conversation["members"].as_array()?.iter().any(|member| {
        member.as_str().is_some_and(|member| member.trim().to_lowercase() == name)
    });
    if !(host || is_member) || !matches!(conversation["type"].as_str(), Some("dm" | "group")) {
        return None;
    }
    let mut public = serde_json::Map::new();
    for field in ["id", "type", "name", "description", "members", "admins", "avatarUrl", "createdAt", "createdBy"] {
        if let Some(value) = conversation.get(field) {
            public.insert(field.into(), value.clone());
        }
    }
    Some(serde_json::Value::Object(public))
}

fn general_conversation(entries: &[serde_json::Value]) -> serde_json::Value {
    let mut general = json!({
        "id":"general","type":"general","name":"Minta File Bang",
        "description":"Semua editor kantor","members":[],"admins":[]
    });
    if let Some(saved) = entries.iter().find(|entry| entry["id"] == "general") {
        if let Some(name) = saved["name"].as_str().filter(|name| !name.is_empty() && *name != "Chat Umum") {
            general["name"] = json!(name);
        }
        for field in ["description", "avatarUrl", "createdAt", "createdBy"] {
            if let Some(value) = saved.get(field) { general[field] = value.clone(); }
        }
    }
    general
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupChange { General, Edit, Leave }

const GROUP_DELETE_JOURNAL: &str = "pending-group-delete.json";

fn pending_group_deletion() -> io::Result<Option<String>> {
    let file = match std::fs::File::open(GROUP_DELETE_JOURNAL) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let value: serde_json::Value = serde_json::from_reader(file).map_err(io::Error::other)?;
    let id = value["id"].as_str().filter(|id| !id.is_empty() && *id != "general" && id.len() <= 200)
        .ok_or_else(|| io::Error::other("Jurnal penghapusan grup tidak valid"))?;
    Ok(Some(id.to_owned()))
}

fn finish_group_deletion(entries: &mut Vec<serde_json::Value>, history: &Path, id: &str) -> io::Result<()> {
    if entries.iter().any(|entry| entry["id"] == id && entry["type"] != "group") {
        return Err(io::Error::other("Hanya grup yang dapat dihapus untuk semua"));
    }
    let next: Vec<_> = entries.iter().filter(|entry| entry["id"] != id).cloned().collect();
    atomic_write(Path::new("conversations.json"), |file| serde_json::to_writer(file, &next).map_err(io::Error::other))?;
    *entries = next;
    crate::history::delete_conversation(history, id).map_err(io::Error::other)?;
    std::fs::remove_file(GROUP_DELETE_JOURNAL)
}

fn recover_group_deletion(history: &Path) -> io::Result<()> {
    if let Some(id) = pending_group_deletion()? {
        let mut entries = load_conversations(Path::new("conversations.json"))?;
        finish_group_deletion(&mut entries, history, &id)?;
    }
    Ok(())
}

fn delete_group(conversations: &ConversationDb, history: &Path, id: &str) -> Result<(), String> {
    let mut entries = conversations.write().expect("conversations");
    let pending = pending_group_deletion().map_err(|error| error.to_string())?;
    if let Some(pending) = pending {
        finish_group_deletion(&mut entries, history, &pending).map_err(|error|
            format!("Penghapusan grup masih tertunda. Akan dilanjutkan saat HOST dibuka kembali. {error}"))?;
        if pending == id { return Ok(()); }
    }
    if id == "general" || !entries.iter().any(|entry| entry["id"] == id && entry["type"] == "group") {
        return Err("Hanya grup yang dapat dihapus untuk semua".into());
    }
    atomic_write(Path::new(GROUP_DELETE_JOURNAL), |file| serde_json::to_writer(file, &json!({"id":id})).map_err(io::Error::other))
        .map_err(|error| format!("Penghapusan grup belum tersimpan. {error}"))?;
    finish_group_deletion(&mut entries, history, id).map_err(|error|
        format!("Penghapusan grup masih tertunda. Akan dilanjutkan saat HOST dibuka kembali. {error}"))
}

fn change_group(
    conversations: &ConversationDb, id: &str, actor: &str, host: bool, known: &[String],
    action: GroupChange, raw: &serde_json::Value,
) -> Result<(), String> {
    let mut saved = conversations.write().expect("conversations");
    let index = saved.iter().position(|entry| entry["id"] == id);
    let same_name = |name: &str, other: &str| name.trim().to_lowercase() == other.trim().to_lowercase();
    let mut updated = if action == GroupChange::General {
        if !host { return Err("HOST only".into()); }
        index.map(|index| saved[index].clone()).unwrap_or_else(|| general_conversation(&saved))
    } else {
        let entry = index.map(|index| &saved[index]).ok_or("Grup tidak ditemukan")?;
        let member = names(&entry["members"]).any(|name| same_name(name, actor));
        if entry["type"] != "group" || !(member || host && action == GroupChange::Edit) {
            return Err("Grup tidak ditemukan".into());
        }
        if action == GroupChange::Edit && !host && !names(&entry["admins"]).any(|name| same_name(name, actor)) {
            return Err("Hanya admin grup atau HOST".into());
        }
        entry.clone()
    };
    if action == GroupChange::Leave {
        let members: Vec<_> = names(&updated["members"]).filter(|name| !same_name(name, actor)).collect();
        let mut admins: Vec<_> = names(&updated["admins"]).filter(|name| members.iter().any(|member| same_name(name, member))).collect();
        if admins.is_empty() { admins.extend(members.first().copied()); }
        let next_admins = json!(admins);
        let next_members = json!(members);
        updated["admins"] = next_admins;
        updated["members"] = next_members;
    } else {
        if let Some(name) = raw["name"].as_str().map(str::trim).filter(|name| !name.is_empty()) {
            updated["name"] = json!(name.chars().take(80).collect::<String>());
        }
        if raw.get("description").is_some() && (action != GroupChange::General || !raw["description"].is_null()) {
            let description: String = raw["description"].as_str().unwrap_or_default().trim().chars().take(300).collect();
            updated["description"] = json!(description);
        }
        if let Some(avatar) = raw.get("avatarUrl") {
            let url = avatar.as_str().unwrap_or_default().trim();
            if url.is_empty() || ["/uploads/", "/avatars/", "/status-media/"].iter().any(|prefix| url.starts_with(prefix)) {
                updated["avatarUrl"] = json!((!url.is_empty()).then_some(url));
            }
        }
        if action == GroupChange::Edit && (raw["members"].is_array() || raw["admins"].is_array()) {
            let mut members = Vec::new();
            if raw["members"].is_array() {
                for requested in names(&raw["members"]) {
                    if let Some(member) = known.iter().find(|name| same_name(name, requested)) {
                        if !members.iter().any(|name: &&str| same_name(name, member)) { members.push(member.as_str()); }
                    }
                }
                if !members.iter().any(|member| same_name(member, actor)) { members.insert(0, actor); }
            } else {
                members.extend(names(&updated["members"]));
            }
            let candidates = if raw["admins"].is_array() { &raw["admins"] } else { &updated["admins"] };
            let mut admins = Vec::new();
            for candidate in names(candidates) {
                if let Some(member) = members.iter().copied().find(|name| same_name(name, candidate)) {
                    if !admins.contains(&member) { admins.push(member); }
                }
            }
            if admins.is_empty() { admins.extend(members.first().copied()); }
            let next_admins = json!(admins);
            let next_members = json!(members);
            updated["admins"] = next_admins;
            updated["members"] = next_members;
        }
    }
    let mut next = saved.clone();
    if let Some(index) = index { next[index] = updated; } else { next.insert(0, updated); }
    atomic_write(Path::new("conversations.json"), |file| serde_json::to_writer(file, &next).map_err(io::Error::other))
        .map_err(|error| format!("Perubahan grup belum tersimpan. {error}"))?;
    *saved = next;
    Ok(())
}

fn names(list: &serde_json::Value) -> impl Iterator<Item = &str> {
    list.as_array().into_iter().flatten().filter_map(serde_json::Value::as_str)
}

struct DeviceRegistry {
    path: PathBuf,
    records: Records,
    sessions: HashMap<String, Session>,
}

struct Session {
    device_id: String,
    expires: Instant,
    key_hash: Option<String>,
}

impl DeviceRegistry {
    fn open(path: PathBuf) -> io::Result<Self> {
        let records = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Records::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { path, records, sessions: HashMap::new() })
    }

    fn save_record(&mut self, id: String, record: serde_json::Value) -> io::Result<()> {
        let mut updated = self.records.clone();
        updated.insert(id, record);
        atomic_save(&self.path, &updated)?;
        self.records = updated;
        Ok(())
    }

    fn issue_session(&mut self, device_id: String, now: Instant) -> String {
        self.sessions.retain(|_, session| session.expires >= now);
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        self.sessions.insert(token.clone(), Session { device_id, expires: now + Duration::from_secs(86400), key_hash: None });
        token
    }

    fn set_device_revoked(&mut self, id: &str, revoked: bool) -> Result<(),String> {
        let record=self.records.get(id).ok_or("Perangkat tidak ditemukan")?;
        if record["isHost"]==true {return Err("Akses perangkat HOST utama tidak dapat dicabut dari menu ini".into());}
        let mut next=self.records.clone();
        next.get_mut(id).expect("device")["revoked"]=json!(revoked);
        crate::cohost::persist_device_registry(&self.path,&mut self.records,next,json!({"deviceId":id,"approve":!revoked}))?;
        self.sessions.retain(|_,session|session.device_id!=id);
        Ok(())
    }

    fn resolve_session(&mut self, token: &str, now: Instant) -> Option<&str> {
        let session = self.sessions.get(token)?;
        let active = session.expires >= now
            && !self.records.get(&session.device_id).is_some_and(|record| record["revoked"] == true);
        if !active {
            self.sessions.remove(token);
            return None;
        }
        self.sessions.get(token).map(|session| session.device_id.as_str())
    }
}

pub(crate) fn atomic_save(path: &Path, records: &Records) -> io::Result<()> {
    atomic_write(path, |file| serde_json::to_writer(file, records).map_err(io::Error::other))
}

pub(crate) fn atomic_write(path: &Path, write: impl FnOnce(&mut std::fs::File) -> io::Result<()>) -> io::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        write(&mut file)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temporary); }
    result
}

fn register_lan_device(devices: &mut DeviceRegistry, raw: &serde_json::Value,
    conversations: &ConversationDb, socket: &SocketRef) -> Result<(), serde_json::Value> {
    let id = raw["deviceId"].as_str().unwrap_or_default();
    let name = clip_presence(&raw["name"], 40);
    let key = raw["deviceKey"].as_str().unwrap_or_default();
    if id.is_empty() || id.chars().count() > 120 || name.is_empty()
        || key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
        || devices.records.contains_key(id) {
        return Ok(());
    }
    let profile = crate::profiles::key(&name);
    let same_profile = |candidate: &str| crate::profiles::key(candidate) == profile;
    let reserved = profile.is_empty() || profile == "host"
        || devices.records.values().any(|record| record["name"].as_str().is_some_and(same_profile))
        || socket.extensions.get::<Arc<crate::cohost::Access>>().is_some_and(|access| access.reserves_profile(&name))
        || socket.extensions.get::<Arc<crate::profiles::Profiles>>().is_some_and(|profiles| profiles.contains(&name))
        || conversations.read().expect("conversations").iter()
            .any(|conversation| names(&conversation["members"]).any(same_profile));
    if reserved {
        return Err(json!({"ok":false,"code":"DEVICE_NOT_APPROVED",
            "error":"Nama ini sudah terdaftar. Sambungkan dengan perangkat asal atau hubungi HOST."}));
    }
    let now = presence_time();
    devices.save_record(id.to_owned(), json!({
        "deviceId":id,"name":name,"pc":clip_presence(&raw["pc"],80),
        "keyHash":format!("{:x}",Sha256::digest(key.as_bytes())),
        "registeredAt":now,"revoked":false,"autoRegistered":true,
        "labPrivateHistoryApproved":true
    })).map_err(|_| json!({"ok":false,"code":"DEVICE_STORAGE_FAILED","error":"Registry perangkat gagal disimpan."}))
}

fn login_lab_device(devices: &mut DeviceRegistry, raw: &serde_json::Value, strict_auth: bool) -> serde_json::Value {
    let id: String = raw["deviceId"].as_str().unwrap_or_default().chars().take(120).collect();
    let mut name: String = raw["name"].as_str().unwrap_or_default().trim().chars().take(40).collect();
    if id.is_empty() || name.is_empty() {
        return json!({"ok":false,"error":"Identitas perangkat tidak valid.","code":"INVALID_DEVICE"});
    }
    let previous = devices.records.get(&id);
    if previous.is_some_and(|record| record["revoked"] == true) {
        return json!({"ok":false,"error":"Akses perangkat dicabut oleh HOST.","code":"REVOKED"});
    }
    let key = raw["deviceKey"].as_str().unwrap_or_default();
    if strict_auth {
        let valid_key = key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit());
        let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        let approved = previous.is_some_and(|record| record["keyHash"] == hash);
        if !valid_key || !approved {
            return json!({"ok":false,"error":"Perangkat belum disetujui untuk akses LAN.","code":"DEVICE_NOT_APPROVED"});
        }
    }
    if key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let key_hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        if let Some(record) = previous.filter(|record| record["keyHash"] == key_hash) {
            if let Some(saved_name) = record["name"].as_str().filter(|name| !name.is_empty()) {
                name = saved_name.into();
            }
        } else {
            let pc: String = raw["pc"].as_str().unwrap_or_default().chars().take(80).collect();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
            let mut record = previous.cloned().filter(|record| record.is_object())
                .unwrap_or_else(|| json!({"deviceId":id,"registeredAt":now,"revoked":false,"autoRegistered":true}));
            record["name"] = json!(name);
            record["pc"] = json!(pc);
            record["keyHash"] = json!(key_hash);
            if let Some(fields) = record.as_object_mut() {
                fields.remove("labPrivateHistoryApproved");
            }
            if previous.is_some() { record["keyUpdatedAt"] = json!(now); }
            if devices.save_record(id.clone(), record).is_err() {
                return json!({"ok":false,"error":"Registry perangkat gagal disimpan.","code":"DEVICE_STORAGE_FAILED"});
            }
        }
    }
    let now = Instant::now();
    let token = devices.issue_session(id.clone(), now);
    if key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        devices.sessions.get_mut(&token).expect("issued session").key_hash = Some(format!("{:x}", Sha256::digest(key.as_bytes())));
    }
    debug_assert_eq!(devices.resolve_session(&token, now), Some(id.as_str()));
    let mut reply = json!({"ok":true,"token":token,"name":name});
    if strict_auth {
        reply["peerTemporary"] = json!(false);
    }
    reply
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn rejects_unknown_expired_and_revoked_tokens() {
        let mut registry = DeviceRegistry { path: PathBuf::new(), records: Records::new(), sessions: HashMap::new() };
        let now = Instant::now();
        let token = registry.issue_session("lab".into(), now);
        assert_eq!(registry.resolve_session(&token, now), Some("lab"));
        assert_eq!(registry.resolve_session("invalid", now), None);
        assert_eq!(registry.resolve_session(&token, now + Duration::from_secs(86401)), None);
        let token = registry.issue_session("lab".into(), now);
        registry.records.insert("lab".into(), json!({"revoked":true}));
        assert_eq!(registry.resolve_session(&token, now), None);
    }

    #[test]
    fn lan_session_requires_existing_device_key_without_rotating_it() {
        let mut registry = DeviceRegistry { path: PathBuf::new(), records: Records::new(), sessions: HashMap::new() };
        let key = "a".repeat(64);
        let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        registry.records.insert("allowed".into(), json!({"name":"Editor Lab","keyHash":hash}));
        for (id, credential) in [("unknown", key.as_str()), ("allowed", ""), ("allowed", &"b".repeat(64))] {
            let rejected = login_lab_device(&mut registry, &json!({"deviceId":id,"name":"Editor","deviceKey":credential}), true);
            assert_eq!(rejected["code"], "DEVICE_NOT_APPROVED");
        }
        let authorized = login_lab_device(&mut registry, &json!({"deviceId":"allowed","name":"Editor","deviceKey":key}), true);
        assert_eq!(authorized["ok"], true);
        assert_eq!(authorized["name"], "Editor Lab");
        assert_eq!(authorized["peerTemporary"], false);
        assert_eq!(registry.records["allowed"]["keyHash"], hash);
        assert_eq!(registry.records.len(), 1);
        assert_eq!(registry.sessions.len(), 1);
    }

    #[test]
    fn presence_does_not_publish_device_key_or_disabled_activity() {
        let mut presence = Presence::from_login(
            &json!({"name":"Editor Lab","token":"secret"}),
            &json!({"deviceId":"device","deviceKey":"private","hostAuthority":true,"activitySharing":false,
                "activity":{"kind":"app","title":"private-title"}}),
            false,
        );
        assert_eq!(presence.public["isHost"], false);
        assert_eq!(presence.public["activity"], serde_json::Value::Null);
        assert!(!presence.public.to_string().contains("private"));
        assert!(!presence.public.to_string().contains("secret"));
        presence.update_activity(&json!({"enabled":true,"activity":{"kind":"app","title":"Satu\nDua"}}));
        assert_eq!(presence.public["activity"]["title"], "Satu Dua");
    }

    #[test]
    fn reactivated_device_retains_strict_lan_credentials_after_restart() {
        let directory=std::env::temp_dir().join(format!("kc-device-reactivate-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path=directory.join("approved-devices.json");
        let key="f".repeat(64);
        let hash=format!("{:x}",Sha256::digest(key.as_bytes()));
        atomic_save(&path,json!({"client":{"name":"Client","keyHash":hash,"labPrivateHistoryApproved":true,
            "revoked":false}}).as_object().unwrap()).unwrap();
        let login=json!({"deviceId":"client","name":"Client","deviceKey":key});
        let mut registry=DeviceRegistry::open(path.clone()).unwrap();
        let initial=login_lab_device(&mut registry,&login,true);
        assert_eq!(initial["ok"],true);
        registry.set_device_revoked("client",true).unwrap();
        assert!(registry.resolve_session(initial["token"].as_str().unwrap(),Instant::now()).is_none());
        assert_eq!(login_lab_device(&mut registry,&login,true)["code"],"REVOKED");
        registry.set_device_revoked("client",false).unwrap();
        let mut registry=DeviceRegistry::open(path).unwrap();
        assert_eq!(login_lab_device(&mut registry,&login,true)["ok"],true);
        assert_eq!(registry.records["client"]["keyHash"],hash);
        assert_eq!(registry.records["client"]["labPrivateHistoryApproved"],true);
        let mut wrong=login;
        wrong["deviceKey"]=json!("e".repeat(64));
        assert_eq!(login_lab_device(&mut registry,&wrong,true)["code"],"DEVICE_NOT_APPROVED");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
