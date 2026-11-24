use super::*;
use axum::extract::State;
use serde_json::Value;

const INTENT:&str="storage-pending.json";
pub(super) fn pending()->bool {Path::new(INTENT).exists()}
fn invalid(message:&str)->io::Error {io::Error::new(io::ErrorKind::InvalidData,message)}
pub(super) fn hash(path:&Path)->io::Result<String> {
    let mut digest=Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?,&mut digest)?;
    Ok(format!("{:x}",digest.finalize()))
}
fn save(path:&Path,value:&Value)->io::Result<()> {
    super::atomic_write(path,|file|file.write_all(&serde_json::to_vec_pretty(value)?))
}
fn remove_if_present(path:&Path)->io::Result<()> {
    match std::fs::remove_file(path) {Ok(())=>Ok(()),Err(error) if error.kind()==io::ErrorKind::NotFound=>Ok(()),Err(error)=>Err(error)}
}
fn archive(id:&str)->io::Result<PathBuf> {
    let id=uuid::Uuid::parse_str(id).map_err(|_|invalid("ID karantina tidak valid"))?.to_string();
    let root=Path::new("quarantine");
    std::fs::create_dir_all(root)?;
    let folder=root.join(id);
    if folder.exists() && std::fs::symlink_metadata(&folder)?.file_type().is_symlink() {return Err(invalid("Folder karantina tidak aman"))}
    Ok(folder)
}

pub(super) fn recover(history:&Path)->io::Result<()> {
    let intent:Value=match std::fs::File::open(INTENT) {
        Ok(file)=>serde_json::from_reader(file).map_err(io::Error::other)?,
        Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(()),
        Err(error)=>return Err(error),
    };
    let stored=intent["stored"].as_str().filter(|stored|valid_name(stored)).ok_or_else(||invalid("Jurnal storage tidak valid"))?;
    let id=intent["id"].as_str().ok_or_else(||invalid("ID jurnal hilang"))?;
    let folder=archive(id)?;
    let upload_dir=super::storage_location::upload_dir(Path::new("."))?;
    let target=upload_dir.join(stored);
    let expected=intent["hash"].as_str().filter(|hash|hash.len()==64 && hash.bytes().all(|byte|byte.is_ascii_hexdigit())).ok_or_else(||invalid("Checksum jurnal tidak valid"))?;
    let restoring=intent["kind"]=="restore";
    if !restoring && intent["kind"]!="quarantine" {return Err(invalid("Operasi jurnal tidak dikenal"))}
    if restoring {
        if target.exists() {
            if hash(&target)?!=expected {return Err(invalid("File tujuan berbeda; pemulihan tidak menimpa file tersebut"))}
        } else {
            if hash(&folder.join("file"))?!=expected {return Err(invalid("Checksum karantina tidak cocok"))}
            std::fs::create_dir_all(upload_dir)?;
            super::storage_location::copy_verified(&folder.join("file"),&target,expected)?;
        }
    } else if hash(&folder.join("file"))?!=expected {return Err(invalid("Checksum salinan karantina tidak cocok"))}
    let index=intent["index"].as_object().ok_or_else(||invalid("Indeks jurnal tidak valid"))?;
    let rows=intent["rows"].as_array().ok_or_else(||invalid("Pesan jurnal tidak valid"))?;
    super::atomic_save(Path::new("file-index.json"),index)?;
    crate::history::write_batch(history,rows,presence_time() as i64).map_err(io::Error::other)?;
    if !restoring {remove_if_present(&target)?;}
    let operation=intent["operationId"].as_str().ok_or_else(||invalid("ID operasi hilang"))?;
    crate::cohost::storage_audit(operation,if restoring {"storage-restore"} else {"storage-quarantine"},json!({"id":id,"stored":stored}))?;
    if restoring {
        for name in ["file","info.json","restore.partial"] {remove_if_present(&folder.join(name))?;}
        match std::fs::remove_dir(&folder) {Ok(())=>{},Err(error) if error.kind()==io::ErrorKind::NotFound=>{},Err(error)=>return Err(error)}
    }
    remove_if_present(Path::new(INTENT))
}

fn quarantine_one(history:&Path,stored:&str)->io::Result<usize> {
    if !valid_name(stored) {return Err(invalid("Nama file tidak valid"))}
    let source=super::storage_location::upload_dir(Path::new("."))?.join(stored);
    if !std::fs::symlink_metadata(&source)?.file_type().is_file() {return Err(invalid("Bukan file lampiran"))}
    let (_,all)=crate::history::storage_inventory(history).map_err(io::Error::other)?;
    let original=all.into_iter().filter(|row|row["file"]["stored"]==stored).collect::<Vec<_>>();
    let mut next=index()?;
    let entries=next.iter().filter(|(_,entry)|entry["stored"]==stored).map(|(key,value)|json!([key,value])).collect::<Vec<_>>();
    next.retain(|_,entry|entry["stored"]!=stored);
    let id=uuid::Uuid::new_v4().to_string();
    let folder=archive(&id)?;
    std::fs::create_dir(&folder)?;
    let checksum=hash(&source)?;
    std::fs::copy(&source,folder.join("file"))?;
    std::fs::OpenOptions::new().write(true).open(folder.join("file"))?.sync_all()?;
    if hash(&folder.join("file"))?!=checksum {return Err(invalid("Karantina gagal: checksum berbeda"))}
    save(&folder.join("info.json"),&json!({"id":id,"stored":stored,"at":presence_time(),"actor":"HOST",
        "size":source.metadata()?.len(),"hash":checksum,"messages":original,"index":entries}))?;
    let rows=original.iter().cloned().map(|mut row| {
        row["file"]=Value::Null;row["attachmentRemoved"]=json!(true);row["attachmentRemovedAt"]=json!(presence_time());row
    }).collect::<Vec<_>>();
    save(Path::new(INTENT),&json!({"operationId":uuid::Uuid::new_v4().to_string(),"kind":"quarantine",
        "id":id,"stored":stored,"hash":checksum,"rows":rows,"index":next}))?;
    recover(history)?;
    Ok(original.len())
}
fn restore_one(history:&Path,id:&str)->io::Result<()> {
    let folder=archive(id)?;
    let info:Value=serde_json::from_reader(std::fs::File::open(folder.join("info.json"))?).map_err(io::Error::other)?;
    let stored=info["stored"].as_str().filter(|stored|valid_name(stored)).ok_or_else(||invalid("Nama lampiran karantina tidak valid"))?;
    let checksum=hash(&folder.join("file"))?;
    if info["hash"].as_str().is_some_and(|expected|expected!=checksum) {return Err(invalid("Checksum karantina berubah"))}
    let target=super::storage_location::upload_dir(Path::new("."))?.join(stored);
    if target.exists() && hash(&target)?!=checksum {return Err(invalid("File tujuan berbeda; pemulihan tidak menimpa file tersebut"))}
    let mut rows=Vec::new();
    for old in info["messages"].as_array().ok_or_else(||invalid("Daftar pesan karantina tidak valid"))? {
        let id=old["id"].as_str().ok_or_else(||invalid("ID pesan karantina tidak valid"))?;
        if let Some(mut current)=crate::history::message(history,id).map_err(io::Error::other)?.filter(|row|row["deleted"]!=true) {
            current["file"]=old["file"].clone();current["attachmentRemoved"]=json!(false);rows.push(current);
        }
    }
    let mut next=index()?;
    for pair in info["index"].as_array().ok_or_else(||invalid("Indeks karantina tidak valid"))? {
        let key=pair[0].as_str().ok_or_else(||invalid("Kunci indeks karantina tidak valid"))?;
        next.insert(key.into(),pair[1].clone());
    }
    save(Path::new(INTENT),&json!({"operationId":uuid::Uuid::new_v4().to_string(),"kind":"restore",
        "id":id,"stored":stored,"hash":checksum,"rows":rows,"index":next}))?;
    recover(history)
}

async fn mutate(state:Storage,headers:HeaderMap,uri:Uri,operation:impl FnOnce(&Path)->io::Result<Value>+Send+'static)->Response {
    if let Err(response)=authorize(&state.access,&headers,&uri) {return response}
    let gate=state.gate.write_owned().await;
    match tokio::task::spawn_blocking(move || {
        let _gate=gate;
        recover(&state.access.history)?;
        crate::cohost::storage_audit_ready()?;
        operation(&state.access.history)
    }).await {
        Ok(Ok(value))=>Json(value).into_response(),
        Ok(Err(error))=>failure(error),Err(error)=>failure(error),
    }
}
async fn detach(State(state):State<Storage>,RoutePath(stored):RoutePath<String>,headers:HeaderMap,uri:Uri)->Response {
    mutate(state,headers,uri,move |path| {
        let detached=quarantine_one(path,&stored)?;
        Ok(json!({"ok":true,"quarantined":true,"freed":0,"detached":detached,"storage":snapshot(path)?}))
    }).await
}
async fn restore(State(state):State<Storage>,headers:HeaderMap,uri:Uri,Json(input):Json<Value>)->Response {
    mutate(state,headers,uri,move |path| {
        restore_one(path,input["id"].as_str().unwrap_or_default())?;
        Ok(json!({"ok":true}))
    }).await
}
async fn cleanup(State(state):State<Storage>,headers:HeaderMap,uri:Uri,Json(input):Json<Value>)->Response {
    if input["confirmed"]!=true {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":"Tinjau preview dan konfirmasi dahulu"}))).into_response()}
    mutate(state,headers,uri,move |path| {
        let (_,rows)=crate::history::storage_inventory(path).map_err(io::Error::other)?;
        let referenced=rows.iter().filter_map(|row|row["file"]["stored"].as_str()).collect::<std::collections::HashSet<_>>();
        let selected=input["stored"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<std::collections::HashSet<_>>();
        let mut removed=0;
        for stored in selected {
            if !valid_name(stored) || referenced.contains(stored) {continue}
            let source=super::storage_location::upload_dir(Path::new("."))?.join(stored);
            let metadata=match std::fs::symlink_metadata(&source) {Ok(meta)=>meta,Err(error) if error.kind()==io::ErrorKind::NotFound=>continue,Err(error)=>return Err(error)};
            if metadata.is_file() && presence_time().saturating_sub(milliseconds(metadata.modified()?))>86_400_000 {
                quarantine_one(path,stored)?;removed+=1;
            }
        }
        Ok(json!({"ok":true,"removed":removed,"freed":0,"quarantined":true,"storage":snapshot(path)?}))
    }).await
}

#[derive(Clone)]
struct Storage {access:Attachments,gate:SnapshotGate,migrating:Arc<std::sync::atomic::AtomicBool>}

pub fn router(access: Attachments,gate:SnapshotGate) -> Router {
    Router::new().route("/storage",get(list))
        .route("/storage/message/{stored}",get(references))
        .route("/storage/quarantine",get(quarantine))
        .route("/storage/cleanup/preview",get(cleanup_preview))
        .route("/storage/file/{stored}",axum::routing::delete(detach))
        .route("/storage/quarantine/restore",post(restore))
        .route("/storage/cleanup",post(cleanup))
        .route("/storage/location",get(location))
        .route("/admin/storage/migrate",post(migrate).layer(axum::extract::DefaultBodyLimit::max(64*1024)))
        .with_state(Storage{access,gate,migrating:Arc::default()})
}

async fn location(State(state):State<Storage>,headers:HeaderMap,uri:Uri)->Response {
    if let Err(response)=authorize(&state.access,&headers,&uri) {return response}
    match super::storage_location::location() {
        Ok(mut result)=>{
            result["migrating"]=json!(state.migrating.load(std::sync::atomic::Ordering::Acquire));
            Json(result).into_response()
        }
        Err(error)=>failure(error),
    }
}

async fn migrate(State(state):State<Storage>,headers:HeaderMap,uri:Uri,Json(input):Json<Value>)->Response {
    if let Err(response)=authorize(&state.access,&headers,&uri) {return response}
    let Ok(gate)=state.gate.clone().try_write_owned() else {
        return (StatusCode::CONFLICT,Json(json!({"ok":false,"error":"HOST masih menulis data. Coba pindahkan storage lagi."}))).into_response()
    };
    match tokio::task::spawn_blocking(move || {
        let _gate=gate;
        state.migrating.store(true,std::sync::atomic::Ordering::Release);
        let result=recover(&state.access.history).and_then(|_|super::storage_location::migrate(&input,|event|{
            emit_migration(&state.access,&event);
        }));
        state.migrating.store(false,std::sync::atomic::Ordering::Release);
        let event=match &result {
            Ok(result)=>json!({"active":false,"done":true,"targetRoot":result["storageRoot"],"copied":result["copied"],"total":result["total"],"bytes":result["bytes"],"deleted":result["deleted"],"warnings":result["warnings"]}),
            Err(error)=>json!({"active":false,"error":error.to_string()}),
        };
        emit_migration(&state.access,&event);
        result
    }).await {
        Ok(Ok(value))=>Json(value).into_response(),
        Ok(Err(error)) if error.kind()==io::ErrorKind::InvalidInput=>(StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":error.to_string()}))).into_response(),
        Ok(Err(error))=>failure(error),Err(error)=>failure(error),
    }
}

fn emit_migration(access:&Attachments,event:&Value) {
    for socket in access.io.sockets() {
        if socket.extensions.get::<SocketSession>().is_some() {
            let _=socket.emit("storage-migration-status",event);
        }
    }
}

fn authorize(access:&Attachments,headers:&HeaderMap,uri:&Uri)->Result<(),Response> {
    if super::device_admin::authorized(access,headers,uri) {Ok(())}
    else {Err((StatusCode::FORBIDDEN,Json(json!({"ok":false,"error":"Hanya HOST"}))).into_response())}
}
fn failure(error:impl std::fmt::Display)->Response {
    (StatusCode::INTERNAL_SERVER_ERROR,Json(json!({"ok":false,"error":format!("Operasi penyimpanan gagal: {error}")}))).into_response()
}
fn valid_name(name:&str)->bool {
    !name.is_empty() && name!="." && name!=".." && !name.contains(['/', '\\', ':'])
}
fn milliseconds(time:std::time::SystemTime)->u64 {
    time.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
fn index()->io::Result<serde_json::Map<String,Value>> {
    match std::fs::File::open("file-index.json") {
        Ok(file)=>serde_json::from_reader(file).map_err(io::Error::other),
        Err(error) if error.kind()==io::ErrorKind::NotFound=>Ok(Default::default()),
        Err(error)=>Err(error),
    }
}
fn refs(rows:&[Value],stored:&str)->Vec<Value> {
    rows.iter().filter(|row|row["file"]["stored"]==stored).map(|row|json!({
        "id":row["id"],"sender":row["sender"],"ts":row["ts"],"conversationId":row["conversationId"].as_str().unwrap_or("general"),
        "text":row["text"].as_str().unwrap_or_default().chars().take(120).collect::<String>()
    })).collect()
}
fn dir_bytes(path:&Path)->io::Result<u64> {
    if !path.exists() {return Ok(0)}
    let mut total=0;
    for entry in std::fs::read_dir(path)? {
        let entry=entry?;
        let kind=entry.file_type()?;
        if kind.is_file() {total+=entry.metadata()?.len();}
        else if kind.is_dir() {total+=dir_bytes(&entry.path())?;}
    }
    Ok(total)
}
fn snapshot(path:&Path)->io::Result<Value> {
    let (message_count,rows)=crate::history::storage_inventory(path).map_err(io::Error::other)?;
    let mut grouped=HashMap::<String,Vec<Value>>::new();
    for row in rows {
        if let Some(stored)=row["file"]["stored"].as_str() {grouped.entry(stored.into()).or_default().push(row);}
    }
    let index=index()?;
    let stats:Value=match std::fs::File::open("storage-stats.json") {
        Ok(file)=>serde_json::from_reader(file).map_err(io::Error::other)?,
        Err(error) if error.kind()==io::ErrorKind::NotFound=>json!({}),
        Err(error)=>return Err(error),
    };
    let mut files=Vec::new();
    let upload_dir=super::storage_location::upload_dir(Path::new("."))?;
    if upload_dir.exists() {
        for entry in std::fs::read_dir(upload_dir)? {
            let entry=entry?;
            if !entry.file_type()?.is_file() || entry.file_name().to_string_lossy().ends_with(".migrate-part") {continue}
            let stored=entry.file_name().to_string_lossy().into_owned();
            let metadata=entry.metadata()?;
            let attached=grouped.get(&stored).map(Vec::as_slice).unwrap_or_default();
            let message_refs=refs(attached,&stored);
            let meta=attached.first().map(|row|&row["file"])
                .or_else(||index.values().find(|item|item["stored"]==stored)).unwrap_or(&Value::Null);
            files.push(json!({"stored":stored,"name":meta["name"].as_str().unwrap_or(&stored),
                "mimetype":meta["mimetype"].as_str().unwrap_or_default(),"hash":meta["hash"].as_str().unwrap_or_default(),
                "size":metadata.len(),"referenced":!message_refs.is_empty(),"messageRefs":message_refs,
                "modifiedAt":milliseconds(metadata.modified()?)}));
        }
    }
    files.sort_by_key(|file|std::cmp::Reverse(file["size"].as_u64().unwrap_or_default()));
    let bytes=files.iter().map(|file|file["size"].as_u64().unwrap_or_default()).sum::<u64>();
    let orphans=files.iter().filter(|file|file["referenced"]!=true).collect::<Vec<_>>();
    let orphan_bytes=orphans.iter().map(|file|file["size"].as_u64().unwrap_or_default()).sum::<u64>();
    let orphan_count=orphans.len();
    Ok(json!({"totalBytes":bytes,"uploadBytes":bytes,"avatarBytes":dir_bytes(Path::new("avatars"))?,
        "uniqueFiles":files.len(),"referencedFiles":files.len()-orphan_count,"orphanFiles":orphan_count,
        "orphanBytes":orphan_bytes,"messageCount":message_count,"duplicatesAvoided":stats["duplicatesAvoided"].as_u64().unwrap_or_default(),
        "bytesSaved":stats["bytesSaved"].as_u64().unwrap_or_default(),
        "largestFiles":files.into_iter().take(200).collect::<Vec<_>>()}))
}
async fn list(State(Storage{access,..}):State<Storage>,headers:HeaderMap,uri:Uri)->Response {
    if let Err(response)=authorize(&access,&headers,&uri) {return response}
    match tokio::task::spawn_blocking(move ||snapshot(&access.history)).await {
        Ok(Ok(storage))=>Json(json!({"ok":true,"storage":storage})).into_response(),
        Ok(Err(error))=>failure(error),Err(error)=>failure(error),
    }
}
async fn references(State(Storage{access,..}):State<Storage>,RoutePath(stored):RoutePath<String>,headers:HeaderMap,uri:Uri)->Response {
    if let Err(response)=authorize(&access,&headers,&uri) {return response}
    if !valid_name(&stored) {return (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":"Nama file tidak valid"}))).into_response()}
    match tokio::task::spawn_blocking(move ||crate::history::storage_inventory(&access.history).map(|(_,rows)|refs(&rows,&stored))).await {
        Ok(Ok(refs))=>Json(json!({"ok":true,"refs":refs})).into_response(),
        Ok(Err(error))=>failure(error),Err(error)=>failure(error),
    }
}
async fn cleanup_preview(State(Storage{access,..}):State<Storage>,headers:HeaderMap,uri:Uri)->Response {
    if let Err(response)=authorize(&access,&headers,&uri) {return response}
    match tokio::task::spawn_blocking(move ||->io::Result<Vec<Value>> {
        // Scan every file: the display's 200-largest limit must not limit cleanup.
        let (_,rows)=crate::history::storage_inventory(&access.history).map_err(io::Error::other)?;
        let referenced=rows.iter().filter_map(|row|row["file"]["stored"].as_str()).collect::<std::collections::HashSet<_>>();
        let mut items=Vec::new();
        let upload_dir=super::storage_location::upload_dir(Path::new("."))?;
        if !upload_dir.exists() {return Ok(items)}
        for entry in std::fs::read_dir(upload_dir)? {
            let entry=entry?;
            if !entry.file_type()?.is_file() || entry.file_name().to_string_lossy().ends_with(".migrate-part") {continue}
            let stored=entry.file_name().to_string_lossy().into_owned();
            let metadata=entry.metadata()?;
            let mtime=milliseconds(metadata.modified()?);
            if !referenced.contains(stored.as_str()) && presence_time().saturating_sub(mtime)>86_400_000 {
                items.push(json!({"stored":stored,"size":metadata.len(),"mtime":mtime}));
            }
        }
        Ok(items)
    }).await {
        Ok(Ok(items))=>Json(json!({"ok":true,"items":items,"note":"File tanpa referensi, umur minimal 24 jam. Dipindah ke karantina."})).into_response(),
        Ok(Err(error))=>failure(error),Err(error)=>failure(error),
    }
}
async fn quarantine(State(Storage{access,..}):State<Storage>,headers:HeaderMap,uri:Uri)->Response {
    if let Err(response)=authorize(&access,&headers,&uri) {return response}
    match tokio::task::spawn_blocking(move ||->io::Result<Vec<Value>> {
        let mut items=Vec::new();
        if !Path::new("quarantine").exists() {return Ok(items)}
        for entry in std::fs::read_dir("quarantine")? {
            let entry=entry?;
            if !entry.file_type()?.is_dir() {continue}
            let file=match std::fs::File::open(entry.path().join("info.json")) {
                Ok(file)=>file,Err(error) if error.kind()==io::ErrorKind::NotFound=>continue,Err(error)=>return Err(error),
            };
            let mut item:Value=serde_json::from_reader(file).map_err(io::Error::other)?;
            if let Some(object)=item.as_object_mut() {object.remove("messages");object.remove("index");}
            items.push(item);
        }
        Ok(items)
    }).await {
        Ok(Ok(items))=>Json(json!({"ok":true,"items":items})).into_response(),
        Ok(Err(error))=>failure(error),Err(error)=>failure(error),
    }
}
