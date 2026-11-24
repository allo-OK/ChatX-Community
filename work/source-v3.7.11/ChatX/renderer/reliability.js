// Network credentials are attached only to the selected HOST origin.
let verifiedOutboxScope='',sessionToken='',outboxBusy=false,pairingRetryTimer=null,rendererClosing=false;window.addEventListener('beforeunload',()=>rendererClosing=true);
const originalFetch=window.fetch.bind(window);
window.fetch=(input,options={})=>{try{const raw=typeof input==='string'?input:input.url;const url=new URL(raw);if(typeof serverBase==='string'&&serverBase&&url.origin===new URL(serverBase).origin){const headers=new Headers(options.headers||(input instanceof Request?input.headers:undefined));if(!headers.has('Authorization'))headers.set('Authorization','Bearer '+(cfg?.isHost?cfg.adminToken:sessionToken));options={...options,headers};}}catch{}return originalFetch(input,options)};
function authenticatedUrl(value){return window.KCAuthenticatedAssetUrl?.(value,{serverBase,cfg,sessionToken})||''}
async function waitForApprovedSession(target,epoch=target?._kcSessionEpoch){
  const current=()=>target===socket&&target.connected&&target._kcSessionEpoch===epoch;
  while(current()){
    const r=await hello(target,epoch);if(!current())return null;
    if(r?.ok){
      const origin=serverBase,trust=cfg?.isHost?{ok:true}:await window.kc.trustHostKey({origin,publicKey:r.hostPublicKey});
      if(!current())return null;
      if(!trust?.ok){setConn(trust?.error||'HOST belum dipercaya',false);return null;}
      if(!cfg?.isHost&&window.kc.outboxRebind){try{const restored=await window.kc.outboxRebind({origin,publicKey:r.hostPublicKey});if(!current())return null;if(!restored?.ok)throw Error(restored?.error||'Antrean belum dipulihkan');verifiedOutboxScope=restored.scope||'';}catch(error){if(!current())return null;verifiedOutboxScope='';techLog('outbox-rebind-failed',{message:error.message});showToast('Antrean HOST lama belum dipulihkan: '+error.message);}}
      sessionToken=r.token;window.kc.setPolicySession?.({origin,token:r.token}).catch(()=>{});window.kcHostFingerprint=r.hostFingerprint;cfg.name=r.name||cfg.name;return r;
    }
    setConn(r?.error||'HOST belum menerima sesi · mencoba kembali',false);
    await new Promise(resolve=>setTimeout(resolve,r?.retryable===true?500:3000));
  }
  return null;
}
function outboxScope(){return verifiedOutboxScope||String(serverBase||lastAuthenticatedServer||'')+'|'+String(cfg?.name||'').toLowerCase()}
function canRetryOutboxRow(row){
  return row.retryable===true||row.code==='ATTACHMENT_UNAVAILABLE'||row.error==='Lampiran tidak valid / akses ditolak';
}
function validateQueuedMessageTarget(payload){
  const id=String(payload?.conversationId||activeConversationId||'general'),list=typeof conversations!=='undefined'&&Array.isArray(conversations)?conversations:[],conv=list.find(c=>String(c.id)===id);if(!conv)return;
  const norm=v=>String(v||'').trim().toLowerCase().replace(/\s+/g,' '),text=String(payload?.text||'').toLowerCase();
  const peer=conv.type==='dm'?(conv.members||[]).find(n=>norm(n)!==norm(cfg?.name)):'';
  if(conv.type==='dm'){
    const people=[...(conv.members||[]),...editorDirectory.map(u=>u.name),...onlineUsers.map(u=>u.name)].filter(Boolean);
    const handles=[{name:null,handle:'everyone'},{name:null,handle:'editor'},
      ...people.map(name=>({name,handle:norm(name)})),
      ...people.map(name=>({name,handle:norm(profileDisplayName(name))}))]
      .filter(item=>item.handle).sort((a,b)=>b.handle.length-a.handle.length);
    let invalid;
    for(const {index} of text.matchAll(/@/g)){
      const tail=text.slice(index+1);
      const match=handles.find(({handle})=>tail.startsWith(handle)&&!/[\p{L}\p{N}_]/u.test(tail.slice(handle.length,handle.length+1)));
      if(match?.name===null)throw Error('@everyone dan @editor hanya bisa dipakai di chat grup.');
      if(match&&norm(match.name)!==norm(peer)){invalid=match.name;break;}
    }
    if(invalid)throw Error(`Chat pribadi ini hanya bisa men-tag ${peer||'lawan chat ini'}. Untuk men-tag ${invalid}, kirim lewat grup yang sama.`);
    if(payload?.workTagTarget?.type==='user'&&norm(payload.workTagTarget.name)!==norm(peer))throw Error('Target tag harus lawan chat ini.');
    if(payload?.workTagTarget?.type==='all')throw Error('Tag semua orang hanya bisa dikirim ke chat grup.');
  }else if(payload?.workTagTarget?.type==='user'){
    const members=conv.type==='general'?(typeof editorDirectory!=='undefined'?editorDirectory:[]).map(u=>u.name):(conv.members||[]);
    if(!members.some(name=>norm(name)===norm(payload.workTagTarget.name)))throw Error('Target tag harus anggota percakapan ini.');
  }
}
let outboxAttachmentDb;
function outboxAttachment(action, scope, id, file){
  outboxAttachmentDb ??= new Promise((resolve,reject)=>{
    const request=indexedDB.open('chatx-outbox-attachments',1);
    request.onupgradeneeded=()=>request.result.createObjectStore('files');
    request.onsuccess=()=>resolve(request.result);
    request.onerror=()=>reject(request.error);
  }).catch(error=>{outboxAttachmentDb=null;throw error;});
  return outboxAttachmentDb.then(db=>new Promise((resolve,reject)=>{
    const transaction=db.transaction('files',action==='get'?'readonly':'readwrite');
    const files=transaction.objectStore('files'),key=[scope,id];
    const request=action==='get'?files.get(key):action==='put'?files.put(file,key):files.delete(key);
    transaction.oncomplete=()=>resolve(request.result);
    transaction.onabort=()=>reject(transaction.error||Error('Penyimpanan lampiran dibatalkan'));
  }));
}
async function removeOutboxOriginal(scope,id){
  try{await outboxAttachment('delete',scope,id);}
  catch(error){techLog('outbox-attachment-cleanup-failed',{message:error.message});}
}
async function queueMessage(payload, originalFile){
  if(!sessionReady&&!lastAuthenticatedServer&&!verifiedOutboxScope)throw Error('Hubungkan ke HOST sekali dahulu');
  validateQueuedMessageTarget(payload);
  const message={...payload,clientMessageId:payload.clientMessageId||crypto.randomUUID()};
  const scope=outboxScope();
  if(originalFile){
    try{await outboxAttachment('put',scope,message.clientMessageId,originalFile);}
    catch(error){techLog('outbox-attachment-save-failed',{message:error.message});showToast('Cadangan lampiran tidak bisa disimpan di PC ini.');}
  }
  try{await window.kc.outboxPut({scope,payload:message});}
  catch(error){if(originalFile)await removeOutboxOriginal(scope,message.clientMessageId);throw error;}
  void flushOutbox();void renderOutbox();
  return{ok:true,queued:true,clientMessageId:message.clientMessageId};
}
async function flushOutbox(){
  if(outboxBusy||!socket?.connected||!sessionReady||!sessionToken)return;
  const target=socket,epoch=target._kcSessionEpoch,scope=outboxScope();
  const current=()=>target===socket&&target.connected&&sessionReady&&target._kcSessionEpoch===epoch&&outboxScope()===scope;
  outboxBusy=true;
  try{
    const rows=await window.kc.outboxList(scope);
    for(const row of rows){
      if(!current())break;if(row.state==='failed')continue;
      const id=row.payload.clientMessageId;
      await window.kc.outboxUpdate({scope,id,patch:{state:'sending',attempts:row.attempts+1}});
      if(!current())break;
      const send=payload=>new Promise(resolve=>target.timeout(10000).emit('message',payload,(err,r)=>resolve(err?{ok:false,retryable:true,error:'Konfirmasi belum diterima; akan dicoba lagi'}:r)));
      let r=await send(row.payload);
      if(current()&&!row.attachmentRetried&&row.payload.file&&
        (r?.code==='ATTACHMENT_UNAVAILABLE'||r?.error==='Lampiran tidak valid / akses ditolak')){
        try{
          const original=await outboxAttachment('get',scope,id);
          if(!current())break;
          if(original){
            await window.kc.outboxUpdate({scope,id,patch:{state:'uploading'}});
            void renderOutbox();
            const file=await uploadFile(original,false);
            const payload={...row.payload,file};
            await window.kc.outboxUpdate({scope,id,patch:{payload,state:'waiting',attachmentRetried:true}});
            if(!current())break;
            r=await send(payload);
          }
        }catch(error){
          r={ok:false,code:'ATTACHMENT_UNAVAILABLE',error:'Upload ulang gagal: '+error.message};
        }
      }
      if(r?.ok){await window.kc.outboxRemove({scope,id});if(row.payload.file)await removeOutboxOriginal(scope,id);}
      else{await window.kc.outboxUpdate({scope,id,patch:{state:r?.retryable?'waiting':'failed',retryable:r?.retryable===true,code:r?.code||'',error:r?.error||'Pesan gagal'}});if(r?.retryable)break;}
    }
  }catch(e){showToast('Antrean pesan: '+e.message)}finally{outboxBusy=false;void renderOutbox()}
}
async function renderOutbox(){if(rendererClosing||!cfg||(!serverBase&&!lastAuthenticatedServer&&!verifiedOutboxScope))return;const rows=await window.kc.outboxList(outboxScope());if(rendererClosing||!document)return;let bar=document.getElementById('outboxBar');if(!bar){bar=document.createElement('button');bar.id='outboxBar';bar.className='kc-outbox-bar';document.querySelector('.composer-wrap')?.prepend(bar);bar.onclick=openOutbox;}bar.hidden=!rows.length;bar.textContent=rows.some(r=>r.state==='failed')?'Pesan belum terkirim · klik detail':rows.some(r=>r.state==='uploading')?'Mengunggah ulang lampiran':rows.some(r=>r.state==='sending')?'Menunggu konfirmasi HOST':rows.length+' pesan dalam antrean';}
async function openOutbox(){
 const scope=outboxScope(),rows=await window.kc.outboxList(scope);
 const html=rows.map(r=>{
  const c=(typeof conversations!=='undefined'?conversations:[]).find(x=>String(x.id)===String(r.payload.conversationId||'general'));
  let chatName='Chat sudah tidak tersedia';if(c){chatName=c.type==='dm'?((c.members||[]).find(n=>String(n).toLowerCase()!==String(cfg?.name||'').toLowerCase())||'Chat pribadi'):(c.name||'Grup');}
  const action=r.state==='failed'?`${canRetryOutboxRow(r)?`<button data-retry="${esc(r.payload.clientMessageId)}">Coba lagi</button>`:''}<button data-dismiss-outbox="${esc(r.payload.clientMessageId)}">Hapus pesan gagal</button>`:'';
  return `<article><b>${esc(r.state==='failed'?'Gagal':r.state==='uploading'?'Mengunggah ulang':r.state==='sending'?'Menunggu konfirmasi HOST':'Dalam antrean')}</b><p>${esc(r.payload.text||r.payload.file?.name||'Lampiran')}</p><small>Tujuan: ${esc(chatName)} · ${esc(r.error||'Tersimpan di PC ini')}</small>${action}</article>`;
 }).join('');
 showReliabilityPanel('Antrean pesan',html||'<p>Semua pesan sudah tersimpan di HOST.</p>');
 document.querySelectorAll('[data-retry]').forEach(b=>b.onclick=async()=>{await window.kc.outboxUpdate({scope,id:b.dataset.retry,patch:{state:'waiting',attachmentRetried:false,retryable:true,error:''}});void flushOutbox();void openOutbox()});
 document.querySelectorAll('[data-dismiss-outbox]').forEach(b=>b.onclick=async()=>{await window.kc.outboxRemove({scope,id:b.dataset.dismissOutbox});await removeOutboxOriginal(scope,b.dataset.dismissOutbox);void openOutbox()});
}
function showReliabilityPanel(title,html){let panel=document.getElementById('reliabilityPanel');if(!panel){panel=document.createElement('dialog');panel.id='reliabilityPanel';document.body.append(panel)}panel.innerHTML=`<header><h3>${esc(title)}</h3><button id="closeReliability">Tutup</button></header><div>${html}</div>`;document.getElementById('closeReliability').onclick=()=>panel.close();if(!panel.open)panel.showModal();}
async function openDevices(){try{const j=await(await fetch(serverBase+'/admin/devices')).json();if(!j.ok)throw Error(j.error);showReliabilityPanel('Akses CLIENT',`<p>CLIENT pada jaringan yang sama tersambung otomatis. HOST dapat mencabut akses perangkat dari daftar ini.</p><button id="refreshDevices">Refresh</button><h4>Perangkat terdaftar</h4>${j.devices.map(d=>`<article><b>${esc(d.name)} · ${esc(d.pc)}</b><p>${d.revoked?'Akses dicabut':'Akses aktif'}</p><button data-revoke="${esc(d.deviceId)}">Cabut akses</button><button data-remove="${esc(d.deviceId)}">Aktifkan kembali</button></article>`).join('')||'<p>Belum ada perangkat.</p>'}`);document.getElementById('refreshDevices').onclick=openDevices;for(const kind of ['revoke','remove'])document.querySelectorAll('[data-'+kind+']').forEach(b=>b.onclick=async()=>{if(!confirm(kind==='remove'?'Aktifkan kembali akses perangkat ini?':'Cabut akses perangkat ini?'))return;const id=b.dataset[kind],body={deviceId:id,remove:kind==='remove'};const r=await(await fetch(serverBase+'/admin/devices',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)})).json();if(!r.ok)showToast(r.error);void openDevices()});}catch(e){showToast(e.message)}}
async function openActivityVisibility(subjectDeviceId){try{if(!cfg?.isHost)throw Error('Pengaturan akses Live Activity hanya tersedia di HOST');const id=String(subjectDeviceId||''),r=await hostAdminRequest('/admin/activity-visibility?subjectDeviceId='+encodeURIComponent(id),{cache:'no-store'}),j=await r.json();if(!r.ok||!j.ok)throw Error(j.error||'Pengaturan activity gagal dimuat');const selected=new Set(j.policy.viewerDeviceIds||[]),people=j.users.map(u=>`<label class="activity-visibility-person"><input type="checkbox" data-activity-viewer="${esc(u.deviceId)}" ${selected.has(u.deviceId)?'checked':''}><span><b>${esc(u.name)}</b><small>${esc(u.pc||'PC')}${u.online?' · online':' · offline'}${u.moderationRole==='ADMIN'?' · Admin':''}</small></span></label>`).join('');showReliabilityPanel('Akses Live Activity · '+j.subject.name,`<p>Activity tetap mengikuti pilihan editor. Pengecualian ini hanya membagikan info aplikasi/media yang aktif, bukan layar atau input keyboard.</p><label class="activity-visibility-person"><input id="activityIncludeAdmins" type="checkbox" ${j.policy.includeAdmins?'checked':''}><span><b>HOST dan Admin</b><small>Izinkan HOST serta akun dengan role Admin melihat saat sharing dimatikan.</small></span></label><h4>Orang tertentu</h4><div class="activity-visibility-people">${people||'<p>Belum ada CLIENT lain yang terdaftar.</p>'}</div><button id="saveActivityVisibility" class="primary">Simpan akses</button>`);document.getElementById('saveActivityVisibility').onclick=async()=>{const viewerDeviceIds=[...document.querySelectorAll('[data-activity-viewer]:checked')].map(x=>x.dataset.activityViewer),body={subjectDeviceId:id,includeAdmins:!!document.getElementById('activityIncludeAdmins')?.checked,viewerDeviceIds},savedResponse=await hostAdminRequest('/admin/activity-visibility',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)}),saved=await savedResponse.json();if(!savedResponse.ok||!saved.ok)return showToast(saved.error||'Pengaturan gagal disimpan');showToast('Akses Live Activity diperbarui.');document.getElementById('reliabilityPanel')?.close();};}catch(e){showToast(e.message||'Pengaturan Live Activity gagal dibuka')}}
async function openQuarantine(){try{const j=await(await fetch(serverBase+'/storage/quarantine')).json();if(!j.ok)throw Error(j.error);showReliabilityPanel('Pulihkan lampiran',j.items.map(f=>`<article><b>${esc(f.stored)}</b><p>${fmtSize(f.size)} · ${new Date(f.at).toLocaleString('id-ID')}</p><button data-restore-file="${f.id}">Pulihkan</button></article>`).join('')||'<p>Karantina kosong.</p>');document.querySelectorAll('[data-restore-file]').forEach(b=>b.onclick=async()=>{const r=await(await fetch(serverBase+'/storage/quarantine/restore',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:b.dataset.restoreFile})})).json();showToast(r.ok?'Lampiran dipulihkan':r.error);void openQuarantine()});}catch(e){showToast(e.message)}}
function mountReliabilityControls(){const mount=document.getElementById('systemModal')?.querySelector('.modal-card')||document.getElementById('systemModal');if(mount&&!document.getElementById('deviceManagerButton')){const box=document.createElement('div');box.className='reliability-actions';box.innerHTML='<button id="deviceManagerButton">Perangkat CLIENT</button><button id="quarantineButton">Pulihkan lampiran</button><button id="restoreBackupButton">Uji / pulihkan backup ke folder baru</button>';mount.append(box);document.getElementById('deviceManagerButton').onclick=openDevices;document.getElementById('quarantineButton').onclick=openQuarantine;document.getElementById('restoreBackupButton').onclick=async()=>{const r=await window.kc.restoreBackup();showToast(r?.ok?'Backup diverifikasi dan dipulihkan ke '+r.path:r?.error||'Dibatalkan',6000)};}void renderOutbox();}
setInterval(()=>{if(typeof cfg!=='undefined'&&cfg&&typeof serverBase!=='undefined'){void flushOutbox();mountReliabilityControls()}},4000);

// v2.1.5 — every HTTP request to the active ChatX server carries the
// current authenticated session. This keeps profile/avatar/file uploads alive
// after manual reconnect and also works with legacy v2.0.x peers.
async function kcFetch(input,init={}){
  const opts={...(init||{})};
  try{
    const raw=typeof input==='string'?input:(input?.url||'');
    const u=new URL(raw,serverBase||location.href);
    const base=serverBase?new URL(serverBase):null;
    if(base&&u.origin===base.origin){
      const token=(cfg?.isHost&&base.origin===new URL(cfg?.serverAddress||serverBase).origin&&cfg?.adminToken)?cfg.adminToken:sessionToken;
      if(token){const h=new Headers(opts.headers||{});if(!h.has('Authorization'))h.set('Authorization','Bearer '+token);opts.headers=h;}
    }
  }catch{}
  return fetch(input,opts);
}
