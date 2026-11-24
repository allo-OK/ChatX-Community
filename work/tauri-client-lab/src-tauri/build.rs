use std::{fs, io, path::Path};

fn copy_asset(from: &Path, to: &Path) -> io::Result<()> {
    println!("cargo:rerun-if-changed={}", from.display());
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_asset(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        fs::create_dir_all(to.parent().expect("asset directory"))?;
        if fs::read(from)? != fs::read(to).unwrap_or_default() || !to.exists() {
            fs::copy(from, to)?;
        }
    }
    Ok(())
}

fn stage_renderer() -> io::Result<()> {
    let source = Path::new("../../source-v3.7.11/ChatX");
    let target = Path::new("../ui/v814");
    // This directory is generated and must not retain resources from an earlier build.
    if target.exists() {
        fs::remove_dir_all(target)?;
    }
    copy_asset(&source.join("renderer"), &target.join("renderer"))?;
    for resource in ["work-break-bell.mp3", "workday-end-bell.wav", "workday-end-video.mp4"] {
        let relative = Path::new("assets").join(resource);
        copy_asset(&source.join(&relative), &target.join(&relative))?;
    }
    let popup = target.join("renderer/work-schedule-popup.html");
    let html = fs::read_to_string(&popup)?;
    let (before, script) = html.split_once("<script>").ok_or_else(|| io::Error::other("Work schedule script missing"))?;
    let (script, after) = script.split_once("</script>").ok_or_else(|| io::Error::other("Work schedule script not closed"))?;
    let script = script.replace("window.kcSchedulePopup.close()", "closeSchedule()");
    fs::write(target.join("renderer/rust-work-schedule.js"), format!(
        "const closeSchedule=()=>window.__TAURI__.core.invoke('close_work_schedule');\ndocument.querySelector('[data-close-schedule]').onclick=closeSchedule;\n{script}"))?;
    fs::write(popup, format!("{}<script src=\"rust-work-schedule.js\"></script>{after}",
        before.replace("onclick=\"window.kcSchedulePopup.close()\"", "data-close-schedule")))?;
    let reliability = target.join("renderer/reliability.js");
    let mut device_panel = fs::read_to_string(&reliability)?;
    let start = device_panel.find("async function openDevices()")
        .ok_or_else(|| io::Error::other("Akses CLIENT binding changed"))?;
    let end = start + device_panel[start..].find('\n')
        .ok_or_else(|| io::Error::other("Akses CLIENT binding has no line ending"))?;
    println!("cargo:rerun-if-changed=../host-device-panel.js");
    device_panel.replace_range(start..end, include_str!("../host-device-panel.js").trim_end());
    device_panel = device_panel.replace("trustHostKey({origin,publicKey:r.hostPublicKey})",
        "trustHostKey({origin,publicKey:r.hostPublicKey,fingerprint:r.hostFingerprint,token:r.token})");
    fs::write(reliability, device_panel)?;
    let cohost_path = target.join("renderer/cohost-settings.js");
    let cohost_source = fs::read_to_string(&cohost_path)?;
    let recipient_marker = "if(canManage){const label=document.createElement('label'),video=";
    if cohost_source.matches(recipient_marker).count() != 1 {
        return Err(io::Error::other("Cohost notification control binding changed"));
    }
    println!("cargo:rerun-if-changed=../notification-recipient-controls.js");
    fs::write(&cohost_path, cohost_source.replace(recipient_marker,
        &format!("{}{}",include_str!("../notification-recipient-controls.js"),recipient_marker)))?;
    let index = fs::read_to_string(source.join("renderer/index.html"))?;
    for resource in index.split("src=\"../").skip(1).filter_map(|part| part.split('"').next()) {
        if resource.starts_with("lib/") || resource == "node_modules/socket.io-client/dist/socket.io.min.js" {
            copy_asset(&source.join(resource), &target.join(resource))?;
        }
    }
    stage_vectorizer(source, &target)?;
    stage_background_engine(source, &target)?;
    stage_notifications(source, &target)?;
    println!("cargo:rerun-if-changed=../ui/chatx.png");
    let target_index = target.join("renderer/index.html");
    let branded_index = fs::read_to_string(&target_index)?.replace("ChatX", "ChatX").replace("ChatX-Chat-CLIENT-SETUP-", "ChatX-CLIENT-SETUP-")
        .replace("<div class=\"brandmark\">KC</div>", "<img class=\"brandmark\" src=\"../../chatx.png\" alt=\"\" width=\"56\" height=\"56\">")
        .replace("<div class=\"empty-logo\">KC</div>", "<img class=\"empty-logo\" src=\"../../chatx.png\" alt=\"\" width=\"56\" height=\"56\">");
    fs::write(target_index, branded_index)?;
    for entry in fs::read_dir(target.join("renderer"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "html") {
            let html = fs::read_to_string(&path)?;
            let branded = html.replace("ChatX", "ChatX").replace("ChatX-Chat-CLIENT-SETUP-", "ChatX-CLIENT-SETUP-")
                .replace("CHATX", "CHATX")
                .replace("../assets/app_icon.png", "../../chatx.png");
            if branded != html {
                fs::write(&path, branded)?;
            }
        }
    }
    let game = target.join("renderer/game-window.js");
    let game_renderer = fs::read_to_string(source.join("renderer/game-window.js"))?;
    fs::write(game, game_renderer.replace("ChatX", "ChatX")
        .replace("window.close()", "(api?.close ? api.close() : window.close())"))?;
    Ok(())
}

fn stage_notifications(source: &Path, target: &Path) -> io::Result<()> {
    let assets = target.join("renderer");
    copy_asset(Path::new("../notification.html"), &assets.join("rust-notification.html"))?;
    copy_asset(Path::new("../notification.js"), &assets.join("rust-notification.js"))?;
    for (original, output) in [("bledar/video-01.mp4", "default.mp4"), ("classic_ping.wav", "classic.wav"),
        ("iphone_ding.mp3", "iphone.mp3"), ("office_bell.wav", "bell.wav"),
        ("soft_pop.wav", "pop.wav"), ("mention_bledar.wav", "mention.wav")] {
        copy_asset(&source.join("assets").join(original), &assets.join("notification-media").join(output))?;
    }
    let path = assets.join("app.js");
    let renderer = fs::read_to_string(&path)?;
    let needle = "socket.on('moderation-updated'";
    if !renderer.contains(needle) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Moderation event binding changed"));
    }
    let mut renderer = renderer.replace("\r\n", "\n")
        .replace("ChatX HOST", "ChatX").replace("ChatX CLIENT", "ChatX").replace("ChatX", "ChatX").replace("ChatX-Chat-CLIENT-SETUP-", "ChatX-CLIENT-SETUP-");
    println!("cargo:rerun-if-changed=../profile-live-update.js");
    let profile_marker="if(key)conversationProfileCache.set(key,p||{});";
    if renderer.matches(profile_marker).count()!=1 { return Err(io::Error::other("Profile update binding changed")); }
    renderer=renderer.replace(profile_marker,include_str!("../profile-live-update.js"));
    renderer=renderer.replace("return!c.query||k.includes(c.query);", "return!c.query||k.includes(c.query)||String(profileDisplayName(n)).toLowerCase().includes(c.query);");
    renderer=renderer.replace("const name=choices[i].name;el.value=", "const name=profileDisplayName(choices[i].name);el.value=");
    renderer=renderer.replace("return t.includes(me)||t.includes('@everyone')", "return t.includes(me)||t.includes('@'+String(profileDisplayName(cfg.name)).toLowerCase())||t.includes('@everyone')");
    renderer=renderer.replace("title:m.sender,body:latest.text", "title:profileDisplayName(m.sender),body:latest.text");
    renderer=renderer.replace("if(mention&&m.tagVideoEnabled!==false&&!chosenVideo", "if(mention&&myPolicy.receiveTagVideoEnabled!==false&&m.tagVideoEnabled!==false&&!chosenVideo");
    let notification_marker = "tagVideoEnabled:m.tagVideoEnabled,bledarVideo:";
    if renderer.matches(notification_marker).count() != 1 {
        return Err(io::Error::other("Notification payload binding changed"));
    }
    renderer = renderer.replace(notification_marker,
        "tagVideoEnabled:myPolicy.receiveTagVideoEnabled===false?false:m.tagVideoEnabled,tagSoundEnabled:myPolicy.receiveTagSoundEnabled!==false,conversationType:conversations.find(c=>String(c.id)===String(m.conversationId||'general'))?.type,conversationName:conversations.find(c=>String(c.id)===String(m.conversationId||'general'))?.name||'Grup kantor',bledarVideo:");
    let scan_start=renderer.find("async function autoScanFileGallery(")
        .ok_or_else(||io::Error::new(io::ErrorKind::InvalidData,"Gallery scan binding changed"))?;
    let scan_end=scan_start+renderer[scan_start..].find('\n')
        .ok_or_else(||io::Error::new(io::ErrorKind::InvalidData,"Gallery scan binding has no line ending"))?;
    let scan=renderer[scan_start..scan_end].to_owned();
    let guard="if(galleryScanBusy||!window.kc?.scanGalleryFolders)return;";
    if !scan.contains(guard) || scan.matches("finally{galleryScanBusy=false;}").count()!=2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData,"Gallery scan lifecycle changed"));
    }
    let patched=scan.replace(guard,
        "if(!window.kc?.scanGalleryFolders)return;if(galleryScanBusy){if(forceRefresh||!rustGalleryPendingScan)rustGalleryPendingScan={showUi,forceRefresh,folders:galleryFolders().slice()};return;}")
        .replace("finally{galleryScanBusy=false;}","finally{rustFinishGalleryScan();}")
        .replace("syncGalleryDatabaseV2()", "syncGalleryDatabaseV2(showUi&&forceRefresh)");
    let prefix=r#"let rustGalleryPendingScan=null;
function rustFinishGalleryScan(){
  galleryScanBusy=false;
  const pending=rustGalleryPendingScan;
  rustGalleryPendingScan=null;
  if(pending){
    cfg.galleryFolders=pending.folders;
    void autoScanFileGallery(pending.showUi,pending.forceRefresh);
  }
}
"#;
    renderer.replace_range(scan_start..scan_end,&format!("{prefix}{patched}"));
    renderer=renderer
        .replace("function syncGalleryDatabaseV2(){", "function syncGalleryDatabaseV2(forceSnapshot=false){")
        .replace("gallerySyncPromise=syncGalleryDatabaseRun()", "gallerySyncPromise=syncGalleryDatabaseRun(forceSnapshot)")
        .replace("async function syncGalleryDatabaseRun(){", "async function syncGalleryDatabaseRun(forceSnapshot){")
        .replace("let full=remote.generation!==generation", "let full=forceSnapshot||remote.generation!==generation")
        .replace("if(delta?.items?.length&&Number(delta.items[0].seq)!==after+1)full=true;",
            "if(!full&&after<localMax&&(!delta?.ok||!delta.items?.length||Number(delta.items[0].seq)!==after+1))full=true;")
        .replace("if(!remote?.ok||!remote.database)return{database:false};",
            "if(!remote?.ok||!remote.database)throw Error(remote?.error||'Index galeri HOST belum tersedia');")
        .replace("if(showUi)showToast(e.message||'Scan File Gallery gagal');",
            "if(showUi)showToast(e.message||'Scan File Gallery gagal');window.kc.logEvent?.({category:'gallery',event:'scan-failed',message:String(e?.message||e)})?.catch(()=>{});");
    let ack_start = renderer.find("function socketAckEvent(event,payload,timeout=15000)")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,"Socket acknowledgment binding changed"))?;
    renderer.insert_str(ack_start, r#"async function rustGalleryAck(event,payload,timeout){
  const active=socket;
  if(!active?.connected)return {ok:false,error:'Tidak terhubung ke HOST'};
  try{
    const result=await active.timeout(timeout).emitWithAck(event,payload);
    return active===socket&&active.connected?result:{ok:false,error:'Koneksi HOST berubah'};
  }catch{return {ok:false,error:'Koneksi HOST terputus atau tidak merespons'};}
}
"#);
    for event in ["file-gallery-index-status","file-gallery-delta"] {
        renderer=renderer.replace(&format!("socketAckEvent('{event}'"),&format!("rustGalleryAck('{event}'"));
    }
    let start = renderer.find("function applyMessageReceipt(p){")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Message receipt binding changed"))?;
    let end = start + renderer[start..].find('\n')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Message receipt binding has no line ending"))?;
    renderer.replace_range(start..end, r#"function applyMessageReceipt(p){
  if(String(p?.conversationId)!==String(activeConversationId)||!p?.receipt)return;
  const message=messageStore.find(item=>String(item.id)===String(p.id));
  if(!message)return;
  message.receipt=p.receipt;
  if(p.readBy)message.readBy=p.readBy;
  saveChatCache(activeConversationId,messageStore);
  const row=[...$('messages').querySelectorAll('.msg-row[data-message-id]')]
    .find(item=>String(item.dataset.messageId)===String(p.id));
  if(row)updateExistingReceipt(row,message);
}"#);
    for (name, replacement) in [
        ("async function focusIncomingMessage(", r#"async function focusIncomingMessage(id,cid=activeConversationId){
  id=String(id||'');cid=String(cid||'general');
  if(!id||String(activeConversationId)===cid&&jumpToMessage(id))return;
  openConversation(cid,id);
}"#),
        ("if(window.kc?.onFocusMessage)window.kc.onFocusMessage(", r#"if(window.kc?.onFocusMessage)window.kc.onFocusMessage(payload=>{
  const data=payload&&typeof payload==='object'?payload:{messageId:payload};
  void focusIncomingMessage(data.messageId,data.conversationId||activeConversationId);
  focusChatComposer(true);
});"#),
        ("async function goToStorageMessage(", r#"async function goToStorageMessage(id){
  const ref=storageFiles.flatMap(file=>file.messageRefs||[]).find(ref=>String(ref.id)===String(id));
  $('storageModal').classList.add('hidden');
  await focusIncomingMessage(id,ref?.conversationId||'general');
}"#),
    ] {
        let start = renderer.find(name).ok_or_else(|| io::Error::other(format!("Message focus binding changed: {name}")))?;
        let end = start + renderer[start..].find('\n').ok_or_else(|| io::Error::other("Message focus binding has no line ending"))?;
        renderer.replace_range(start..end, replacement);
    }
    for (original, replacement) in [
        ("function openConversation(id){",
         "let rustHistoryRequest=0,rustFocusedHistory=false;\nfunction openConversation(id,focusId=''){const request=++rustHistoryRequest;rustFocusedHistory=false;"),
        ("function closeActiveConversation(){",
         "function closeActiveConversation(){rustHistoryRequest++;rustFocusedHistory=false;"),
        ("socket.emit('conversation-history',{id,limit:120},r=>{\n    if(String(activeConversationId)!==id)return;",
         "socket.emit('conversation-history',{id,limit:120,...(focusId?{aroundId:focusId}:{})},r=>{\n    if(String(activeConversationId)!==id||request!==rustHistoryRequest)return;"),
        ("messageStore=Array.isArray(r.items)?r.items.slice():[];saveChatCache(id,messageStore);historyHasMore=!!r.hasMore;renderCurrentMessages(true);",
         r#"messageStore=Array.isArray(r.items)?r.items.slice():[];rustFocusedHistory=!!r.hasNewer;saveChatCache(id,messageStore);historyHasMore=!!r.hasMore;renderCurrentMessages(true);
    if(focusId){requestAnimationFrame(()=>{if(request===rustHistoryRequest&&!jumpToMessage(focusId))showToast('Pesan tidak ditemukan di tampilan chat.');});socket.emit('conversation-open',{id},()=>requestConversationList());return;}"#),
        ("function saveChatCache(id, items) {",
         "function saveChatCache(id, items) {if(rustFocusedHistory&&String(id)===String(activeConversationId))return;"),
        ("const wasNearBottom=chatIsNearBottom(150);if(messageStore.some",
         "if(rustFocusedHistory){requestConversationList();return;}const wasNearBottom=chatIsNearBottom(150);if(messageStore.some"),
        ("const id=String(activeConversationId),oldest=messageStore.length",
         "const request=rustHistoryRequest,id=String(activeConversationId),oldest=messageStore.length"),
        ("r=>{historyLoadingOlder=false;if(String(activeConversationId)!==id)return;",
         "r=>{if(request!==rustHistoryRequest)return;historyLoadingOlder=false;if(String(activeConversationId)!==id)return;"),
        ("const request=++rustHistoryRequest;rustFocusedHistory=false;",
         "const request=++rustHistoryRequest;rustFocusedHistory=false;historyLoadingOlder=false;"),
        ("appendMessageTimeline(messageStore,target);}finally",
         r#"appendMessageTimeline(messageStore,target);if(rustFocusedHistory){const latest=document.createElement('button');latest.type='button';latest.className='older-messages-btn';latest.textContent='↓ Ke pesan terbaru';latest.onclick=()=>openConversation(activeConversationId);target.appendChild(latest);}}finally"#),
        ("function activeConversation(){return conversations.find(c=>String(c.id)===String(activeConversationId))||null;}",
         "let rustAuditConversation=null;\nfunction activeConversation(){return conversations.find(c=>String(c.id)===String(activeConversationId))||(cfg?.isHost&&String(rustAuditConversation?.id)===String(activeConversationId)?rustAuditConversation:null);}"),
        ("const c=r.conversation||known;if(c){",
         "const c=r.conversation||known;if(c&&cfg?.isHost)rustAuditConversation=c;if(c){"),
        ("socket.on('conversation-removed',p=>{",
         "socket.on('conversation-removed',p=>{if(String(rustAuditConversation?.id)===String(p?.id))rustAuditConversation=null;"),
    ] {
        if !renderer.contains(original) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "HOST audit conversation binding changed"));
        }
        renderer = renderer.replacen(original, replacement, 1);
    }
    fs::write(path, renderer.replacen(needle,
        "socket.on('cohost-updated',data=>{if(String(data?.name||'').trim().toLowerCase()===String(cfg?.name||'').trim().toLowerCase()){myPolicy.cohost=data.scopes||{};window.KCCohost?.refresh();window.KCBledarLibrary?.refresh();}});socket.on('moderation-updated'", 1))?;
    let cohost = fs::read_to_string(assets.join("cohost-settings.js"))?;
    fs::write(assets.join("cohost-settings.js"), cohost.replace("myPolicy.cohost=data.scopes;",
        "myPolicy.cohost=data.scopes;if(data.permissions){myPolicy.permissions={...myPolicy.permissions,...data.permissions};applyFeaturePolicy();}"))
}

fn stage_background_engine(source: &Path, target: &Path) -> io::Result<()> {
    let assets = target.join("renderer");
    copy_asset(Path::new("../background-worker.js"), &assets.join("rust-background-worker.js"))?;
    let runtime = source.join("node_modules/onnxruntime-web/dist");
    for file in ["ort.bundle.min.mjs", "ort.webgpu.bundle.min.mjs", "ort-wasm-simd-threaded.mjs",
        "ort-wasm-simd-threaded.wasm", "ort-wasm-simd-threaded.jsep.mjs", "ort-wasm-simd-threaded.jsep.wasm"] {
        copy_asset(&runtime.join(file), &assets.join("onnx").join(file))?;
    }
    let package = source.join("node_modules/@imgly/background-removal-node");
    copy_asset(&package.join("LICENSE.md"), &assets.join("onnx/imgly-license.md"))?;
    let models = package.join("dist");
    let manifest = models.join("resources.json");
    copy_asset(&manifest, &assets.join("onnx/models/resources.json"))?;
    let resources: serde_json::Value = serde_json::from_slice(&fs::read(manifest)?)?;
    for model in ["/models/small", "/models/medium"] {
        let chunks = resources[model]["chunks"].as_array()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing background model chunks"))?;
        for chunk in chunks {
            let hash = chunk["hash"].as_str().filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid model chunk"))?;
            copy_asset(&models.join(hash), &assets.join("onnx/models").join(hash))?;
        }
    }
    let app = assets.join("app.js");
    let mut renderer = fs::read_to_string(&app)?;
    for (original, replacement) in [
        ("let headCropPreviewToken=0;", "let headCropPreviewToken=0,headCropResultPending=Promise.resolve();\nasync function waitHeadCropResult(){let pending;do{pending=headCropResultPending;await pending;}while(pending!==headCropResultPending);}"),
        ("renderHeadFaceList();updateHeadCropResultPreview();}", "renderHeadFaceList();headCropResultPending=updateHeadCropResultPreview();}"),
        ("drawHeadCrop();await autoDetectHead();return true;", "await autoDetectHead();return true;"),
        ("async function saveHeadCropCurrent(nameBase='KC-crop-kepala'){", "async function saveHeadCropCurrent(nameBase='KC-crop-kepala'){await waitHeadCropResult();"),
        ("$('sendHeadCrop').onclick=()=>{prepareCanvasForChat", "$('sendHeadCrop').onclick=async()=>{await waitHeadCropResult();prepareCanvasForChat"),
        ("drawHeadCrop();await new Promise(r=>setTimeout(r,180));const base='KC-'", "drawHeadCrop();const base='KC-'"),
    ] {
        if !renderer.contains(original) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Crop renderer changed"));
        }
        renderer = renderer.replacen(original, replacement, 1);
    }
    fs::write(app, renderer)
}

fn stage_vectorizer(source: &Path, target: &Path) -> io::Result<()> {
    let assets = target.join("renderer");
    copy_asset(Path::new("../vector-worker.js"), &assets.join("rust-vector-worker.js"))?;
    copy_asset(&source.join("node_modules/image-q/dist/esm/image-q.mjs"), &assets.join("image-q.mjs"))?;
    copy_asset(&source.join("node_modules/image-q/LICENSE"), &assets.join("image-q-license.txt"))?;
    let package = source.join("node_modules/potrace");
    copy_asset(&package.join("LICENSE"), &assets.join("potrace-license.txt"))?;
    let modules = ["Potrace", "utils", "types/Bitmap", "types/Curve", "types/Histogram", "types/Opti",
        "types/Path", "types/Point", "types/Quad", "types/Sum"];
    let mut bundle = String::from("const modules = Object.create(null);\n");
    for module in modules {
        let path = package.join(format!("lib/{module}.js"));
        println!("cargo:rerun-if-changed={}", path.display());
        bundle.push_str(&format!("modules['{module}'] = (require, module, exports) => {{\n{}\n}};\n", fs::read_to_string(path)?));
    }
    bundle.push_str(r#"
const cache = Object.create(null);
function load(id) {
  if (id === 'jimp') return null;
  if (cache[id]) return cache[id].exports;
  const module = cache[id] = { exports: {} };
  modules[id](relative => {
    if (!relative.startsWith('.')) return load(relative);
    const parts = id.split('/').slice(0, -1);
    for (const part of relative.split('/')) {
      if (part === '..') parts.pop();
      else if (part !== '.') parts.push(part);
    }
    return load(parts.join('/'));
  }, module, module.exports);
  return module.exports;
}
export const Potrace = load('Potrace');
export const Bitmap = load('types/Bitmap');
"#);
    fs::write(assets.join("potrace.mjs"), bundle)?;
    let gpu = source.join("renderer/gpu-worker.js");
    println!("cargo:rerun-if-changed={}", gpu.display());
    let gpu = fs::read_to_string(gpu)?;
    let function = gpu.split_once("async function gpuPosterize").and_then(|(_, rest)| rest.split_once("api.onTask"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Vector GPU worker changed"))?.0;
    fs::write(assets.join("vector-gpu.mjs"), format!(r#"
const fromB64 = value => Uint8Array.from(atob(value), char => char.charCodeAt(0));
const chunkString = bytes => {{
  let value = '';
  for (let i=0; i<bytes.length; i+=32768) value += String.fromCharCode(...bytes.subarray(i,i+32768));
  return btoa(value);
}};
const document = {{ createElement() {{
  const canvas = new OffscreenCanvas(1,1);
  canvas.toBlob = (callback, type) => canvas.convertToBlob({{ type }}).then(callback, () => callback(null));
  return canvas;
}} }};
export async function gpuPosterize{function}
"#))?;
    let app = source.join("renderer/app.js");
    let app = fs::read_to_string(app)?;
    let preview = "$('vectorResultPreview').src='file:///'";
    if !app.contains(preview) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Vector result preview changed"));
    }
    fs::write(assets.join("app.js"), app.replace(preview, "$('vectorResultPreview').src=r.preview||'file:///'") + "\ndocument.getElementById('openVectorizerWeb')?.addEventListener('click', () => window.__TAURI__.core.invoke('open_vectorizer_web').catch(error => showToast(String(error))));\n")?;
    let index = fs::read_to_string(source.join("renderer/index.html"))?;
    let iframe = "<iframe id=\"vectorResultPreview\" title=\"Preview Hasil\"></iframe>";
    if !index.contains(iframe) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Vector result preview element changed"));
    }
    fs::write(assets.join("index.html"), index.replace(iframe, "<img id=\"vectorResultPreview\" alt=\"Preview Hasil\">").replace("<button id=\"runVectorBatch\"", "<button id=\"openVectorizerWeb\" type=\"button\" class=\"secondary\">Vectorizer.AI · Web</button><button id=\"runVectorBatch\""))?;
    stage_required_update(&assets)
}

fn stage_required_update(assets: &Path) -> io::Result<()> {
    let lab = Path::new("../ui");
    for name in ["client-update.css", "client-update.js"] {
        copy_asset(&lab.join(name), &assets.join(name))?;
    }
    println!("cargo:rerun-if-changed=../ui/index.html");
    let lab_html = fs::read_to_string(lab.join("index.html"))?;
    let dialog = lab_html
        .split_once("<dialog id=\"requiredUpdate\"")
        .and_then(|(_, rest)| rest.split_once("</dialog>").map(|(dialog, _)| dialog))
        .ok_or_else(|| io::Error::other("Required update dialog missing"))?;
    let markup = format!("<dialog id=\"requiredUpdate\"{dialog}</dialog>\n<script src=\"client-update.js\"></script>\n</body>");
    for name in ["index.html", "game-window.html"] {
        let page = assets.join(name);
        let html = fs::read_to_string(&page)?;
        if html.matches("</head>").count() != 1 || html.matches("</body>").count() != 1 {
            return Err(io::Error::other(format!("Renderer markup changed: {name}")));
        }
        let html = html.replace("</head>", "<link rel=\"stylesheet\" href=\"client-update.css\">\n</head>");
        fs::write(page, html.replace("</body>", &markup))?;
    }
    Ok(())
}

fn main() {
    stage_renderer().expect("Stage ChatX v8.1.4 renderer");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let profile = output.ancestors().nth(3).expect("Cargo profile directory");
    let vips = Path::new("../../source-v3.7.11/ChatX/node_modules/@img/sharp-win32-x64");
    copy_asset(&vips.join("lib/libvips-42.dll"), &profile.join("libvips-42.dll"))
        .expect("Stage native Upscale detail pass");
    copy_asset(&vips.join("LICENSE"), &profile.join("libvips-LICENSE.txt"))
        .expect("Stage libvips license");
    tauri_build::build();
}
