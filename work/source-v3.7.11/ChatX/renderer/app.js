let cfg, socket, serverBase, selectedFile=null, selectedFiles=[], unread=0, onlineUsers=[], mentionIndex=0, myProfile=null, myPolicy={moderationRole:'EDITOR',roleLabel:'Editor',permissions:{personal:true,editor:false,everyone:false},mutedUntil:0,bypassAppMute:true,bypassWindowsMute:false}, currentReply=null, editingMessage=null;
let chatImageSequence=[],chatImageIndex=0,chatImageWorkspaceOpen=false,chatImageLayoutRaf=0;
let reconnectTimer=null, rediscoverTimer=null, connecting=false, reconnectQueued=false, connectGeneration=0, connectAttempt=0, sessionReady=false, lastAuthenticatedServer='', socketConnectErrorStreak=0, discoveryRetryAttempt=0, disconnectedSince=Date.now(), lastWatchdogRecoveryAt=0, pendingClientUpdate=null, publishedInstallerFile=null;
let hostPeerMonitor=null,clientPrimaryMonitor=null,clientPrimaryBusy=false,hostHandoverBusy=false,hostHandoverActive=false,hostHandoverReleaseTimer=null,lastPrimaryNotice='',currentServerTemporary=false;
let sendingSticker=false;
let messageStore=[];
let historyHasMore=false,historyLoadingOlder=false,chatScrolledAway=false,newMessagesWhileScrolled=0;
let conversationScrollPositions={},scrollRestoreInProgress=false,scrollSaveTimer=null;
let remoteTyping=new Map(),typingStopTimer=null,lastTypingEmitAt=0;
let conversations=[], editorDirectory=[], activeConversationId=null, conversationFilter='all', conversationMenuTarget=null, messageContextTarget=null;
let conversationPrefs={pins:[],favorites:[],mutes:[],archives:[],deleted:[]};
let appearancePrefs={accent:'#1765cb',peerAlign:'left',sidebarWidth:360,fontFamily:'SF Pro Display'};
const TEAM_ROLES=['EDITOR','ADMIN','QC','FLEX'];
function normalizeTeamRole(v){const r=String(v||'').replace(/[\r\n\t<>]/g,' ').trim().slice(0,40).toUpperCase();return r||(cfg?.isHost?'KOORDINATOR':'EDITOR');}
function defaultWorkTagPrefs(){return{'print-test':true,'design-revision':true,'design-request':true};}
function normalizeWorkTagPrefs(v){const d=defaultWorkTagPrefs(),x=v&&typeof v==='object'?v:{};return{'print-test':x['print-test']!==false,'design-revision':x['design-revision']!==false,'design-request':x['design-request']!==false};}
function myTeamRole(){return normalizeTeamRole(cfg?.teamRole||cfg?.userRole);}
function myWorkTagPrefs(){return normalizeWorkTagPrefs(cfg?.workTagPrefs);}
let hiddenMessageIds=new Set();
const $=id=>document.getElementById(id);
async function refreshComputeHints(){try{const c=await window.kc.getComputeCapabilities?.();if(!c?.ok)return;window.__kcComputeCaps=c;}catch{}}
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const fmtSize=n=>n>=1e9?(n/1e9).toFixed(2)+' GB':n>=1e6?(n/1e6).toFixed(1)+' MB':n>=1e3?(n/1e3).toFixed(0)+' KB':n+' B';
const isImage=f=>f&&((f.mimetype||'').startsWith('image/')||/\.(png|jpe?g|gif|webp|svg)$/i.test(f.name||''));
const fullUrl=u=>authenticatedUrl(u);
const notifDefaults={normalSize:'medium',position:'top-right',normalSound:'none',style:'card'};
let toastTimer=null;
function showToast(message,duration=2600){
  let el=document.getElementById('kcToast');
  if(!el){el=document.createElement('div');el.id='kcToast';el.className='kc-toast';document.body.appendChild(el);}
  el.textContent=String(message||'');el.classList.add('show');clearTimeout(toastTimer);toastTimer=setTimeout(()=>el.classList.remove('show'),Math.max(1200,Number(duration)||2600));
}
function insertAtCursor(value){const el=$('text');if(!el)return;const v=String(value||''),s=Number.isInteger(el.selectionStart)?el.selectionStart:el.value.length,e=Number.isInteger(el.selectionEnd)?el.selectionEnd:s;el.value=el.value.slice(0,s)+v+el.value.slice(e);const pos=s+v.length;el.focus();el.setSelectionRange(pos,pos);el.dispatchEvent(new Event('input',{bubbles:true}));}

const bugEventRing=[];const bugThrottle=new Map();
function techLog(type,detail={}){try{recordTechEvent(type,detail);const p=window.kc?.logEvent?.({type,detail});if(p&&typeof p.catch==='function')p.catch(()=>{})}catch{}}
function recordTechEvent(type,meta={}){const clean={type:String(type||'event').slice(0,80),ts:Date.now()};for(const[k,v]of Object.entries(meta||{}).slice(0,12)){if(/text|body|content|base64/i.test(k))continue;clean[k]=typeof v==='object'?JSON.stringify(v).slice(0,300):String(v).slice(0,300)}bugEventRing.push(clean);if(bugEventRing.length>20)bugEventRing.shift();}
function reportBug(type,error,severity='error',extra={}){try{const msg=error?.message||String(error||type),key=type+'|'+msg.slice(0,160),now=Date.now(),cooldown=/^socket-(connect-error|disconnect)$/.test(String(type))?5*60*1000:30000;if((bugThrottle.get(key)||0)>now-cooldown)return;bugThrottle.set(key,now);const payload={type,source:'renderer',severity,message:msg,stack:error?.stack||'',context:{...extra,lastEvents:bugEventRing.slice(-12)}};const pr=window.kc?.reportBug?.(payload);if(pr&&typeof pr.catch==='function')pr.catch(()=>{})}catch{}}
window.addEventListener('error',e=>reportBug('window-error',e.error||new Error(e.message||'Window error'),'error',{filename:e.filename||'',line:e.lineno||0,column:e.colno||0}));
window.addEventListener('unhandledrejection',e=>{const r=e.reason instanceof Error?e.reason:new Error(String(e.reason||'Unhandled rejection'));reportBug('unhandledrejection',r,'error')});
function versionParts(v){return String(v||'0').split('.').map(x=>parseInt(x,10)||0)}
function newerVersion(a,b){const A=versionParts(a),B=versionParts(b),n=Math.max(A.length,B.length);for(let i=0;i<n;i++){if((A[i]||0)!==(B[i]||0))return(A[i]||0)>(B[i]||0)}return false;}
function editConversationIdentity(c){window.KCGroupIdentity.open(c,async values=>{const result=await socketAckEvent(c.type==='general'?'conversation-general-update':'conversation-group-update',{id:c.id,...values});if(!result?.ok){const error=new Error(result?.error||'Gagal mengubah grup');reportBug('group-identity-update-failed',error);throw error;}Object.assign(c,values);$('chatInfoModal').classList.add('hidden');requestConversationList();if(activeConversationId===c.id)openConversation(c.id);showToast('Nama dan deskripsi grup diperbarui.');});}
function appConfirm({title='Konfirmasi',text='',ok='OK',cancel='Batal'}={}){return new Promise(resolve=>{const modal=$('confirmModal'),okBtn=$('confirmOk'),cancelBtn=$('confirmCancel');$('confirmTitle').textContent=title;$('confirmText').textContent=text;okBtn.textContent=ok;cancelBtn.textContent=cancel;let done=false;const finish=v=>{if(done)return;done=true;modal.classList.add('hidden');okBtn.onclick=null;cancelBtn.onclick=null;setTimeout(()=>$('text')?.focus(),0);resolve(v);};okBtn.onclick=()=>finish(true);cancelBtn.onclick=()=>finish(false);modal.classList.remove('hidden');setTimeout(()=>okBtn.focus(),0);});}
async function checkClientUpdate(){if(cfg?.isHost||!serverBase)return;const updateOrigin=serverBase;try{const j=window.kc.checkRustClientUpdate?await window.kc.checkRustClientUpdate():await (await kcFetch(updateOrigin+'/client-update/status',{cache:'no-store'})).json();if(!j||serverBase!==updateOrigin)return;if(j.ok&&j.available&&j.manifest)await window.kc.setRequiredUpdate?.({origin:updateOrigin,manifest:j.manifest,notes:j.notes,changelog:j.changelog});if(j.ok&&j.available&&newerVersion(j.version,cfg.version)){pendingClientUpdate=j;window.KCUpdateChangelog?.show(j);if(!window.KCUpdateChangelog)$('updateBannerText').textContent=`Versi ${j.version} tersedia${j.notes?' · '+j.notes:''}`;$('updateBanner').classList.remove('hidden');}}catch{}}

function themeStorageKey(){return'kc-theme:'+String(cfg?.name||'setup').toLowerCase();}
function applyTheme(theme,save=false){const t=theme==='light'?'light':'dark';try{window.kc?.setNativeTheme?.(t)?.catch?.(()=>{});}catch{}document.documentElement.dataset.theme=t;document.body.dataset.theme=t;document.body.classList.toggle('dark',t==='dark');document.body.classList.toggle('light',t==='light');const btn=$('themeButton');if(btn){btn.textContent=t==='light'?'☾':'☀';btn.title=t==='light'?'Pakai Dark Mode':'Pakai Light Mode';}if(save)try{localStorage.setItem(themeStorageKey(),t)}catch{}}
function loadTheme(){let t='dark';try{t=localStorage.getItem(themeStorageKey())||'dark'}catch{}applyTheme(t,false);}
window.addEventListener('storage',e=>{try{if(e.key===themeStorageKey()&&e.newValue)applyTheme(e.newValue,false)}catch{}});


function convPrefKey(){return'kc-conversation-prefs:'+String(cfg?.name||'setup').toLowerCase();}
function loadConversationPrefs(){try{conversationPrefs={pins:[],favorites:[],mutes:[],archives:[],deleted:[],...(JSON.parse(localStorage.getItem(convPrefKey())||'{}')||{})};}catch{conversationPrefs={pins:[],favorites:[],mutes:[],archives:[],deleted:[]};}}
function saveConversationPrefs(){try{localStorage.setItem(convPrefKey(),JSON.stringify(conversationPrefs));}catch{}renderConversationList();}
function appearanceKey(){return'kc-appearance:'+String(cfg?.name||'setup').toLowerCase();}
function loadAppearancePrefs(){try{appearancePrefs={accent:'#1765cb',peerAlign:'left',sidebarWidth:360,fontFamily:'SF Pro Display',...(JSON.parse(localStorage.getItem(appearanceKey())||'{}')||{})};}catch{}applyAppearancePrefs(false);}
function saveAppearancePrefs(){try{localStorage.setItem(appearanceKey(),JSON.stringify(appearancePrefs));}catch{}applyAppearancePrefs(false);}
function hiddenMessageKey(){return'kc-hidden-messages:'+String(cfg?.name||'setup').toLowerCase();}
function conversationScrollKey(){return'kc-chat-scroll:'+String(cfg?.name||'setup').toLowerCase();}
function loadConversationScrollPositions(){try{const v=JSON.parse(localStorage.getItem(conversationScrollKey())||'{}');conversationScrollPositions=v&&typeof v==='object'?v:{}}catch{conversationScrollPositions={}}}
function captureConversationScrollPosition(){const box=$('messages');if(!box||!activeConversationId)return null;const id=String(activeConversationId),nearBottom=chatDistanceFromBottom()<=150;let anchor=null,offset=0;if(!nearBottom){const boxTop=box.getBoundingClientRect().top;for(const row of box.querySelectorAll('.msg-row[data-message-id]')){const rect=row.getBoundingClientRect();if(rect.bottom>boxTop+1){anchor=String(row.dataset.messageId||'');offset=Math.round(rect.top-boxTop);break;}}}return{top:Math.max(0,Math.round(box.scrollTop)),anchor,offset,nearBottom,updatedAt:Date.now()};}
function saveConversationScrollPosition(){if(!activeConversationId||scrollRestoreInProgress)return;const saved=captureConversationScrollPosition();if(!saved)return;conversationScrollPositions[String(activeConversationId)]=saved;clearTimeout(scrollSaveTimer);scrollSaveTimer=setTimeout(()=>{try{localStorage.setItem(conversationScrollKey(),JSON.stringify(conversationScrollPositions))}catch{}},260);}
function flushConversationScrollPosition(){if(!activeConversationId)return;const saved=captureConversationScrollPosition();if(saved)conversationScrollPositions[String(activeConversationId)]=saved;clearTimeout(scrollSaveTimer);scrollSaveTimer=null;try{localStorage.setItem(conversationScrollKey(),JSON.stringify(conversationScrollPositions))}catch{}}
function restoreConversationScrollPosition(id,saved){const box=$('messages');if(!box||String(activeConversationId)!==String(id))return;if(saved&&typeof saved==='object'&&saved.nearBottom){box.scrollTop=box.scrollHeight;return;}const anchorId=saved&&typeof saved==='object'?String(saved.anchor||''):'';const anchor=anchorId&&[...box.querySelectorAll('.msg-row[data-message-id]')].find(row=>String(row.dataset.messageId)===anchorId);if(anchor){const delta=anchor.getBoundingClientRect().top-box.getBoundingClientRect().top-(Number(saved.offset)||0);box.scrollTop=Math.max(0,box.scrollTop+delta);return;}const top=typeof saved==='number'?saved:Number(saved?.top)||0;box.scrollTop=Math.min(top,Math.max(0,box.scrollHeight-box.clientHeight));}
function loadHiddenMessages(){try{hiddenMessageIds=new Set(JSON.parse(localStorage.getItem(hiddenMessageKey())||'[]').map(String));}catch{hiddenMessageIds=new Set();}}
function saveHiddenMessages(){try{localStorage.setItem(hiddenMessageKey(),JSON.stringify([...hiddenMessageIds].slice(-8000)));}catch{}}
function hideMessagesLocal(ids){for(const id of ids||[])if(id!=null)hiddenMessageIds.add(String(id));saveHiddenMessages();renderCurrentMessages(false);}
function isMessageHidden(m){return !!m?.id&&hiddenMessageIds.has(String(m.id));}
function applyAppearancePrefs(syncControls=true){const preset=['green','custom'].includes(appearancePrefs.preset)?appearancePrefs.preset:(!appearancePrefs.preset&&/^#[0-9a-f]{6}$/i.test(appearancePrefs.accent||'')&&!['#1765cb','#128c4a','#25d366'].includes(appearancePrefs.accent.toLowerCase())?'custom':'blue');const accent=preset==='green'?'#128c4a':preset==='custom'&&/^#[0-9a-f]{6}$/i.test(appearancePrefs.accent||'')?appearancePrefs.accent:'#1765cb';const align=['left','center','right'].includes(appearancePrefs.peerAlign)?appearancePrefs.peerAlign:'left';const width=Math.max(280,Math.min(520,Number(appearancePrefs.sidebarWidth)||360));const fontFamily=String(appearancePrefs.fontFamily||'SF Pro Display').slice(0,120);appearancePrefs={preset,accent,peerAlign:align,sidebarWidth:width,fontFamily};document.documentElement.dataset.accentPreset=preset;document.documentElement.style.setProperty('--kc-accent',accent);document.documentElement.style.setProperty('--conversation-width',width+'px');document.documentElement.style.setProperty('--conversation-sidebar-width',width+'px');document.documentElement.style.setProperty('--kc-font',`\"${fontFamily.replace(/\"/g,'')}\", \"SF Pro Display\", \"Segoe UI\", Arial, sans-serif`);document.body.dataset.peerAlign=align;if(syncControls){if(document.getElementById('accentPreset'))document.getElementById('accentPreset').value=preset;if($('accentColor'))$('accentColor').value=accent;if($('peerMessageAlign'))$('peerMessageAlign').value=align;if($('sidebarWidthRange'))$('sidebarWidthRange').value=String(width);if($('sidebarWidthValue'))$('sidebarWidthValue').textContent=width+' px';if($('appFontFamily'))$('appFontFamily').value=fontFamily;}}
function prefHas(kind,id){return (conversationPrefs[kind]||[]).includes(String(id));}
function toggleConvPref(kind,id){id=String(id);const a=conversationPrefs[kind]||[];conversationPrefs[kind]=a.includes(id)?a.filter(x=>x!==id):[...a,id];saveConversationPrefs();}
function profileDisplayName(name){
  const key=String(name||'').trim().toLowerCase();
  if(key===String(cfg?.name||'').trim().toLowerCase())return myProfile?.displayName||cfg?.displayName||name;
  return conversationProfileCache.get(key)?.displayName||onlineUsers.find(u=>String(u.name||'').trim().toLowerCase()===key)?.displayName||editorDirectory.find(u=>String(u.name||'').trim().toLowerCase()===key)?.displayName||name;
}
function conversationDisplayTitle(c){const name=displayConversationName(c);return c?.type==='dm'?profileDisplayName(name):name;}
function currentProfileAvatar(name, fallback){
  const profile=conversationProfileCache.get(profileKeyLocal(name));
  return profile && Object.hasOwn(profile, "avatarUrl") ? profile.avatarUrl : fallback;
}
function messageDisplayText(message){
  const text=message.text||'';
  const recipients=message.mentionRecipients;
  if(!Array.isArray(recipients)||!recipients.length)return text;
  const accounts=recipients.slice().sort((a,b)=>b.length-a.length);
  return text.replace(/@[^@]*/g,part=>{
    const account=accounts.find(name=>part.slice(1,1+name.length).toLowerCase()===name.toLowerCase()
      && !/[\p{L}\p{N}_]/u.test(part.slice(1+name.length,2+name.length)));
    return account?'@'+profileDisplayName(account)+part.slice(1+account.length):part;
  });
}
function refreshVisibleProfileNames(updatedKey){
  const byId=new Map(messageStore.map(m=>[String(m.id),m]));
  document.querySelectorAll('.msg-row[data-message-id]').forEach(row=>{
    const m=byId.get(row.dataset.messageId),label=row.querySelector('.sender');
    if(m&&label)label.textContent=m.sender===cfg.name?'Kamu':profileDisplayName(m.sender);
    if(updatedKey&&m?.mentionRecipients?.some(name=>profileKeyLocal(name)===updatedKey)){
      const body=row.querySelector('.bubble > .body');
      if(body)body.textContent=messageDisplayText(m);
    }
    if(m && profileKeyLocal(m.sender)===updatedKey){
      const avatar=row.querySelector(':scope > .message-avatar');
      if(avatar)avatar.outerHTML=avatarHtml(currentProfileAvatar(m.sender,m.senderAvatar),m.sender);
    }
  });
}
function refreshProfileNameUI(){
  const name=profileDisplayName(cfg.name);
  if($('myName'))$('myName').textContent=name+(cfg.isHost?' · HOST':'');
  if($('profileNameLabel'))$('profileNameLabel').textContent=name;
  if($('profileDisplayName')&&document.activeElement!==$('profileDisplayName'))$('profileDisplayName').value=name;
  renderDiscordProfileBar();
}
$('saveProfileName')?.addEventListener('click',async()=>{
  const button=$('saveProfileName');if(button.disabled)return;button.disabled=true;
  try{
    const displayName=String($('profileDisplayName').value||'').replace(/\s+/g,' ').trim();
    if(!displayName||displayName.length>40)throw Error('Isi nama profil, maksimal 40 karakter.');
    const response=await kcFetch(serverBase+'/profile/'+encodeURIComponent(cfg.name)+'/name',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({displayName})});
    const result=await response.json();if(!response.ok||!result.ok)throw Error(result.error||'Nama belum tersimpan.');
    await window.kc.saveConfig({displayName:result.profile.displayName});cfg.displayName=result.profile.displayName;myProfile=result.profile;
    refreshProfileNameUI();renderConversationList();showToast('Nama profil disimpan.');
  }catch(e){showToast(e.message||'Nama belum tersimpan. Coba lagi.');}finally{button.disabled=false;}
});
function displayConversationName(c){if(!c)return'Chat';if(c.type==='dm'){const other=(c.members||[]).find(n=>String(n).toLowerCase()!==String(cfg?.name||'').toLowerCase());return other||'Chat Pribadi';}return c.name||'Chat';}
function conversationSubtitle(c){if(!c)return'';if(c.type==='general')return'Semua editor kantor';if(c.type==='group')return`${(c.members||[]).length} anggota`;const live=conversationLiveActivity(c);if(live)return`${activityIcon(live)} ${activitySummary(live)}`;const other=displayConversationName(c),u=onlineUsers.find(x=>String(x.name).toLowerCase()===String(other).toLowerCase());return u?'online':'offline';}
function conversationPeerUser(c){if(!c||c.type!=='dm')return null;const other=displayConversationName(c);return (onlineUsers||[]).find(x=>profileKeyLocal(x.name)===profileKeyLocal(other))||(editorDirectory||[]).find(x=>profileKeyLocal(x.name)===profileKeyLocal(other))||null;}
function jobTitle(u){const role=normalizeTeamRole(u?.teamRole||'EDITOR');return (role==='ADMIN'?'Admin CS':role)+(u?.isHost?' · HOST':u?.cohost||Object.values(u?.policy?.cohost||{}).some(Boolean)?' · Co-HOST':'');}
function conversationTeamRole(c){if(!c||c.type!=='dm')return'';const u=conversationPeerUser(c);return jobTitle({...u,teamRole:c.liveTeamRole||u?.teamRole||'EDITOR'});}
function conversationLiveActivity(c){if(!c||c.type!=='dm')return null;if(c.liveActivitySharing!==false&&c.liveActivity)return c.liveActivity;const u=conversationPeerUser(c);if(!u||u.activitySharing===false)return null;if(u.activity)return u.activity;if(u.online)return{kind:'chat',app:'ChatX',title:'Online di ChatX',state:'active'};return null;}
const conversationProfileCache=new Map(),conversationProfileLoading=new Set();
function conversationAvatarUrl(c){if(!c||c.type!=='dm')return c?.avatarUrl||null;const other=displayConversationName(c),key=String(other||'').trim().toLowerCase();const sources=[...(editorDirectory||[]),...(onlineUsers||[])];const src=sources.find(u=>String(u?.name||'').trim().toLowerCase()===key);return currentProfileAvatar(other,src?.avatarUrl||c.avatarUrl||null);}
async function hydrateConversationAvatar(c){if(!c||c.type!=='dm'||!serverBase)return;const name=displayConversationName(c),key=String(name||'').toLowerCase();if(!key||conversationAvatarUrl(c)||conversationProfileCache.has(key)||conversationProfileLoading.has(key))return;conversationProfileLoading.add(key);try{const r=await kcFetch(serverBase+'/profile/'+encodeURIComponent(name),{cache:'no-store'}),j=await r.json();if(j?.profile&&!conversationProfileCache.has(key)){conversationProfileCache.set(key,j.profile);if(j.profile.avatarUrl||j.profile.displayName)renderConversationList();}}catch{}finally{conversationProfileLoading.delete(key);}}
function hydrateConversationAvatars(list){for(const c of list||[])hydrateConversationAvatar(c);}
function convAvatarHtml(c){const name=displayConversationName(c),url=conversationAvatarUrl(c);if(url)return `<div data-profile-photo="${esc(url)}" data-profile-name="${esc(name)}" class="avatar has-photo" style="--avatar-url:url('${esc(fullUrl(url))}');background-image:var(--avatar-url)"></div>`;const ch=c?.type==='group'?'👥':c?.type==='general'?'#':(name||'?')[0].toUpperCase();return `<div class="avatar">${esc(ch)}</div>`;}
const conversationTimeFormat = new Intl.DateTimeFormat('id-ID', {hour:'2-digit', minute:'2-digit'});
const conversationDateFormat = new Intl.DateTimeFormat('id-ID', {day:'2-digit', month:'2-digit'});
let conversationListFrame = 0;
function scheduleConversationList() {
  if (conversationListFrame) return;
  conversationListFrame = requestAnimationFrame(() => {
    conversationListFrame = 0;
    renderConversationList();
  });
}
function fmtConvTime(ts){if(!ts)return'';const d=new Date(ts),now=new Date();if(!Number.isFinite(d.getTime()))return'';if(d.toDateString()===now.toDateString())return conversationTimeFormat.format(d);const y=new Date(now);y.setDate(now.getDate()-1);if(d.toDateString()===y.toDateString())return'Kemarin';return conversationDateFormat.format(d);}
function renderConversationList(){if(conversationListFrame){cancelAnimationFrame(conversationListFrame);conversationListFrame=0;}const box=$('conversationList');if(!box)return;const q=String($('sidebarSearchInput')?.value||'').trim().toLowerCase();const archivedIds=conversationPrefs.archives||[];const archivedCount=conversations.filter(c=>archivedIds.includes(String(c.id))).length;if($('archivedShortcut')){$('archivedShortcut').classList.toggle('hidden',archivedCount===0);$('archivedCount').textContent=String(archivedCount);$('archivedShortcut').classList.toggle('active',conversationFilter==='archived');}
let visible=conversations.filter(c=>!prefHas('deleted',c.id));let list=conversationFilter==='archived'?visible.filter(c=>prefHas('archives',c.id)):visible.filter(c=>!prefHas('archives',c.id));if(conversationFilter==='unread')list=list.filter(c=>Number(c.unread||0)>0||prefHas('manualUnread',c.id));if(conversationFilter==='favorite')list=list.filter(c=>prefHas('favorites',c.id));if(conversationFilter==='group')list=list.filter(c=>c.type==='group');if(q)list=list.filter(c=>[conversationDisplayTitle(c),messageDisplayText({text:c.lastText,mentionRecipients:c.lastMentionRecipients}),profileDisplayName(c.lastSender)].join(' ').toLowerCase().includes(q));list.sort((a,b)=>{const ap=prefHas('pins',a.id)?1:0,bp=prefHas('pins',b.id)?1:0;if(ap!==bp)return bp-ap;return Number(b.lastTs||0)-Number(a.lastTs||0)});const existingRows=new Map([...box.querySelectorAll('.conversation-row')].map(row=>[String(row.dataset.id),row]));if(!list.length){box.innerHTML=`<div class="empty-conversation-list">${conversationFilter==='archived'?'Belum ada chat yang diarsipkan.':'Belum ada chat yang cocok.'}</div>`;return;}hydrateConversationAvatars(list);for(const [index,c] of list.entries()){const row=existingRows.get(String(c.id))||document.createElement('div');existingRows.delete(String(c.id));row.className='conversation-row'+(String(c.id)===String(activeConversationId)?' active':'');row.dataset.id=c.id;const preview=c.lastText?`${c.lastSender&&c.type!=='dm'?esc(profileDisplayName(c.lastSender))+': ':''}${esc(messageDisplayText({text:c.lastText,mentionRecipients:c.lastMentionRecipients}))}`:(c.type==='group'?'Grup baru':c.type==='dm'?'Mulai percakapan':'Semua editor kantor');const live=conversationLiveActivity(c),liveText=live?activitySummary(live):'';const rowHtml=`${convAvatarHtml(c)}<div class="conv-main"><div class="conv-top"><div class="conv-name-wrap"><b>${esc(conversationDisplayTitle(c))}</b>${c.type==='dm'?`<span class="conv-role">${esc(conversationTeamRole(c))}</span>`:''}${live?`<span class="conv-live-dot" title="Live Activity">${activityIcon(live)}</span>`:''}</div><small>${fmtConvTime(c.lastTs)}</small></div>${live?`<div class="conv-activity" title="${esc(liveText)}"><span>${activityIcon(live)}</span>${esc(liveText)}</div>`:''}<div class="conv-bottom"><span class="conv-preview">${preview}</span><span class="conv-badges">${prefHas('pins',c.id)?'<span class="conv-pin">📌</span>':''}${prefHas('mutes',c.id)?'<span class="conv-mute">🔕</span>':''}${prefHas('archives',c.id)?'<span class="conv-archive">🗄</span>':''}${Number(c.unread||0)>0?`<span class="conv-unread">${Number(c.unread)>99?'99+':Number(c.unread)}</span>`:prefHas('manualUnread',c.id)?'<span class="conv-unread conv-unread-dot" aria-label="Ditandai belum dibaca"></span>':''}</span></div></div><button class="conv-more" title="Menu chat">⋮</button>`;if(row._renderHtml!==rowHtml){const template=document.createElement('template');template.innerHTML=rowHtml;const next=[...template.content.children];next.forEach((child,i)=>{const current=row.children[i];if(!current)row.appendChild(child);else if(current.outerHTML!==child.outerHTML)current.replaceWith(child)});while(row.children.length>next.length)row.lastElementChild.remove();row._renderHtml=rowHtml;}row.onclick=e=>{if(e.target.closest('.conv-more')){showConversationMenu(c,e.target.closest('.conv-more'));return;}openConversation(c.id)};row.oncontextmenu=e=>{e.preventDefault();showConversationMenu(c,e)};if(box.children[index]!==row)box.insertBefore(row,box.children[index]||null);}for(const row of existingRows.values())row.remove();box.querySelector('.empty-conversation-list')?.remove();}
function showConversationMenu(c,anchor){
  const menu=$('conversationMenu'); if(!menu||!c)return;
  conversationMenuTarget=c;const unreadAction=menu.querySelector('[data-action=unread]');if(unreadAction)unreadAction.textContent=prefHas('manualUnread',c.id)?'Tandai sudah dibaca':'Tandai belum dibaca';
  menu.querySelectorAll('.group-only').forEach(x=>x.classList.toggle('hidden',c.type!=='group'));
  menu.querySelectorAll('.host-group-only').forEach(x=>x.classList.toggle('hidden',!(cfg?.isHost&&c.type==='group')));
  menu.classList.remove('hidden');
  menu.classList.remove('menu-open');
  // position after layout so the menu never escapes the screen
  requestAnimationFrame(()=>{
    let x=16,y=16;
    if(anchor?.clientX!=null && !anchor.getBoundingClientRect){x=anchor.clientX;y=anchor.clientY;}
    else if(anchor?.getBoundingClientRect){const r=anchor.getBoundingClientRect();x=r.right-8;y=r.bottom+6;}
    const mr=menu.getBoundingClientRect();
    x=Math.max(8,Math.min(window.innerWidth-mr.width-8,x-mr.width+8));
    y=Math.max(8,Math.min(window.innerHeight-mr.height-8,y));
    menu.style.left=x+'px';menu.style.top=y+'px';
    menu.classList.add('menu-open');
  });
}
function requestConversationList(){if(!socket?.connected)return;socket.emit('conversation-list-request',{},r=>{if(r?.ok){conversations=r.items||[];scheduleConversationList();}});}
function activeConversation(){return conversations.find(c=>String(c.id)===String(activeConversationId))||null;}
function conversationTagScope(c=activeConversation()){return c?.type==='dm'?'private':'group';}
function tagScopeLabel(scope){return scope==='private'?'TAG PRIBADI':'TAG GRUP';}
function updateActiveConversationHeader(c){
  if(!c)return;const name=displayConversationName(c),sub=conversationSubtitle(c);
  $('activeChatTitle').textContent=conversationDisplayTitle(c);$('activeChatSubtitle').textContent=sub||'';
  const av=$('activeChatAvatar');if(av){const url=conversationAvatarUrl(c);av.classList.toggle('has-photo',!!url);if(url){av.style.setProperty('--avatar-url',`url('${fullUrl(url)}')`);av.style.backgroundImage='var(--avatar-url)';}else{av.style.removeProperty('--avatar-url');av.style.backgroundImage='';}av.textContent=url?'':(c.type==='group'?'👥':c.type==='general'?'#':(name||'?')[0].toUpperCase());}
  $('activeChatPin')?.classList.toggle('active',prefHas('pins',c.id));$('activeChatMute')?.classList.toggle('active',prefHas('mutes',c.id));
}
function messageDayKey(ts){const d=new Date(Number(ts)||Date.now());return`${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`;}
function messageDayLabel(ts){const d=new Date(Number(ts)||Date.now()),now=new Date(),today=messageDayKey(now),yesterdayDate=new Date(now.getFullYear(),now.getMonth(),now.getDate()-1),yesterday=messageDayKey(yesterdayDate),key=messageDayKey(d);if(key===today)return'Today';if(key===yesterday)return'Yesterday';return new Intl.DateTimeFormat('id-ID',{day:'numeric',month:'long',...(d.getFullYear()!==now.getFullYear()?{year:'numeric'}:{})}).format(d);}
function appendMessageTimeline(items,target,skipSeparatorKey=''){let previous='';for(const m of items||[]){if(isMessageHidden(m))continue;const day=messageDayKey(m.ts);if(day!==previous){if(day!==skipSeparatorKey){const divider=document.createElement('div');divider.className='chat-date-separator';divider.dataset.dayKey=day;divider.innerHTML=`<span>${esc(messageDayLabel(m.ts))}</span>`;target.appendChild(divider);}previous=day;}renderMessage(m,false,target);}}
function updateChatDateIndicator(){const box=$('messages'),sticky=$('chatDateSticky'),label=$('chatDateStickyLabel');if(!box||!sticky||!label)return;const dividers=[...box.querySelectorAll('.chat-date-separator')];if(!dividers.length){sticky.classList.remove('visible');return;}const edge=box.getBoundingClientRect().top+58;let current=dividers[0];for(const divider of dividers){if(divider.getBoundingClientRect().top<=edge)current=divider;else break;}label.textContent=current.textContent;const top=box.getBoundingClientRect().top+7;sticky.classList.toggle('visible',box.scrollTop>80&&current.getBoundingClientRect().bottom<=top);}
function appendLiveMessage(m,notify){const box=$('messages');if(!box)return;const day=messageDayKey(m.ts),rows=box.querySelectorAll('.msg-row'),last=rows[rows.length-1];if(!last||last.dataset.dayKey!==day){const divider=document.createElement('div');divider.className='chat-date-separator';divider.dataset.dayKey=day;divider.innerHTML=`<span>${esc(messageDayLabel(m.ts))}</span>`;box.appendChild(divider);}renderMessage(m,notify);updateChatDateIndicator();}
let messageRowsForRefresh=null;
function renderCurrentMessages(preserveScroll=false){const box=$('messages');if(!box)return;const saved=preserveScroll?captureConversationScrollPosition():null,oldTop=box.scrollTop,oldHeight=box.scrollHeight;const desired=[],target={appendChild:node=>desired.push(node)};messageRowsForRefresh=new Map([...box.querySelectorAll('.msg-row[data-message-id]')].map(row=>[String(row.dataset.messageId),row]));try{if(historyHasMore){let wrap=box.querySelector('.older-messages-wrap');if(!wrap){wrap=document.createElement('div');wrap.className='older-messages-wrap';wrap.innerHTML='<button id="loadOlderMessages" class="older-messages-btn">↑ Muat pesan sebelumnya</button>';wrap.querySelector('button').addEventListener('click',loadOlderMessages);}const button=wrap.querySelector('button');if(button&&!historyLoadingOlder){button.disabled=false;button.textContent='↑ Muat pesan sebelumnya';}target.appendChild(wrap);}let sticky=box.querySelector('#chatDateSticky');if(!sticky){sticky=document.createElement('div');sticky.id='chatDateSticky';sticky.className='chat-date-sticky';sticky.innerHTML='<span id="chatDateStickyLabel"></span>';}target.appendChild(sticky);appendMessageTimeline(messageStore,target);}finally{messageRowsForRefresh=null;}const keep=new Set(desired);for(const node of [...box.children])if(!keep.has(node))node.remove();desired.forEach((node,index)=>{if(box.children[index]!==node)box.insertBefore(node,box.children[index]||null)});if(preserveScroll==='prepend'){box.scrollTop=Math.max(0,box.scrollHeight-oldHeight+oldTop);}else if(preserveScroll){restoreConversationScrollPosition(activeConversationId,saved);}else scrollBottom();updateChatDateIndicator();}
function updateExistingReceipt(row,m){const receipt=row.querySelector('.message-receipt');if(!receipt)return;const reads=Object.keys(m.readBy||{}).length,dm=m.conversationType==='dm',read=dm?!!m.receipt?.readAt:!!reads,delivered=dm?!!m.receipt?.deliveredAt:true;receipt.classList.toggle('read',read);receipt.classList.toggle('delivered',delivered&&!read);receipt.innerHTML=window.KCChatReceipts?.icon(delivered||read)||(delivered||read?'✓✓':'✓');receipt.title=dm?(read?'Dibaca':delivered?'Terkirim ke perangkat penerima':'Penerima offline / belum terkirim'):(reads?reads+' anggota sudah membaca':'Belum dibaca');}

function closeActiveConversation(){flushConversationScrollPosition();emitTypingState(false);activeConversationId=null;messageStore=[];historyHasMore=false;historyLoadingOlder=false;chatScrolledAway=false;newMessagesWhileScrolled=0;remoteTyping.clear();renderTypingIndicator();clearMessageSelection();clearReply();$('conversationMenu')?.classList.add('hidden');$('messageContextMenu')?.classList.add('hidden');renderConversationList();$('chatEmptyState')?.classList.remove('hidden');$('chatHeader')?.classList.add('chat-inactive');$('messages').innerHTML='';$('scrollToBottom')?.classList.add('hidden');document.querySelector('.composer-wrap')?.classList.add('chat-inactive');}
function chatCacheKey(id){return 'kc-chat-cache-v220:'+String(cfg?.deviceId||cfg?.name||'pc')+':'+String(id||'general')}
const pendingChatCache = new Map();
let chatCacheTimer;
function flushChatCache() {
  clearTimeout(chatCacheTimer);
  chatCacheTimer = undefined;
  const next = pendingChatCache.entries().next();
  if (next.done) return;
  const [key, items] = next.value;
  pendingChatCache.delete(key);
  try { localStorage.setItem(key,JSON.stringify(items)); } catch {}
  if (pendingChatCache.size) chatCacheTimer = setTimeout(flushChatCache, 250);
}
function saveChatCache(id, items) {
  pendingChatCache.set(chatCacheKey(id), (items || []).slice(-360));
  if (!chatCacheTimer) chatCacheTimer = setTimeout(flushChatCache, 250);
}
function loadChatCache(id){try{return JSON.parse(localStorage.getItem(chatCacheKey(id))||'[]')}catch{return[]}}
function openConversation(id){
  if(statusWorkspaceIsOpen())closeStatusWorkspace();if(chatImageWorkspaceOpen)closeChatImageWorkspace();
  flushConversationScrollPosition();id=String(id||'');if(!id)return;if(prefHas('manualUnread',id)){conversationPrefs.manualUnread=(conversationPrefs.manualUnread||[]).filter(x=>x!==id);saveConversationPrefs();}activeConversationId=id;$('chatEmptyState')?.classList.add('hidden');$('chatHeader')?.classList.remove('chat-inactive');document.querySelector('.composer-wrap')?.classList.remove('chat-inactive');clearMessageSelection();clearReply();document.querySelectorAll('.conversation-row').forEach(row=>row.classList.toggle('active',String(row.dataset.id)===id));
  const known=conversations.find(c=>String(c.id)===id);if(known)updateActiveConversationHeader(known);renderTypingIndicator();
  const box=$('messages');const cached=loadChatCache(id);if(cached.length){messageStore=cached;historyHasMore=false;renderCurrentMessages(true);}else if(box)box.innerHTML='<div class="chat-loading">Memuat percakapan…</div>';if(!socket?.connected){showToast('Offline · menampilkan cache chat lokal');return;}
  socket.emit('conversation-history',{id,limit:120},r=>{
    if(String(activeConversationId)!==id)return;
    if(!r?.ok){if(box)box.innerHTML='<div class="chat-loading">Chat tidak bisa dimuat.</div>';showToast(r?.error||'Gagal membuka chat');return;}
    const c=r.conversation||known;if(c){const ix=conversations.findIndex(x=>String(x.id)===id);if(ix>=0)conversations[ix]={...conversations[ix],...c};else conversations.push(c);updateActiveConversationHeader(c);}
    messageStore=Array.isArray(r.items)?r.items.slice():[];saveChatCache(id,messageStore);historyHasMore=!!r.hasMore;renderCurrentMessages(true);const saved=conversationScrollPositions[id],hasSaved=Object.prototype.hasOwnProperty.call(conversationScrollPositions,id);scrollRestoreInProgress=true;if(hasSaved){const restore=()=>restoreConversationScrollPosition(id,saved);requestAnimationFrame(()=>{restore();requestAnimationFrame(restore)});}else{scrollBottom();setTimeout(()=>scrollBottom(),120);}setTimeout(()=>{if(String(activeConversationId)!==id)return;scrollRestoreInProgress=false;flushConversationScrollPosition();updateScrollToBottomButton()},500);requestAnimationFrame(()=>focusChatComposer(false));
    socket.emit('conversation-open',{id},()=>{requestConversationList();focusChatComposer(false);});
  });
}
function loadOlderMessages(){if(historyLoadingOlder||!historyHasMore||!activeConversationId||!socket?.connected)return;historyLoadingOlder=true;const id=String(activeConversationId),oldest=messageStore.length?Math.min(...messageStore.map(m=>Number(m.ts||0))):Date.now();const btn=document.querySelector('.older-messages-btn');if(btn){btn.disabled=true;btn.textContent='Memuat…';}socket.emit('conversation-history',{id,limit:120,beforeTs:oldest,beforeId:messageStore[0]?.id||''},r=>{historyLoadingOlder=false;if(String(activeConversationId)!==id)return;if(!r?.ok){if(btn){btn.disabled=false;btn.textContent='↑ Muat pesan sebelumnya';}return;}const seen=new Set(messageStore.map(m=>String(m.id)));const older=(Array.isArray(r.items)?r.items:[]).filter(m=>!seen.has(String(m.id)));messageStore=[...older,...messageStore];historyHasMore=!!r.hasMore;const box=$('messages');if(!box||!older.length){renderCurrentMessages('prepend');return;}let wrap=box.querySelector('.older-messages-wrap');if(historyHasMore&&!wrap){wrap=document.createElement('div');wrap.className='older-messages-wrap';wrap.innerHTML='<button id="loadOlderMessages" class="older-messages-btn">↑ Muat pesan sebelumnya</button>';box.prepend(wrap);wrap.querySelector('button').addEventListener('click',loadOlderMessages);}else if(wrap){const loadBtn=wrap.querySelector('.older-messages-btn');if(loadBtn){loadBtn.disabled=false;loadBtn.textContent='↑ Muat pesan sebelumnya';}}if(!historyHasMore)wrap?.remove();const fragment=document.createDocumentFragment(),first=box.querySelector('.msg-row'),existingDay=first?.dataset.dayKey||'';appendMessageTimeline(older,fragment,existingDay);const oldTop=box.scrollTop,oldHeight=box.scrollHeight;scrollRestoreInProgress=true;if(first)box.insertBefore(fragment,first);else box.appendChild(fragment);requestAnimationFrame(()=>{if(String(activeConversationId)!==id)return;box.scrollTop=oldTop+Math.max(0,box.scrollHeight-oldHeight);scrollRestoreInProgress=false;saveConversationScrollPosition();updateChatDateIndicator();});});}
function typingNamesForActive(){const now=Date.now(),names=[];for(const [k,v] of remoteTyping){if(now-Number(v.ts||0)>2600){remoteTyping.delete(k);continue;}if(v.typing&&String(v.conversationId)===String(activeConversationId)&&profileKeyLocal(v.name)!==profileKeyLocal(cfg?.name))names.push(v.name);}return names;}
function renderTypingIndicator(){const el=$('typingIndicator'),text=$('typingText');if(!el||!text)return;const names=typingNamesForActive().map(profileDisplayName);if(!names.length){el.classList.add('hidden');return;}text.textContent=names.length===1?`${names[0]} sedang mengetik`:names.length===2?`${names[0]} dan ${names[1]} sedang mengetik`:`${names[0]} dan ${names.length-1} lainnya sedang mengetik`;el.classList.remove('hidden');}
function emitTypingState(on){if(!activeConversationId||!socket?.connected)return;const now=Date.now();if(on&&now-lastTypingEmitAt<700)return;lastTypingEmitAt=now;socket.emit('typing-state',{id:activeConversationId,typing:!!on});if(!on){clearTimeout(typingStopTimer);typingStopTimer=null;}}
function scheduleTypingStop(){clearTimeout(typingStopTimer);typingStopTimer=setTimeout(()=>emitTypingState(false),1400);}
function chatInputMayFocus(){if(window.getSelection()?.toString())return false;if(!activeConversationId)return false;const ae=document.activeElement;if(ae&&ae!==document.body&&ae!==$('text')){const tag=String(ae.tagName||'').toLowerCase();if(['input','textarea','select'].includes(tag)||ae.isContentEditable)return false;}return !document.querySelector('.modal:not(.hidden), .popover:not(.hidden), .context-menu:not(.hidden)');}
function focusChatComposer(force=false){if(!activeConversationId)return;const el=$('text');if(!el)return;if(force||chatInputMayFocus())setTimeout(()=>{if(!force&&!chatInputMayFocus())return;try{el.focus({preventScroll:true})}catch{try{el.focus()}catch{}}},20);}
window.addEventListener('focus',()=>focusChatComposer(false));
document.addEventListener('click',e=>{if(!activeConversationId)return;if(e.target.closest('#messages, #chatHeader, .chat-main')&&!e.target.closest('button,a,input,textarea,select,[contenteditable=true]'))focusChatComposer(false);});
function chatDistanceFromBottom(){const box=$('messages');return box?Math.max(0,box.scrollHeight-box.clientHeight-box.scrollTop):0;}
function chatIsNearBottom(threshold=120){return chatDistanceFromBottom()<=threshold;}
function updateScrollToBottomButton(){const b=$('scrollToBottom'),badge=$('scrollToBottomUnread');if(!b)return;const away=!!activeConversationId&&!chatIsNearBottom(150);chatScrolledAway=away;b.classList.toggle('hidden',!away);if(badge){if(newMessagesWhileScrolled>0&&away){badge.textContent=newMessagesWhileScrolled>99?'99+':String(newMessagesWhileScrolled);badge.title=`${newMessagesWhileScrolled} pesan baru`;badge.classList.remove('hidden');b.title=`${newMessagesWhileScrolled} pesan baru · Ke pesan terbaru`;}else{badge.textContent='';badge.title='';badge.classList.add('hidden');b.title='Ke pesan terbaru';}}if(!away)newMessagesWhileScrolled=0;}
$('messages')?.addEventListener('scroll',()=>{const box=$('messages');if(!scrollRestoreInProgress){saveConversationScrollPosition();if(historyHasMore&&!historyLoadingOlder&&box.scrollTop<72)loadOlderMessages();}updateScrollToBottomButton();updateChatDateIndicator();},{passive:true});
window.addEventListener('beforeunload',flushConversationScrollPosition);
$('scrollToBottom')?.addEventListener('click',()=>{newMessagesWhileScrolled=0;scrollBottom(true);});
function renderNewChatUsers(){const q=String($('newDmSearch')?.value||'').trim().toLowerCase();const users=[...(editorDirectory||[]),...(onlineUsers||[])];const known=new Map();for(const u of users)if(String(u.name).toLowerCase()!==String(cfg.name).toLowerCase()){const k=String(u.name).toLowerCase(),prev=known.get(k)||{};known.set(k,{...prev,...u,online:!!u.online||onlineUsers.some(x=>String(x.name).toLowerCase()===k)});}const list=[...known.values()].filter(u=>!q||String(profileDisplayName(u.name)).toLowerCase().includes(q)).sort((a,b)=>(b.online?1:0)-(a.online?1:0)||String(a.name).localeCompare(String(b.name)));$('newDmList').innerHTML=list.map(u=>`<div class="new-chat-user" data-name="${esc(u.name)}">${avatarHtml(u.avatarUrl,u.name,'avatar')}<div><b>${esc(profileDisplayName(u.name))}</b><div class="conv-preview">${u.online?'online':(u.lastSeen?'terakhir terdaftar '+new Date(u.lastSeen).toLocaleDateString('id-ID'):'offline')}</div></div></div>`).join('')||'<div class="storage-note" style="padding:15px">Tidak ada editor online lain.</div>';$('newGroupMembers').innerHTML=list.map(u=>`<div class="new-chat-user"><label><input type="checkbox" value="${esc(u.name)}">${avatarHtml(u.avatarUrl,u.name,'avatar')}<span><b>${esc(profileDisplayName(u.name))}</b><small>${u.online?' online':' offline'}</small></span></label></div>`).join('')||'<div class="storage-note" style="padding:15px">Tidak ada editor online lain.</div>';document.querySelectorAll('#newDmList .new-chat-user[data-name]').forEach(el=>el.onclick=()=>createDm(el.dataset.name));const updateGroupCount=()=>{const n=document.querySelectorAll('#newGroupMembers input:checked').length;if($('newGroupMemberCount'))$('newGroupMemberCount').textContent=n+' dipilih';};document.querySelectorAll('#newGroupMembers input[type=checkbox]').forEach(x=>x.onchange=updateGroupCount);updateGroupCount();}
function createDm(name){socket.emit('conversation-create-dm',{name},r=>{if(!r?.ok)return showToast(r?.error||'Gagal membuat chat');$('newChatModal').classList.add('hidden');requestConversationList();setTimeout(()=>openConversation(r.conversation.id),60);});}
function openChatInfo(){
  const c=activeConversation(); if(!c)return;
  const name=conversationDisplayTitle(c), type=c.type==='group'?'Grup':c.type==='dm'?'Chat Pribadi':'Minta File Bang';
  $('chatInfoTitle').textContent=name; $('chatInfoType').textContent=type;
  const ch=c.type==='group'?'👥':c.type==='general'?'#':(name||'?')[0].toUpperCase(),heroUrl=conversationAvatarUrl(c);
  let body=`<div class="chat-info-hero"><div class="avatar chat-info-avatar ${heroUrl?'has-photo':''}" ${heroUrl?`style="--avatar-url:url('${esc(fullUrl(heroUrl))}');background-image:var(--avatar-url)"`:''}>${heroUrl?'':esc(ch)}</div><b>${esc(name)}</b><span>${esc(conversationSubtitle(c)||type)}</span></div>`;
  if(c.type==='group'){
    const adminSet=new Set((c.admins||[]).map(x=>String(x).toLowerCase()));
    body+=`<div class="chat-info-block"><small>DESKRIPSI</small><p>${esc(c.description||'Tidak ada deskripsi grup')}</p>${(cfg?.isHost||adminSet.has(String(cfg?.name||'').toLowerCase()))?'<div class="group-edit-actions"><button id="editGroupIdentity" class="secondary">✎ Edit Nama / Deskripsi</button><button id="changeGroupAvatar" class="secondary">🖼 Ganti Foto Grup</button></div>':''}</div><div class="chat-info-section-title">${(c.members||[]).length} anggota</div><div class="chat-info-members">${(c.members||[]).map(n=>`<div class="chat-info-member"><div class="avatar mini-avatar">${esc(String(n||'?')[0].toUpperCase())}</div><span>${esc(profileDisplayName(n))}</span>${adminSet.has(String(n).toLowerCase())?'<b>Admin</b>':''}</div>`).join('')}</div><div class="chat-info-actions">${(cfg?.isHost||adminSet.has(String(cfg?.name||'').toLowerCase()))?'<button id="inviteGroupNow" class="primary">+ Invite Anggota</button>':''}<button id="leaveGroupNow" class="danger">Keluar Grup</button>${cfg?.isHost?'<button id="deleteGroupNow" class="danger">Hapus Grup untuk Semua</button>':''}</div>`;
  } else {
    const peer=c.type==='dm'?conversationPeerUser(c):null,live=c.type==='dm'?conversationLiveActivity(c):null,peerRole=c.type==='dm'?conversationTeamRole(c):'';
    body+=`<div class="chat-info-block"><small>INFO</small><p>${c.type==='general'?'Grup kantor seluruh editor. History lama tetap dipertahankan.':'Percakapan pribadi lokal antara dua editor.'}</p>${c.type==='dm'?`<span class="chat-info-role">ROLE · ${esc(peerRole)}</span><div class="chat-info-activity">${live?`${activityIcon(live)} <span>${esc(activitySummary(live))}</span>`:'◌ <span>Activity tidak tersedia / offline</span>'}</div>${peer?.pc?`<small>${esc(peer.pc)}</small>`:''}`:''}${c.type==='general'&&cfg?.isHost?'<div class="group-edit-actions"><button id="editOfficeGroup" class="secondary">✎ Edit Nama / Deskripsi</button><button id="changeOfficeGroupAvatar" class="secondary">🖼 Ganti Foto Grup</button></div>':''}</div>`;
  }
  $('chatInfoBody').innerHTML=body; $('chatInfoModal').classList.remove('hidden');
  setTimeout(()=>{const b=$('leaveGroupNow');if(b)b.onclick=async()=>{const yes=await appConfirm({title:'Keluar grup?',text:'Kamu tidak akan menerima pesan baru dari grup ini.',ok:'Keluar'});if(!yes)return;socket.emit('conversation-leave',{id:c.id},r=>{if(r?.ok){$('chatInfoModal').classList.add('hidden');closeActiveConversation();requestConversationList();}else showToast(r?.error||'Gagal keluar grup');});};const inv=$('inviteGroupNow');if(inv)inv.onclick=()=>openGroupInvite(c);const editGroup=$('editGroupIdentity');if(editGroup)editGroup.onclick=()=>editConversationIdentity(c);const changeAvatar=$('changeGroupAvatar');if(changeAvatar)changeAvatar.onclick=()=>{groupAvatarTarget=c;$('groupAvatarInput')?.click();};const officeAvatar=document.getElementById('changeOfficeGroupAvatar');if(officeAvatar)officeAvatar.onclick=()=>{groupAvatarTarget=c;$('groupAvatarInput')?.click();};const eg=document.getElementById('editOfficeGroup');if(eg)eg.onclick=()=>editConversationIdentity(c);const del=$('deleteGroupNow');if(del)del.onclick=async()=>{const yes=await appConfirm({title:'Hapus grup untuk semua?',text:'Grup dan seluruh riwayat pesan akan dihapus permanen untuk semua anggota.',ok:'Hapus Grup'});if(!yes)return;socket.emit('conversation-delete-group',{id:c.id},r=>{if(r?.ok){$('chatInfoModal').classList.add('hidden');closeActiveConversation();requestConversationList();}else showToast(r?.error||'Gagal menghapus grup');});};},0);
}
async function init(){
  techLog('renderer-init-start',{readyState:document.readyState,href:location.href});
  techLog('get-config-start');
  cfg=await window.kc.getConfig();
  techLog('get-config-done',{isHost:cfg?.isHost,role:cfg?.role,version:cfg?.version,buildRoleSource:cfg?.buildRoleSource,serverAddress:cfg?.serverAddress,executableName:cfg?.executableName});
  const roleBadge=$('runtimeRoleBadge');
  if(roleBadge){
    roleBadge.textContent=`${cfg.isHost?'HOST':'CLIENT'} · v${cfg.version||'?'} · ${cfg.buildRoleSource||'unknown'}`;
    roleBadge.classList.add(cfg.isHost?'host':'client');
    roleBadge.title=`EXE: ${cfg.executableName||'-'} | product: ${cfg.productName||'-'}`;
  }

  if(!cfg.name){
    // Role sudah dikunci oleh build HOST / CLIENT; user hanya perlu mengisi nama.
    $('setup').classList.remove('hidden');
    const isHost=cfg.role==='server';
    if($('setupTeamRole'))$('setupTeamRole').value=normalizeTeamRole(cfg?.teamRole||(isHost?'KOORDINATOR':'EDITOR'));
    $('serverRole').style.display=isHost?'flex':'none';
    $('clientRole').style.display=isHost?'none':'flex';
    if(isHost){$('serverRole').querySelector('b').textContent='SIMPAN NAMA & BUKA HOST';$('serverRole').querySelector('span').textContent='Server utama + chat + panel admin';}
    else {$('clientRole').querySelector('b').textContent='SIMPAN NAMA & BUKA CLIENT';$('clientRole').querySelector('span').textContent='Otomatis mencari HOST di jaringan LAN';}
    return;
  }
  cfg.teamRole=normalizeTeamRole(cfg.teamRole);cfg.workTagPrefs=normalizeWorkTagPrefs(cfg.workTagPrefs);if($('myName'))$('myName').textContent=cfg.displayName||cfg.name;if($('profileNameLabel'))$('profileNameLabel').textContent=cfg.displayName||cfg.name;loadConversationPrefs();loadConversationScrollPositions();loadAppearancePrefs();loadHiddenMessages();setAvatar($('myAvatarButton'),null,cfg.name);setAvatar($('railProfile'),null,cfg.name);loadTheme();applyPerformanceProfile(cfg.performanceProfile||'balanced',false);document.title=cfg.isHost?'ChatX HOST':'ChatX CLIENT';if(cfg.isHost&&$('myName'))$('myName').textContent=(cfg.displayName||cfg.name)+' · HOST';
  if(cfg.isHost){$('railSystem')?.classList.remove('hidden');$('railModeration')?.classList.remove('hidden');$('railActivity')?.classList.remove('hidden');$('railHostMore')?.classList.remove('hidden');$('adminButton').classList.remove('hidden');$('updateManagerButton').classList.remove('hidden');$('systemButton')?.classList.remove('hidden');$('diagnosticsButton')?.classList.remove('hidden');$('storageLocationSection')?.classList.remove('hidden');}else $('railServerPicker')?.classList.remove('hidden');
  if($('allowTemporaryServer'))$('allowTemporaryServer').checked=cfg.allowTemporaryServer===true;
  techLog('renderer-init-connect',{isHost:cfg.isHost,role:cfg.role,name:cfg.name});
  await connectLoop();
  techLog('renderer-init-done',{serverBase});
}
async function chooseRole(role){const name=$('setupName').value.trim();if(!name){$('setupName').focus();return;}const lockedRole=cfg?.role||role,teamRole=normalizeTeamRole($('setupTeamRole')?.value||(lockedRole==='server'?'KOORDINATOR':'EDITOR')),workTagPrefs=defaultWorkTagPrefs();await window.kc.saveConfig({role:lockedRole,name,teamRole,workTagPrefs});window.kc.restart();}
$('serverRole').onclick=()=>chooseRole('server');$('clientRole').onclick=()=>chooseRole('client');

function clearReconnectTimers(){if(reconnectTimer){clearTimeout(reconnectTimer);reconnectTimer=null;}if(rediscoverTimer){clearTimeout(rediscoverTimer);rediscoverTimer=null;}}
function scheduleDiscovery(delay=null){if(rediscoverTimer)return;const wait=Number.isFinite(delay)?Math.max(0,delay):window.KCReconnectPolicy.discoveryRetryDelay(discoveryRetryAttempt++);techLog('discovery-retry-scheduled',{wait,attempt:discoveryRetryAttempt});rediscoverTimer=setTimeout(()=>{rediscoverTimer=null;connectLoop();},wait);}
function requestFreshDiscovery(reason='network-change',delay=500){
  if(cfg?.isHost&&socket?.connected&&sessionReady&&['network-interface-change','startup-network-ready','system-resume'].includes(reason)){techLog('host-local-session-preserved',{reason});return;}
  connectGeneration++;connectAttempt++;connecting=false;sessionReady=false;sessionToken='';reconnectQueued=false;socketConnectErrorStreak=0;discoveryRetryAttempt=0;disconnectedSince=Date.now();clearReconnectTimers();
  if(socket){try{socket.removeAllListeners();socket.disconnect();}catch{}socket=null;}
  serverBase='';techLog('network-fresh-discovery-requested',{reason});
  setConn('Memulihkan koneksi · mencari HOST...',false);scheduleDiscovery(delay);
}
async function socketAckCompat(event,payload={},timeout=8000){return new Promise(resolve=>{try{socket.timeout(timeout).emit(event,payload,(err,r)=>resolve(err?null:r));}catch{resolve(null);}});}
async function mirrorLegacyPeerToHost(){
  if(!cfg?.isHost||!socket?.connected||!window.kc?.mergeLegacyPeerSnapshot)return null;
  const list=await socketAckCompat('conversation-list-request',{},7000);if(!list?.ok)return null;
  const convs=Array.isArray(list.items)?list.items:[];const all=[];
  for(const c of convs){let beforeTs=Infinity,beforeId='',guard=0;while(guard++<60){const r=await socketAckCompat('conversation-history',{id:c.id,limit:120,beforeTs,beforeId},8000);if(!r?.ok)break;const items=Array.isArray(r.items)?r.items:[];all.push(...items);if(!r.hasMore||!items.length)break;beforeTs=Math.min(...items.map(m=>Number(m.ts||Date.now())));beforeId=String(items[0]?.id||'');}}
  const result=await window.kc.mergeLegacyPeerSnapshot({base:serverBase,token:sessionToken,conversations:convs,messages:all});
  techLog('legacy-peer-mirror',result||{});return result;
}
async function connectLoop(){
  if(socket?.connected&&sessionReady)return;
  if(connecting){reconnectQueued=true;techLog('connect-loop-skip',{reason:'already-connecting',queued:true});return;} connecting=true; clearReconnectTimers();const generation=connectGeneration,attempt=++connectAttempt;
  const hostMode=cfg?.isHost||cfg?.role==='server';
  techLog('connect-loop-start',{hostMode,isHost:cfg?.isHost,role:cfg?.role,serverAddress:cfg?.serverAddress});
  setConn(hostMode?(attempt===1?'Menghubungkan ke HOST lokal…':`Mencoba ulang koneksi HOST · percobaan ${attempt}…`):(attempt===1?'Mencari server…':`Mencoba ulang koneksi · percobaan ${attempt}…`),false);
  try{
    const validServerAddress=value=>typeof value==='string'&&/^https?:\/\/[^\s]+$/i.test(value.trim());
    // HOST mode is pinned to its own permanent listener; it never joins another PC's server.
    let found=null;
    if(hostMode&&window.kc.startHostPrimary){
      try{
        const primary=await window.KCReconnectPolicy.withDeadline(()=>window.kc.startHostPrimary(),15000,'Menyalakan listener HOST lokal');
        if(primary?.ok)found=primary.address||cfg?.serverAddress||null;
        else techLog('host-primary-unavailable',{error:primary?.error||'Listener HOST tidak siap'});
      }catch(e){techLog('host-primary-start-timeout',{message:e?.message||String(e)});}
    }
    if(!hostMode){
      // Manual choice is accepted only after a fresh LAN health scan confirms it
      // is a permanent HOST. Stale config pointing to a former temporary peer is ignored.
      if(validServerAddress(cfg?.manualServerAddress)){
        try{
          found=await window.KCReconnectPolicy.withDeadline(()=>window.kc.probePrimaryServer(cfg.manualServerAddress),1800,'Validasi HOST pilihan');
        }catch(e){techLog('manual-host-validation-failed',{message:e?.message||String(e)});}
      }
      if(!found){try{found=await window.KCReconnectPolicy.withDeadline(()=>window.kc.discoverPrimaryServer(),9000,'Pencarian HOST utama');}catch(e){techLog('primary-discovery-timeout',{message:e?.message||String(e)});}}
      if(!found)found=await window.KCReconnectPolicy.withDeadline(()=>window.kc.discover(),15000,'Pencarian HOST utama');
    }
    // Never fall back to a cached address on HOST or to an unclassified server on CLIENT.
    if(found&&!validServerAddress(found)){techLog('discovery-invalid-result',{type:typeof found});found=null;}
    if(generation!==connectGeneration){techLog('connect-loop-stale-result',{generation,current:connectGeneration});return;}
    // Socket.IO may have recovered while IPC discovery was still pending.
    // Do not replace that live transport or interrupt its authentication.
    if(socket?.connected){techLog('connect-loop-preserved-live-socket',{sessionReady});return;}
    techLog('connect-target-resolved',{hostMode,found:found||null});
    if(!found){setConn(hostMode?`Listener HOST lokal belum siap · percobaan ${discoveryRetryAttempt+1}, akan mencoba ulang otomatis`:'HOST utama belum ditemukan · akan mencari ulang otomatis',false);connecting=false;scheduleDiscovery();return;}
    serverBase=found; tryConnect(found);
  }catch(e){if(generation!==connectGeneration||attempt!==connectAttempt)return;techLog('connect-loop-error',{hostMode,message:e?.message||String(e)});setConn(hostMode?`Server HOST belum terjangkau · percobaan ${discoveryRetryAttempt+1}, akan mencari ulang otomatis`:'Server belum ditemukan · akan mencari ulang otomatis',false);scheduleDiscovery();}
  finally{if(attempt===connectAttempt){connecting=false;if(reconnectQueued){reconnectQueued=false;scheduleDiscovery(450);}}}
}
function hello(target=socket,epoch=target?._kcSessionEpoch){return new Promise(resolve=>{if(!target?.connected||target!==socket||target._kcSessionEpoch!==epoch)return resolve(null);target.timeout(8000).emit('hello',{name:cfg.name,pc:cfg.pc,deviceId:cfg.deviceId,deviceKey:cfg.deviceKey,version:cfg.version||'',adminToken:cfg.adminToken||null,hostAuthority:cfg?.isHost===true,teamRole:myTeamRole(),workTagPrefs:myWorkTagPrefs(),activitySharing:activityShareEnabled(),activity:lastLocalActivity||{kind:'chat',app:'ChatX',title:'Online di ChatX',state:'active'}},(err,r)=>{if(target!==socket||!target.connected||target._kcSessionEpoch!==epoch)return resolve(null);resolve(err?{ok:false,error:'HOST belum merespons'}:r)});});}
function tryConnect(url){
  if(socket){try{socket.removeAllListeners();socket.disconnect();}catch{}}
  const target=io(url,{transports:['websocket','polling'],upgrade:true,rememberUpgrade:false,tryAllTransports:true,reconnection:true,reconnectionAttempts:Infinity,reconnectionDelay:1000,reconnectionDelayMax:5000,randomizationFactor:.35,timeout:5000});socket=target;
  target.on('session-auth',r=>{if(target===socket&&r?.ok)sessionToken=r.token;});
  target.on('peer-handover',(info={})=>{if(target!==socket)return;if(!sessionReady||!currentServerTemporary){techLog('peer-handover-ignored',{reason:'current-session-is-not-temporary'});return;}const announced=String(info?.address||'');const hostMode=cfg?.isHost===true||cfg?.role==='server';const addr=window.KCHandoverAddress?.resolve(announced,hostMode)||announced;techLog('peer-handover-notice',{address:announced||null,reconnectAddress:addr||null,hostMode});clearInterval(clientPrimaryMonitor);clientPrimaryMonitor=null;setConn('HOST utama sudah menyinkronkan data · memindahkan koneksi...',false);if(/^https?:\/\/[^\s]+$/i.test(addr)){connectGeneration++;sessionReady=false;sessionToken='';try{target.removeAllListeners();target.disconnect();}catch{}socket=null;serverBase=addr;setTimeout(()=>tryConnect(addr),350);}else setTimeout(()=>requestFreshDiscovery('peer-handover-notice',700),700);});
  target.on('connect',async()=>{if(target!==socket)return;const epoch=(target._kcSessionEpoch||0)+1;target._kcSessionEpoch=epoch;socketConnectErrorStreak=0;discoveryRetryAttempt=0;sessionReady=false;sessionToken='';techLog('socket-connect',{url,id:target.id||''});clearReconnectTimers();serverBase=url;setConn('HOST ditemukan · memulihkan sesi...',false);let auth;try{auth=await window.KCReconnectPolicy.withDeadline(()=>waitForApprovedSession(target,epoch),20000,'Pemulihan sesi HOST');}catch(error){techLog('session-recovery-timeout',{message:error.message});}if(target!==socket||target._kcSessionEpoch!==epoch)return;if(!auth?.ok){requestFreshDiscovery('session-recovery-failed',1000);return;}if(auth?.peerTemporary===true){techLog('temporary-server-rejected',{url,role:cfg?.isHost?'host':'client'});sessionReady=false;sessionToken='';setConn(cfg?.isHost?'HOST menolak server non-utama · memakai listener lokal':'Server sementara ditolak · mencari HOST utama…',false);try{target.removeAllListeners();target.disconnect();}catch{}if(socket===target)socket=null;serverBase='';requestFreshDiscovery('temporary-server-rejected',500);return;}sessionReady=true;lastAuthenticatedServer=serverBase;currentServerTemporary=auth?.peerTemporary===true;disconnectedSince=0;setConn('Terhubung ke jaringan lokal',true);if((typeof cfg!=='undefined'&&cfg?.isHost)&&auth?.peerTemporary===true&&window.kc.syncPeerToHost){if(typeof clearInterval==='function')clearInterval(hostPeerMonitor);hostPeerMonitor=null;setConn('HOST online · sinkronisasi data non-primary server...',true);try{const synced=await window.kc.syncPeerToHost({base:serverBase,token:sessionToken,handoverToken:auth.handoverToken});if(!synced?.ok)throw Error(synced?.error||'Sinkronisasi gagal');techLog('peer-handover-sync-complete',synced);setConn('Sinkronisasi selesai · menyalakan server HOST utama...',true);const primary=await window.KCReconnectPolicy.withDeadline(()=>window.kc.startHostPrimary(),15000,'Menyalakan server HOST utama');if(!primary?.ok||!primary?.lanAddress)throw Error(primary?.error||'Server HOST utama belum siap');techLog('host-primary-ready',primary);setConn('HOST siap · mengalihkan client dari non-primary server...',true);const takeover=await new Promise(resolve=>target.timeout(10000).emit('peer-handover-request',{syncedAt:Date.now(),handoverToken:auth.handoverToken,hostAddress:primary.lanAddress},(err,r)=>resolve(err?{ok:false,error:'Temporary server belum mengonfirmasi takeover'}:r)));if(!takeover?.ok)throw Error(takeover?.error||'Takeover belum dikonfirmasi');setConn('Takeover dikonfirmasi · reconnect ke HOST utama...',true);setTimeout(()=>{requestFreshDiscovery('host-handover-complete',250);},250);return;}catch(error){techLog('peer-handover-sync-failed',{message:error?.message||String(error)});setConn('Terhubung ke server non-utama · sync HOST gagal, akan dicoba lagi',true);setTimeout(()=>requestFreshDiscovery('host-handover-retry',5000),5000);}}if(cfg?.isHost&&auth?.peerTemporary!==true)techLog('host-primary-session-ready',{legacyMirror:false});bindGameSocket();bindV1110Socket();startActivityReporter();void flushOutbox();setTimeout(()=>{if(target===socket&&target._kcSessionEpoch===epoch&&sessionReady)autoScanFileGallery(false,false);},900);scheduleGalleryRefresh(target,epoch);unread=0;updateUnread();void loadMyProfile();if(target!==socket||target._kcSessionEpoch!==epoch)return;requestConversationList();setTimeout(()=>{if(target!==socket||target._kcSessionEpoch!==epoch)return;if(activeConversationId)openConversation(activeConversationId);else closeActiveConversation();},120);setTimeout(checkClientUpdate,700);/* v2.2.13: temporary-server peer monitor disabled; HOST is the only authority. */if(typeof cfg!=='undefined'&&!cfg?.isHost&&auth?.peerTemporary===true)startClientPrimaryMonitor();else{if(typeof clearInterval==='function')clearInterval(clientPrimaryMonitor);clientPrimaryMonitor=null;lastPrimaryNotice='';}});
  target.on('connect_error',err=>{if(target!==socket)return;socketConnectErrorStreak++;if(!disconnectedSince)disconnectedSince=Date.now();techLog('socket-connect-error',{url,name:err?.name||'',message:err?.message||'',streak:socketConnectErrorStreak});if(socketConnectErrorStreak===1||socketConnectErrorStreak%6===0)reportBug('socket-connect-error',err||new Error('Socket connect error'),'warning',{streak:socketConnectErrorStreak});setConn('HOST belum terjangkau · mencari ulang otomatis...',false);if(socketConnectErrorStreak>=3)scheduleDiscovery(window.KCReconnectPolicy.discoveryRetryDelay(socketConnectErrorStreak-3));});
  target.on('disconnect',reason=>{if(target!==socket)return;target._kcSessionEpoch=(target._kcSessionEpoch||0)+1;sessionReady=false;sessionToken='';disconnectedSince=Date.now();for(const[id,d]of galleryIncomingTransfers){galleryDownloadCompletion.get(id)?.(false);galleryDownloadCompletion.delete(id);}galleryIncomingTransfers.clear();techLog('socket-disconnect',{url,reason});if(!['io client disconnect','transport close','transport error','ping timeout'].includes(String(reason)))reportBug('socket-disconnect',new Error(String(reason||'disconnect')),'warning',{reason});setConn('Koneksi terputus · mencari HOST dan mencoba lagi...',false);if(reason==='io server disconnect')try{target.connect()}catch{};scheduleDiscovery(window.KCReconnectPolicy.discoveryRetryDelay(0));});
  socket.on('history',items=>{if(activeConversationId!=='general')return;messageStore=Array.isArray(items)?items.slice():[];renderCurrentMessages(false);});
  socket.on('message',m=>{const selfDevice=String(m?.senderDeviceId||'')&&String(m.senderDeviceId)===String(cfg?.deviceId||'');if(!selfDevice&&m?.conversationType==='dm')socket.emit('message-receipt',{id:m.id,stage:'delivered'},()=>{});const tagForMe=m?.workTag?workTagAppliesToMe(m):mentionForMe(m?.text||'',m),muted=prefHas('mutes',m.conversationId||'general'),mayBreakMute=tagForMe&&m?.bypassAppMute===true;if(!selfDevice&&(!muted||mayBreakMute)){void sendRichNotification(m).catch(e=>reportBug('notification-dispatch-failed',e,'warning',{messageId:m.id}));}else techLog('notification-skipped',{messageId:m.id,reason:selfDevice?'own-device':'chat-muted',tagForMe,senderMatchesProfile:String(m.sender||'').trim().toLowerCase()===String(cfg?.name||'').trim().toLowerCase()});if(String(m.conversationId||'general')===String(activeConversationId)){const wasNearBottom=chatIsNearBottom(150);if(messageStore.some(x=>x.id===m.id))return;messageStore.push(m);saveChatCache(activeConversationId,messageStore);appendLiveMessage(m,false);if(messageStore.length>360){const cut=messageStore.length-360;messageStore.splice(0,cut);historyHasMore=true;const rows=$('messages')?.querySelectorAll('.msg-row');if(rows&&rows.length>360)for(let i=0;i<rows.length-360;i++)rows[i].remove();}if(wasNearBottom||selfDevice)scrollBottom(true);else{newMessagesWhileScrolled++;updateScrollToBottomButton();}socket.emit('conversation-open',{id:activeConversationId},()=>{});}requestConversationList();});
  socket.on('message-receipt',applyMessageReceipt);
  socket.on('history-refresh',items=>{if(activeConversationId!=='general')return;messageStore=Array.isArray(items)?items.slice():[];renderCurrentMessages(true);});
  socket.on('conversation-list',items=>{conversations=Array.isArray(items)?items:[];scheduleConversationList();});
  socket.on('typing-state',p=>{const key=String(p?.conversationId||'')+'|'+profileKeyLocal(p?.name||'');if(!p?.typing)remoteTyping.delete(key);else remoteTyping.set(key,{...p,ts:Date.now()});renderTypingIndicator();setTimeout(renderTypingIndicator,2800);});
  socket.on('job-desk-updated',items=>window.KCCohost?.jobs(items));
  socket.on('editor-directory',items=>{editorDirectory=Array.isArray(items)?items:[];renderNewChatUsers();scheduleConversationList();const c=activeConversation();if(c)updateActiveConversationHeader(c);});
  socket.on('conversation-refresh',p=>{if(String(p?.id)!==String(activeConversationId))return;messageStore=Array.isArray(p.items)?p.items:[];renderCurrentMessages(true);});
  const applyMessageDelta=p=>{if(String(p?.id)!==String(activeConversationId)||!p?.item)return;const i=messageStore.findIndex(m=>String(m?.id)===String(p.item.id));if(i>=0)messageStore[i]=p.item;else messageStore.push(p.item);renderCurrentMessages(true);};
  socket.on('message-reactions',p=>{if(String(p?.id)===String(activeConversationId))window.KCChatReactions?.apply(p.item);});
  socket.on('message-edited',p=>{applyMessageDelta(p);requestConversationList();});
  socket.on('message-deleted',p=>{applyMessageDelta(p);requestConversationList();});socket.on('conversation-removed',p=>{if(String(p?.id)===String(activeConversationId))closeActiveConversation();requestConversationList();});
  const presenceSocket = socket;
  let presenceFrame = null;
  socket.on('presence', users => {
    if (presenceSocket !== socket) return;
    onlineUsers = users || [];
    if (presenceFrame !== null) return;
    presenceFrame = requestAnimationFrame(() => {
      presenceFrame = null;
      if (presenceSocket !== socket) return;
      renderPresence();
      renderActivityPanel();
      updateMentionPicker();
      renderConversationList();
      const invite = $('gameInviteModal');
      if (invite && !invite.classList.contains('hidden')) renderGameInviteUsers();
      const conversation = activeConversation();
      if (conversation) updateActiveConversationHeader(conversation);
    });
  });
  socket.on('my-policy',p=>{myPolicy=p||myPolicy;window.KCCohost?.refresh();applyFeaturePolicy();window.kc.refreshFeaturePolicy?.();updateMentionPicker();});
  socket.on('moderation-updated',d=>{if(d?.deviceId===cfg.deviceId||d?.profileKey===String(cfg.name||'').trim().toLowerCase()){myPolicy=d.policy||myPolicy;window.KCCohost?.refresh();showToast('HOST memperbarui role dan izin kamu.');applyFeaturePolicy();window.kc.refreshFeaturePolicy?.();updateMentionPicker();}if(cfg.isHost){if(!$('adminModal').classList.contains('hidden'))void loadAdmin(true);else{moderationInvalidation++;moderationLoadedAt=0;}}});
  socket.on('profile-updated',p=>{const key=String(p?.name||'').trim().toLowerCase();if(key)conversationProfileCache.set(key,p||{});renderPresence();renderNewChatUsers();refreshVisibleProfileNames(key);refreshComposerMentions(key);renderTypingIndicator();updateMentionPicker();if(key===String(cfg.name||'').trim().toLowerCase()){myProfile=p;renderProfilePreview();refreshProfileNameUI();}renderConversationList();const c=activeConversation();if(c)updateActiveConversationHeader(c);if(c?.members?.some(name=>profileKeyLocal(name)===key)&&$('chatInfoModal')?.classList.contains('hidden')===false)openChatInfo();});
  socket.on(window.kc.checkRustClientUpdate?'rust-client-update-available':'client-update-available',()=>setTimeout(checkClientUpdate,300));
  socket.on('diagnostics-updated',()=>{if(cfg.isHost&&!$('diagnosticsModal').classList.contains('hidden'))loadDiagnostics();});
  socket.on('asset-library-updated',()=>{renderAssetLibraries();});
  socket.on('storage-migration-status',d=>{if(!cfg.isHost)return;if(d?.active){$('storageMigrationStatus').textContent=`Memindahkan storage... ${d.copied||0}/${d.total||'?'} file${d.bytes?' · '+fmtSize(d.bytes):''}`;$('storageLocationSection')?.classList.add('migrating');}else{$('storageLocationSection')?.classList.remove('migrating');if(d?.error)$('storageMigrationStatus').textContent='Migrasi gagal: '+d.error;else if(d?.done)$('storageMigrationStatus').textContent=`Migrasi selesai · ${d.copied||0} file.`;loadStorageLocation();}});
}
function setConn(text,ok){const conn=$('connectionText'),status=$('status'),host=$('statusHost');if(conn)conn.textContent=text;if(status){status.textContent=(ok?'● ':'○ ')+text;status.style.color=ok?'#69b7ff':'#7c8e9f';}if(host){host.textContent=ok?(cfg?.isHost?'HOST online':'HOST online'):'HOST reconnect';host.classList.toggle('ok',!!ok);}const repair=$('repairConnection');if(repair)repair.classList.toggle('hidden',!!ok);renderDiscordProfileBar();}
function setAvatar(el,url,name){if(!el)return;el.dataset.profilePhoto=url||'';el.dataset.profileName=name||'';const inner=el.querySelector('span')||el;if(url){el.style.setProperty('--avatar-url',`url("${fullUrl(url)}")`);el.style.backgroundImage='var(--avatar-url)';el.classList.add('has-photo');inner.textContent='';}else{el.style.removeProperty('--avatar-url');el.style.backgroundImage='';el.classList.remove('has-photo');inner.textContent=(name||'?')[0].toUpperCase();}}
function avatarHtml(url,name,cls='message-avatar'){if(url)return`<div data-profile-photo="${esc(url)}" data-profile-name="${esc(name)}" class="${cls} has-photo" style="--avatar-url:url('${esc(fullUrl(url))}');background-image:var(--avatar-url)"></div>`;return`<div class="${cls}">${esc((name||'?')[0].toUpperCase())}</div>`;}
function renderPresence(){$('onlineCount').textContent=onlineUsers.length;if($('statusEditors'))$('statusEditors').textContent=`Editor: ${onlineUsers.length} online`;const presenceHtml=onlineUsers.map(u=>{const a=u.activitySharing===false?null:u.activity,sub=a?`${activityIcon(a)} ${activitySummary(a)}`:(u.pc||'online');return`<div class="person">${avatarHtml(u.avatarUrl,u.name,'person-avatar')}<div><b>${esc(profileDisplayName(u.name))}${u.isHost?' <em class="host-badge">HOST</em>':''}</b><small>${esc(sub)}</small></div></div>`;}).join('');if($('onlineList')._renderHtml!==presenceHtml){$('onlineList').innerHTML=presenceHtml;$('onlineList')._renderHtml=presenceHtml;}}
let activityTimer=null,lastActivityPayload='',lastLocalActivity=null;
function activityShareKey(){return'kc-share-activity:'+String(cfg?.name||'setup').toLowerCase();}
function activityShareEnabled(){try{const v=localStorage.getItem(activityShareKey());return v===null?true:v==='1';}catch{return true;}}
function setActivityShareEnabled(on){try{localStorage.setItem(activityShareKey(),on?'1':'0');}catch{}if($('shareActivity'))$('shareActivity').checked=!!on;publishLocalActivity(true);}
function friendlyAppName(raw=''){const k=String(raw||'').toLowerCase();const map={chrome:'Google Chrome',msedge:'Microsoft Edge',firefox:'Firefox',brave:'Brave',spotify:'Spotify',photoshop:'Adobe Photoshop',illustrator:'Adobe Illustrator',coreldraw:'CorelDRAW',code:'Visual Studio Code',explorer:'File Explorer',winword:'Microsoft Word',excel:'Microsoft Excel',powerpnt:'Microsoft PowerPoint',vlc:'VLC',potplayermini64:'PotPlayer',mpv:'MPV',lockapp:'PC Terkunci','chatx chat host':'ChatX','chatx chat client':'ChatX'};return map[k]||String(raw||'').replace(/[-_]+/g,' ').replace(/\b\w/g,m=>m.toUpperCase()).slice(0,80);}
function normalizeLocalActivity(raw={}){
  const app=String(raw.app||'').trim(),appKey=app.toLowerCase(),title=String(raw.title||'').trim(),mediaTitle=String(raw.mediaTitle||'').trim(),artist=String(raw.mediaArtist||'').trim(),source=String(raw.mediaSource||'').trim(),playback=String(raw.playback||'').toLowerCase();
  const mediaPlaying=/play/.test(playback);
  if(mediaPlaying&&mediaTitle){
    const sourceKey=(source+' '+app).toLowerCase(),music=/spotify|music|groove|foobar|itunes|deezer|tidal/.test(sourceKey)||!!artist;
    return{kind:music?'music':'video',app:friendlyAppName(app||source.split('!')[0]||'Media'),title:mediaTitle.slice(0,220),artist:artist.slice(0,140),source:source.slice(0,120),state:'playing'};
  }
  if(['chrome','msedge','firefox','brave'].includes(appKey)&&/youtube/i.test(title)){
    return{kind:'video',app:friendlyAppName(app),title:title.replace(/\s*[-–—]\s*YouTube.*$/i,'').trim().slice(0,220)||'YouTube',artist:'',source:'YouTube',state:'watching'};
  }
  if(/spotify/.test(appKey))return{kind:'music',app:'Spotify',title:(title&&!/^spotify/i.test(title)?title:'Mendengarkan musik').slice(0,220),artist:'',source:'Spotify',state:'listening'};
  if(/vlc|mpv|potplayer|mpc/.test(appKey))return{kind:'video',app:friendlyAppName(app),title:title.slice(0,220)||'Memutar video',artist:'',source:friendlyAppName(app),state:'watching'};
  if(/chatx/.test(appKey)||/chatx chat/i.test(title))return{kind:'chat',app:'ChatX',title:'Sedang membuka ChatX',artist:'',source:'',state:'active'};
  if(appKey==='lockapp')return{kind:'idle',app:'PC Terkunci',title:'PC sedang dikunci',artist:'',source:'',state:'locked'};
  return{kind:'app',app:friendlyAppName(app||'Desktop'),title:'',artist:'',source:'',state:'active'};
}
function activityIcon(a){return a?.kind==='music'?'🎵':a?.kind==='video'?'🎬':a?.kind==='chat'?'💬':a?.kind==='idle'?'🔒':'🖥️';}
function activitySummary(a){
  if(!a)return'Belum ada activity';
  if(a.kind==='music')return a.artist?`${a.title||'Musik'} — ${a.artist}`:(a.title||'Mendengarkan musik');
  if(a.kind==='video')return a.title||'Menonton video';
  if(a.kind==='chat')return a.title||'ChatX';
  if(a.kind==='idle')return a.title||'PC terkunci';
  return a.app?`Menggunakan ${a.app}`:'Aktif';
}
function renderProfileSettings(){if($('teamRoleSelect'))$('teamRoleSelect').value=myTeamRole();const p=myWorkTagPrefs();if($('workTagPrintTest'))$('workTagPrintTest').checked=p['print-test'];if($('workTagDesignRevision'))$('workTagDesignRevision').checked=p['design-revision'];if($('workTagDesignRequest'))$('workTagDesignRequest').checked=p['design-request'];}
function renderDiscordProfileBar(){const greeting=$('chatxWelcomeGreeting');if(greeting){const name=String(profileDisplayName(cfg?.name)||'').trim();greeting.textContent=name?`Halo, ${name}!`:'Halo!';}const bar=$('discordProfileBar');if(!bar)return;const online=!!socket?.connected,a=lastLocalActivity||{kind:'chat',app:'ChatX',title:'Online di ChatX',state:'active'};if($('userPanelName'))$('userPanelName').textContent=profileDisplayName(cfg?.name)||'Editor';if($('userPanelHost')){$('userPanelHost').classList.toggle('hidden',!cfg?.isHost&&!Object.values(myPolicy.cohost||{}).some(Boolean));$('userPanelHost').textContent=cfg?.isHost?'HOST':'Co-HOST';}if($('userPanelPresence'))$('userPanelPresence').textContent=(myTeamRole()==='ADMIN'?'Admin CS':myTeamRole())+' · '+(online?'● Online':'○ Menghubungkan…');if($('userPanelActivity'))$('userPanelActivity').textContent=activityShareEnabled()?`${activityIcon(a)} ${activitySummary(a)}`:'◌ Activity sharing dimatikan';bar.classList.toggle('offline',!online);setAvatar($('userPanelAvatar'),myProfile?.avatarUrl,cfg?.name||'Editor');}
function renderMyActivityPreview(){const box=$('myActivityPreview'),a=lastLocalActivity||{kind:'chat',app:'ChatX',title:'Online di ChatX',state:'active'};if(box){box.textContent=activityShareEnabled()?`${jobTitle({teamRole:myTeamRole(),isHost:cfg.isHost,cohost:Object.values(myPolicy.cohost||{}).some(Boolean)})} · ${activityIcon(a)} ${activitySummary(a)}`:`${myTeamRole()} · Activity sharing dimatikan.`;box.classList.toggle('off',!activityShareEnabled());}if($('shareActivity'))$('shareActivity').checked=activityShareEnabled();renderProfileSettings();renderDiscordProfileBar();}
function ensureStatusWorkspaceMounted(){
  const modal=$('activityModal'),shell=document.querySelector('.whatsapp-shell');
  if(modal&&shell&&modal.parentElement!==shell)shell.appendChild(modal);
}
function statusWorkspaceReset(){
  $('statusWorkspaceEmpty')?.classList.remove('hidden');
  $('statusWorkspaceViewer')?.classList.add('hidden');
  $('activityWorkspaceViewer')?.classList.add('hidden');
  document.querySelectorAll('#statusList .status-card.active,#activityList .activity-row.active').forEach(x=>x.classList.remove('active'));
  const media=$('statusViewerBody')?.querySelector('video,audio');try{media?.pause?.();}catch(_){ }if($('statusReplyInput'))$('statusReplyInput').value='';$('statusReplyBar')?.classList.add('hidden');
}
function statusWorkspaceIsOpen(){return !!$('activityModal')&&!$('activityModal').classList.contains('hidden');}
function openStatusWorkspace(){
  if(chatImageWorkspaceOpen)closeChatImageWorkspace();ensureStatusWorkspaceMounted();statusWorkspaceReset();renderStatuses();renderActivityPanel();requestStatuses();
  document.body.classList.add('status-workspace-open');$('activityModal')?.classList.remove('hidden');
  document.querySelectorAll('.wa-rail button').forEach(b=>b.classList.toggle('active',b.id==='railActivity'));
}
function closeStatusWorkspace(){
  if(!statusWorkspaceIsOpen())return;statusWorkspaceReset();$('activityModal')?.classList.add('hidden');document.body.classList.remove('status-workspace-open');
  $('railActivity')?.classList.remove('active');$('railChats')?.classList.add('active');
}
function setWorkspaceAvatar(el,user,url){if(!el)return;if(url){el.classList.add('has-photo');el.style.backgroundImage=`url("${fullUrl(url)}")`;el.textContent='';}else{el.classList.remove('has-photo');el.style.backgroundImage='';el.textContent=String(user||'?').slice(0,1).toUpperCase();}}
function openActivityWorkspace(userName){
  const u=(onlineUsers||[]).find(x=>profileKeyLocal(x?.name)===profileKeyLocal(userName));if(!u)return;
  const a=u.activity,share=u.activitySharing!==false;
  $('statusWorkspaceEmpty')?.classList.add('hidden');$('statusWorkspaceViewer')?.classList.add('hidden');$('activityWorkspaceViewer')?.classList.remove('hidden');
  setWorkspaceAvatar($('activityViewerAvatar'),u.name,u.avatarUrl);$('activityViewerName').textContent=profileDisplayName(u.name)||'Editor';$('activityViewerPc').textContent=u.pc||'PC';
  $('activityViewerIcon').textContent=share?activityIcon(a):'◌';$('activityViewerTitle').textContent=share?activitySummary(a):'Activity sharing dimatikan';$('activityViewerApp').textContent=share?(a?.app||'ChatX'):'';$('activityViewerUpdated').textContent=u.activityUpdatedAt?'Update '+new Date(u.activityUpdatedAt).toLocaleTimeString('id-ID',{hour:'2-digit',minute:'2-digit'}):'Realtime';
  document.querySelectorAll('#statusList .status-card.active,#activityList .activity-row.active').forEach(x=>x.classList.remove('active'));document.querySelector(`#activityList .activity-row[data-activity-user="${CSS.escape(profileKeyLocal(u.name))}"]`)?.classList.add('active');
}
function renderActivityPanel(){
  const box=$('activityList');if(!box)return;
  const clients=(onlineUsers||[]).filter(u=>!u.isHost).sort((a,b)=>String(a.name||'').localeCompare(String(b.name||'')));
  if(!clients.length){box.innerHTML='<div class="activity-empty compact">Belum ada CLIENT online.</div>';return;}
  box.innerHTML=clients.map(u=>{const a=u.activity,share=u.activitySharing!==false;const updated=u.activityUpdatedAt?new Date(u.activityUpdatedAt).toLocaleTimeString('id-ID',{hour:'2-digit',minute:'2-digit'}):'';return`<div class="activity-row" data-activity-user="${esc(profileKeyLocal(u.name))}">${avatarHtml(u.avatarUrl,u.name,'activity-avatar')}<div class="activity-main"><div class="activity-name"><b>${esc(profileDisplayName(u.name))}</b><span>${u.activityAccessOverride?'Pengecualian Host/Admin':esc(updated||u.pc||'')}</span></div><div class="activity-now ${share?'':'off'}">${share?`${activityIcon(a)} ${esc(activitySummary(a))}`:'◌ Activity sharing dimatikan'}</div></div>${cfg?.isHost?`<button class="activity-access-button" type="button" data-activity-access="${esc(u.deviceId)}">Atur akses</button>`:''}</div>`;}).join('');
  box.querySelectorAll('[data-activity-user]').forEach(row=>row.onclick=()=>openActivityWorkspace(row.dataset.activityUser));
  box.querySelectorAll('[data-activity-access]').forEach(button=>button.onclick=e=>{e.stopPropagation();if(typeof openActivityVisibility==='function')void openActivityVisibility(button.dataset.activityAccess);});
}
let activityReadSerial=0;
async function publishLocalActivity(force=false){
 const target=socket;if(!target?.connected||!window.kc?.getLocalActivity)return;
 const serial=++activityReadSerial,epoch=target._kcSessionEpoch,enabled=activityShareEnabled();
 const current=()=>serial===activityReadSerial&&socket===target&&target.connected&&target._kcSessionEpoch===epoch;
 const publish=a=>{if(!current())return;lastLocalActivity=a;renderMyActivityPreview();const payload=JSON.stringify({enabled,activity:a});if(force||payload!==lastActivityPayload){target.emit('activity-update',{enabled,activity:a});lastActivityPayload=payload;}};
 const fallback=()=>({kind:'chat',app:'ChatX',title:'Online di ChatX',artist:'',source:'',state:'active'});
 try{const r=await window.kc.getLocalActivity();if(!current())return;publish(r?.ok?normalizeLocalActivity(r.activity||{}):fallback());}catch(e){if(!current())return;techLog('activity-read-failed',{message:e?.message||String(e)});publish(fallback());}
}

function startActivityReporter(){
 if(activityTimer)clearInterval(activityTimer);
 activityTimer=null;
 publishLocalActivity(true);
 activityTimer=setInterval(()=>publishLocalActivity(false),15000);
}
function workTagAppliesToMe(m){if(!m?.workTag)return false;if(myWorkTagPrefs()[m.workTag]===false)return false;const t=m.workTagTarget;if(!t||t.type==='all')return true;return t.type==='user'&&profileKeyLocal(t.name)===profileKeyLocal(cfg.name);}
function mentionForMe(text,message){if(Array.isArray(message?.mentionRecipients))return message.mentionMass===true||message.mentionRecipients.some(name=>profileKeyLocal(name)===profileKeyLocal(cfg.name));const t=(text||'').toLowerCase(),me='@'+cfg.name.toLowerCase();return t.includes(me)||t.includes('@everyone')||t.includes('@editor')||t.includes('@semua');}
function replySummary(r){if(!r)return'';const content=messageDisplayText(r)||r.fileName||'Pesan';return`<div class="reply-quote" data-jump="${esc(r.id)}"><b>${esc(profileDisplayName(r.sender))}</b><span>${r.isImage?'🖼️ ':''}${esc(content)}</span></div>`;}
function statusReplySummary(r){if(!r)return'';const media=r.type==='image'&&r.mediaUrl?`<img class="status-reply-thumb" src="${esc(fullUrl(r.mediaUrl))}" alt="Status">`:'';const text=(r.text||r.mediaName||({video:'Status video',audio:'Status audio',image:'Status gambar'}[r.type])||'Status').slice(0,180);return`<div class="status-reply-card ${media?'':'no-media'}" data-status-reply="${esc(r.id||'')}">${media}<div class="status-reply-copy"><b>↩ Membalas status ${esc(r.user||'')}</b><span>${esc(text)}</span><small>${r.createdAt?new Date(r.createdAt).toLocaleString('id-ID'):'Status'}</small></div></div>`;}
const notifThreadCache=new Map();
function notificationThreadFor(m){
  const sender=String(m?.sender||'Editor'),threadKey=String(m?.conversationId||'general'),now=Date.now(),prev=notifThreadCache.get(threadKey)||[];
  const fresh=prev.filter(x=>now-x.ts<18000).slice(-3);
  fresh.push({id:String(m?.id||''),ts:now,sender:profileDisplayName(sender),text:messageDisplayText(m),fileName:String(m?.file?.name||''),imageUrl:m?.file&&isImage(m.file)?fullUrl(m.file.url):null,mention:m?.workTag?workTagAppliesToMe(m):mentionForMe(m?.text||'',m),mentionLevel:m?.mentionLevel||'notice',fullscreenMode:m?.fullscreenMode||'video',tagVideoEnabled:m?.tagVideoEnabled,tagScope:m?.tagScope||null,bledarVideo:m?.bledarVideo||null});
  const list=fresh.slice(-4);notifThreadCache.set(threadKey,list);return list;
}
function latestNotificationTag(m){return{mentionLevel:m.mentionLevel,fullscreenMode:m.fullscreenMode,bledarVideo:m.bledarVideo};}
const dispatchedNotificationIds=new Map();
async function sendRichNotification(m){
  const notificationStartedAt=Date.now();
  const id=String(m?.id||'');if(id&&dispatchedNotificationIds.has(id)){techLog('notification-skipped',{messageId:id,reason:'duplicate-event'});return;}if(id){dispatchedNotificationIds.set(id,Date.now());if(dispatchedNotificationIds.size>2000)dispatchedNotificationIds.delete(dispatchedNotificationIds.keys().next().value);}unread++;updateUnread();
  const messages=notificationThreadFor(m),tagForMe=m?.workTag?workTagAppliesToMe(m):mentionForMe(m?.text||'',m),mention=tagForMe,tagged=tagForMe?latestNotificationTag(m):null,latest=messages[messages.length-1],scope=m?.tagScope||(mention?conversationTagScope(conversations.find(c=>String(c.id)===String(m?.conversationId||'general'))):null);
  let chosenVideo=m?.bledarVideo||tagged?.bledarVideo;if(mention&&m.tagVideoEnabled!==false&&!chosenVideo&&(m.tagVideoEnabled===true||(tagged?.mentionLevel||m.mentionLevel)==='fullscreen')){try{const r=await kcFetch(serverBase+'/bledar-videos/pick',{signal:AbortSignal.timeout(1800),cache:'no-store'}),j=await r.json();if(!r.ok||!j.ok||!j.video)throw Error(j.error||'HOST belum menyediakan pilihan video');chosenVideo=j.video;}catch(e){reportBug('bledar-selection-unavailable',e,'warning',{messageId:m.id});}}const payload={notificationStartedAt,notificationDispatchedAt:Date.now(),tagVideoEnabled:m.tagVideoEnabled,bledarVideo:chosenVideo?{...chosenVideo,url:fullUrl(chosenVideo.url)}:null,title:profileDisplayName(m.sender),body:messageDisplayText(latest)||(latest.fileName?'Mengirim file: '+latest.fileName:'Pesan baru'),messages,conversationType:m.conversationType,mention,tagScope:scope,tagLabel:mention?tagScopeLabel(scope):'',mentionLevel:tagged?.mentionLevel||m.mentionLevel||'notice',fullscreenMode:tagged?.fullscreenMode||m.fullscreenMode||'video',bypassWindowsMute:!!m?.bypassWindowsMute,messageId:m.id,conversationId:m.conversationId||'general',imageUrl:latest.imageUrl||null,theme:document.documentElement.dataset.theme||'dark',accent:appearancePrefs?.accent||'#1765cb'};let delivered=false;try{for(let attempt=0;attempt<2;attempt++){try{const ack=await window.kc.sendNotification(payload);if(ack?.ok===false)throw Error(ack.error||'Notifikasi ditolak');delivered=true;techLog('notification-accepted',{messageId:id,attempt});break;}catch(e){techLog('notification-ipc-retry',{messageId:id,attempt,reason:String(e.message).slice(0,160)});if(attempt===1)throw e;}}}finally{if(!delivered&&id)dispatchedNotificationIds.delete(id);}
}
const selectedMessageIds=new Set();
function getMessageById(id){return messageStore.find(m=>String(m?.id||'')===String(id||''))||null;}
function updateMessageSelectionUI(){
  document.querySelectorAll('.msg-row').forEach(row=>row.classList.toggle('selected-message',selectedMessageIds.has(String(row.dataset.messageId||''))));
  const bar=$('messageSelectionBar'),count=$('messageSelectionCount'),replyBtn=$('selectionReply'),deleteBtn=$('selectionDelete');
  if(count)count.textContent=String(selectedMessageIds.size);
  bar?.classList.toggle('hidden',selectedMessageIds.size===0);
  if(replyBtn)replyBtn.disabled=selectedMessageIds.size!==1;
  if(deleteBtn){
    const deletable=[...selectedMessageIds].map(getMessageById).filter(Boolean).some(m=>!m.deleted);
    deleteBtn.disabled=!deletable;
  }
}
function toggleMessageSelection(m,force){
  if(!m?.id)return;
  const id=String(m.id),want=typeof force==='boolean'?force:!selectedMessageIds.has(id);
  if(want)selectedMessageIds.add(id);else selectedMessageIds.delete(id);
  updateMessageSelectionUI();
}
function clearMessageSelection(){selectedMessageIds.clear();updateMessageSelectionUI();}
function copySelectedMessages(){
  const items=[...selectedMessageIds].map(getMessageById).filter(Boolean).sort((a,b)=>(a.ts||0)-(b.ts||0));
  if(!items.length)return;
  const text=items.map(m=>`${m.sender}: ${m.text||m.file?.name||'[Pesan]'}`).join('\n');
  navigator.clipboard?.writeText(text).then(()=>showToast(`${items.length} pesan disalin.`)).catch(()=>showToast('Gagal menyalin pesan.'));
}
function messageDeleteChoice({title='Hapus pesan?',count=1}={}){return new Promise(resolve=>{
  let modal=$('messageDeleteModal');if(!modal){modal=document.createElement('div');modal.id='messageDeleteModal';modal.className='modal hidden';modal.innerHTML=`<div class="modal-card confirm-card delete-choice-card"><div class="modal-head"><div><b id="messageDeleteTitle">Hapus pesan?</b><span id="messageDeleteText">Pilih tindakan.</span></div><button id="messageDeleteClose">×</button></div><div class="delete-choice-actions"><button id="deleteForMe" class="secondary">Hapus untuk saya</button><button id="deleteForEveryone" class="danger">Hapus untuk semua orang</button><button id="deleteCancel" class="secondary">Batal</button></div></div>`;document.body.appendChild(modal);}
  $('messageDeleteTitle').textContent=title;$('messageDeleteText').textContent=count>1?`${count} pesan dipilih.`:'Pesan ini akan hilang dari tampilan akun kamu.';const all=$('deleteForEveryone');all.classList.toggle('hidden',!cfg?.isHost);
  let done=false;const finish=v=>{if(done)return;done=true;modal.classList.add('hidden');resolve(v);};$('deleteForMe').onclick=()=>finish('me');all.onclick=()=>finish('everyone');$('deleteCancel').onclick=()=>finish(null);$('messageDeleteClose').onclick=()=>finish(null);modal.onclick=e=>{if(e.target===modal)finish(null)};modal.classList.remove('hidden');
});}
async function deleteSelectedMessages(){
  const items=[...selectedMessageIds].map(getMessageById).filter(m=>m&&!m.deleted);
  if(!items.length)return showToast('Tidak ada pesan terpilih.');
  const choice=await messageDeleteChoice({title:`Hapus ${items.length} pesan?`,count:items.length});if(!choice)return;
  if(choice==='me'){hideMessagesLocal(items.map(m=>m.id));clearMessageSelection();showToast(`${items.length} pesan dihapus untuk kamu.`);return;}
  if(!cfg?.isHost)return showToast('Hanya HOST yang dapat menghapus untuk semua orang.');
  let ok=0,fail=0;for(const m of items){await new Promise(resolve=>socket.emit('delete-message',{id:m.id,scope:'everyone'},r=>{if(r?.ok)ok++;else fail++;resolve();}));}
  clearMessageSelection();showToast(fail?`${ok} pesan dihapus untuk semua · ${fail} gagal`:`${ok} pesan dihapus untuk semua orang.`);
}
function replySelectedMessage(){if(selectedMessageIds.size!==1)return;const m=getMessageById([...selectedMessageIds][0]);if(m){clearMessageSelection();setReply(m);}}
function conversationImages(){return (messageStore||[]).filter(m=>!m.deleted&&m.file&&isImage(m.file)).map(m=>({messageId:String(m.id),url:fullUrl(m.file.url),downloadUrl:fullUrl(m.file.downloadUrl||`/download/${encodeURIComponent(m.file.stored||'')}`),name:m.file.name||'Gambar',sender:m.sender||'',ts:Number(m.ts||0)}));}
async function chatImagePng(url) {
  const response = await kcFetch(url);
  if (!response.ok) throw Error('Gambar tidak bisa dibaca (HTTP ' + response.status + ').');
  const blob = await response.blob();
  const signature = new Uint8Array(await blob.slice(0, 8).arrayBuffer());
  if (signature.length === 8 && new DataView(signature.buffer).getBigUint64(0) === 0x89504e470d0a1a0an) {
    return blob.type === 'image/png' ? blob : new Blob([blob], {type:'image/png'});
  }
  const bitmap = await createImageBitmap(blob);
  try {
    const canvas = document.createElement('canvas');
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    canvas.getContext('2d').drawImage(bitmap, 0, 0);
    return await new Promise((resolve,reject) => canvas.toBlob(
      png => png ? resolve(png) : reject(Error('Gagal membuat PNG.')), 'image/png'));
  } finally { bitmap.close(); }
}
function pngBlobBase64(blob) {
  return new Promise((resolve,reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(',')[1]);
    reader.onerror = () => reject(Error('Gagal membaca gambar.'));
    reader.readAsDataURL(blob);
  });
}
async function copyChatImageToClipboard(url) {
  try {
    const png = chatImagePng(url);
    let copied = false;
    if (navigator.clipboard?.write && typeof ClipboardItem === 'function') {
      try {
        await navigator.clipboard.write([new ClipboardItem({'image/png':png})]);
        copied = true;
      } catch {}
    }
    if (!copied) {
      const base64 = await pngBlobBase64(await png);
      const result = await window.kc.setClipboardPng(base64);
      if (!result?.ok) throw Error(result?.error || 'Clipboard gagal.');
    }
    showToast('Gambar disalin ke clipboard.');
    return true;
  } catch (error) {
    showToast('Gagal menyalin gambar: ' + (error.message || error));
    return false;
  }
}
window.addEventListener('kc-download-status', event => {
  if (typeof event.detail === 'string') showToast(event.detail, 4500);
});
async function downloadChatAttachment({url,name}) {
  try {
    if (window.kc.downloadChatFile) {
      showToast('Pilih lokasi untuk menyimpan lampiran.');
      const result = await window.kc.downloadChatFile({url,name});
      if (result?.canceled) showToast('Unduhan dibatalkan.');
      return result;
    }
    const target = new URL(url, serverBase);
    if (target.origin !== new URL(serverBase).origin || !target.pathname.startsWith('/download/')) {
      throw Error('Alamat lampiran tidak valid.');
    }
    // The download URL already carries the session token. Let WebView stream it to disk.
    const link = document.createElement('a');
    link.href = target.href;
    link.download = name;
    document.body.append(link);
    link.click();
    link.remove();
    return {canceled:true};
  } catch (error) {
    return {ok:false,error:error.message || String(error)};
  }
}
function bindScrollableImage(stage,img){
  if(!stage||!img)return;
  let zoom=1,x=0,y=0,drag=null;
  function paint(){img.style.transform=`translate(${x}px, ${y}px) scale(${zoom})`;img.style.cursor=zoom>1?'grab':'zoom-in';}
  function reset(){zoom=1;x=0;y=0;paint();}
  img.addEventListener('load',reset);
  stage.addEventListener('wheel',e=>{
    if(e.target.closest('button'))return;e.preventDefault();
    const next=Math.max(1,Math.min(8,zoom*Math.exp(-e.deltaY*.002)));
    const rect=stage.getBoundingClientRect(),px=e.clientX-rect.left-rect.width/2,py=e.clientY-rect.top-rect.height/2;
    x=px-(px-x)*next/zoom;y=py-(py-y)*next/zoom;zoom=next;
    if(zoom===1){x=0;y=0;}paint();
  },{passive:false});
  img.addEventListener('pointerdown',e=>{if(zoom<=1||e.button!==0)return;e.preventDefault();drag={x:e.clientX,y:e.clientY,ox:x,oy:y};img.setPointerCapture(e.pointerId);img.style.cursor='grabbing';});
  img.addEventListener('pointermove',e=>{if(!drag)return;x=drag.ox+e.clientX-drag.x;y=drag.oy+e.clientY-drag.y;paint();});
  function release(){drag=null;paint();}
  img.addEventListener('pointerup',release);img.addEventListener('pointercancel',release);
  img.addEventListener('dblclick',e=>{e.preventDefault();reset();});
  reset();return reset;
}
function centerAttachmentThumbnail(strip,active,behavior='smooth'){
  if(!strip||!active)return;
  strip.scrollTo({left:active.offsetLeft-strip.offsetLeft+active.offsetWidth/2-strip.clientWidth/2,behavior});
}
function sizeChatImageWorkspace(){chatImageLayoutRaf=0;if(!chatImageWorkspaceOpen)return;const stage=$('chatImageStage'),img=$('chatImageFull'),thumbs=$('chatImageThumbs'),head=document.querySelector('.chat-image-head');if(!stage||!img)return;const stageRect=stage.getBoundingClientRect();const thumbsH=Math.round((thumbs?.getBoundingClientRect()?.height)||0);const headH=Math.round((head?.getBoundingClientRect()?.height)||0);const viewportH=window.innerHeight||document.documentElement.clientHeight||0;const reserved=Math.max(0,thumbsH+headH+10);const available=Math.max(140,Math.min(stage.clientHeight||0,viewportH-reserved));const padding=getComputedStyle(stage),vertical=parseFloat(padding.paddingTop)+parseFloat(padding.paddingBottom),horizontal=parseFloat(padding.paddingLeft)+parseFloat(padding.paddingRight);img.style.maxHeight=Math.max(1,available-vertical)+'px';img.style.maxWidth=Math.max(1,(stage.clientWidth||0)-horizontal)+'px';}
function queueChatImageWorkspaceLayout(){if(chatImageLayoutRaf)cancelAnimationFrame(chatImageLayoutRaf);chatImageLayoutRaf=requestAnimationFrame(sizeChatImageWorkspace);}
function renderChatImageWorkspace(){const item=chatImageSequence[chatImageIndex];if(!item)return closeChatImageWorkspace();const loaded=new Map([...document.querySelectorAll('#messages .img-preview')].filter(image=>image.complete&&image.naturalWidth>0).map(image=>[String(image.dataset.openImage),image.currentSrc||image.src]));const img=$('chatImageFull'),stage=$('chatImageStage'),source=loaded.get(item.messageId)||item.url;img.decoding='async';img.fetchPriority='high';img.loading='eager';img.onload=()=>{stage.dataset.imageLoading='';$('chatImageRetry')?.classList.add('hidden');queueChatImageWorkspaceLayout();};img.onerror=()=>{stage.dataset.imageLoading='error';$('chatImageRetry')?.classList.remove('hidden');};$('chatImageRetry')?.classList.add('hidden');if(img.getAttribute('src')!==source){stage.dataset.imageLoading='1';img.src=source;}if(img.complete&&img.naturalWidth>0)stage.dataset.imageLoading='';$('chatImageTitle').textContent=item.name;$('chatImageMeta').textContent=`${item.sender||'Chat'} · ${new Date(item.ts||Date.now()).toLocaleString('id-ID')} · ${chatImageIndex+1}/${chatImageSequence.length}`;$('downloadChatImage').href=item.downloadUrl;$('downloadChatImage').setAttribute('download',item.name||'image');$('downloadChatImage').onclick=async e=>{e.preventDefault();const r=await downloadChatAttachment({url:item.downloadUrl,name:item.name||'image'});if(r?.ok)showToast('File disimpan: '+r.path,4200);else if(!r?.canceled)showToast(r?.error||'Download gagal');};$('chatImagePrev').disabled=chatImageIndex<=0;$('chatImageNext').disabled=chatImageIndex>=chatImageSequence.length-1;const thumbs=$('chatImageThumbs');const indices=window.KCChatImageWindow?.indices(chatImageSequence.length,chatImageIndex)||chatImageSequence.map((_,i)=>i);thumbs.innerHTML=indices.map(i=>{const x=chatImageSequence[i],cached=loaded.get(x.messageId);return `<button type="button" class="chat-image-thumb ${i===chatImageIndex?'active':''}" data-image-index="${i}" title="${esc(x.name)}"><img src="${esc(cached||x.url)}" loading="lazy" decoding="async" fetchpriority="low" alt=""></button>`;}).join('');thumbs.querySelectorAll('[data-image-index]').forEach(b=>b.onclick=e=>{e.stopPropagation();chatImageIndex=Number(b.dataset.imageIndex)||0;renderChatImageWorkspace();});requestAnimationFrame(()=>{centerAttachmentThumbnail(thumbs,thumbs.querySelector('.chat-image-thumb.active'));queueChatImageWorkspaceLayout();});}
function openChatImageWorkspace(messageId){chatImageSequence=conversationImages();if(!chatImageSequence.length)return;chatImageIndex=Math.max(0,chatImageSequence.findIndex(x=>String(x.messageId)===String(messageId)));chatImageWorkspaceOpen=true;document.body.classList.add('chat-image-workspace-open');$('chatImageWorkspace').classList.remove('hidden');renderChatImageWorkspace();queueChatImageWorkspaceLayout();}
function closeChatImageWorkspace(){if(!chatImageWorkspaceOpen&&!document.body.classList.contains('chat-image-workspace-open'))return;chatImageWorkspaceOpen=false;if(chatImageLayoutRaf){cancelAnimationFrame(chatImageLayoutRaf);chatImageLayoutRaf=0;}document.body.classList.remove('chat-image-workspace-open');$('chatImageWorkspace')?.classList.add('hidden');$('chatImageFull')?.removeAttribute('src');focusChatComposer(false);}
function stepChatImage(dir){const next=Math.max(0,Math.min(chatImageSequence.length-1,chatImageIndex+dir));if(next===chatImageIndex)return;chatImageIndex=next;renderChatImageWorkspace();}
function chatDisplayFileName(m){const original=String(m?.file?.name||'file'),stored=String(m?.file?.stored||'');if(!stored||!Array.isArray(messageStore))return original;const peers=messageStore.filter(x=>x?.file&&String(x.file.name||'')===original&&String(x.file.stored||'')).sort((a,b)=>Number(a.ts||0)-Number(b.ts||0));const unique=[...new Set(peers.map(x=>String(x.file.stored)))];if(unique.length<2)return original;const index=unique.indexOf(stored);if(index<=0)return original;const ext=original.match(/(\.[^.]+)$/)?.[1]||'',stem=ext?original.slice(0,-ext.length):original;return `${stem} (${index+1})${ext}`;}
function retryChatImage(img){if(!img)return;img.onerror=()=>{const n=Number(img.dataset.retryCount||0);if(n>=8){img.classList.add('image-load-error');img.onerror=null;return;}img.dataset.retryCount=String(n+1);setTimeout(()=>{if(!img.isConnected)return;try{const u=new URL(img.src);u.searchParams.set('kc_retry',`${n+1}-${Date.now()}`);retryChatImage(img);img.src=u.href;}catch{}},Math.min(8000,800*(n+1)));};}
function chatDoubleClickCanReply(event){
  if(event.target.closest('button,a,img,[data-jump],.body,.sender,.meta,.file-info,.image-name,.worktag-badge,.deleted-message'))return false;
  return !String(window.getSelection?.()||'').trim();
}
function renderMessage(m,notify,target=null){
  if(isMessageHidden(m))return;
  const {receipt,readBy,reactions,...content}=m;const contentKey=JSON.stringify([content,chatDisplayFileName(m),serverBase,cfg.name,cfg.isHost,m.file?fullUrl(m.file.url):'',m.senderAvatar?fullUrl(m.senderAvatar):'']);const existing=messageRowsForRefresh?.get(String(m.id));if(existing&&existing._renderContentKey===contentKey){updateExistingReceipt(existing,m);window.KCChatReactions?.update(existing,m);existing.classList.toggle('selected-message',selectedMessageIds.has(String(m.id)));(target||$('messages')).appendChild(existing);return;}
  const mine=m.sender===cfg.name,row=document.createElement('div');row._renderContentKey=contentKey;if(existing)row.dataset.kcMotion='refresh';row.className='msg-row'+(mine?' mine':'')+(m.sticker?' sticker-message':'')+(m.workTag?' worktag-'+m.workTag:'');row.dataset.messageId=m.id;row.dataset.dayKey=messageDayKey(m.ts);
  let file='';
  if(m.file){const view=fullUrl(m.file.url),down=fullUrl(m.file.downloadUrl||`/download/${encodeURIComponent(m.file.stored||'')}`),downloadName=esc(chatDisplayFileName(m));if(isImage(m.file)){file=`<div class="image-attachment"><img class="img-preview" src="${esc(view)}" loading="lazy" decoding="async" data-open-image="${esc(m.id)}"><div class="image-actions"><div class="image-name"><b>${downloadName}</b><span>${fmtSize(m.file.size||0)}${m.file.hash?' · tersimpan unik':''}</span></div><div class="image-action-buttons"><button class="copy-image-btn" data-copy-image="${esc(view)}" type="button">⧉ Salin</button><a class="download-btn" data-chat-download href="${esc(down)}" download="${downloadName}">⬇ Unduh</a></div></div></div>`;}else{file=`<div class="file-card"><div class="file-icon">${esc((m.file.name.split('.').pop()||'FILE').toUpperCase())}</div><div class="file-info"><b>${downloadName}</b><span>${fmtSize(m.file.size||0)}</span></div><a data-chat-download href="${esc(down)}" download="${downloadName}">UNDUH</a></div>`;}}
  const canEdit=(mine||cfg?.isHost)&&!m.deleted,canDelete=!m.deleted;
  const tagScope=m.tagScope||((m.workTag||mentionForMe(m.text||'',m))?conversationTagScope(conversations.find(c=>String(c.id)===String(m.conversationId||activeConversationId))):null),tagScopeBadge=tagScope?`<div class="tag-scope-badge ${tagScope==='private'?'private':'group'}">${tagScope==='private'?'👤 TAG PRIBADI':'👥 TAG GRUP'}</div>`:'';
  const groupReads=Object.keys(m.readBy||{}).length;const receiptMarkup=mine?(m.conversationType==='dm'?`<span class="message-receipt${m.receipt?.readAt?' read':m.receipt?.deliveredAt?' delivered':''}" title="${m.receipt?.readAt?'Dibaca':m.receipt?.deliveredAt?'Terkirim ke perangkat penerima':'Penerima offline / belum terkirim'}">${window.KCChatReceipts?.icon(!!(m.receipt?.deliveredAt||m.receipt?.readAt))||(m.receipt?.deliveredAt||m.receipt?.readAt?'✓✓':'✓')}</span>`:`<span class="message-receipt${groupReads?' read':''}" title="${groupReads?groupReads+' anggota sudah membaca':'Belum dibaca'}">${window.KCChatReceipts?.icon(true)||'✓✓'}</span>`):'';
  row.innerHTML=`${mine?'':avatarHtml(currentProfileAvatar(m.sender,m.senderAvatar),m.sender)}<div class="bubble"><button type="button" class="message-menu-arrow" title="Menu pesan" aria-label="Menu pesan" aria-haspopup="menu"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 6 4 4 4-4"/></svg></button><button type="button" class="message-menu-emoji" title="Beri reaksi" aria-label="Beri reaksi" aria-haspopup="dialog">☺</button><div class="message-actions"><button class="select-action" title="Pilih pesan">✓</button><button class="reply-action" title="Reply">↩</button>${canEdit?'<button class="edit-action" title="Edit pesan">✎</button>':''}${canDelete?'<button class="delete-action" title="Hapus pesan">🗑</button>':''}</div><div class="sender">${mine?'Kamu':esc(profileDisplayName(m.sender))}</div>${tagScopeBadge}${m.workTag?`<div class="worktag-badge ${esc(m.workTag)}">${m.workTag==='print-test'?'🧪 TES CETAK':m.workTag==='design-request'?'🎨 MINTA DESAIN':'✏️ REVISI DESAIN'}</div>`:''}${m.deleted?'<div class="deleted-message">🚫 Pesan telah dihapus</div>':`${replySummary(m.reply)}${statusReplySummary(m.statusReply)}${file}${m.text?`<div class="body">${esc(messageDisplayText(m))}</div>`:''}`}<div class="meta"><span>${new Date(m.ts).toLocaleTimeString('id-ID',{hour:'2-digit',minute:'2-digit'})}${m.editedAt?' · diedit':''}</span>${receiptMarkup}</div></div>`;
  (target||$('messages')).appendChild(row);
  row.classList.toggle('selected-message',selectedMessageIds.has(String(m.id)));
  row.querySelectorAll('.img-preview').forEach(retryChatImage);row.querySelectorAll('[data-open-image]').forEach(img=>img.onclick=e=>{e.stopPropagation();if(m.sticker&&m.file?.url&&!m.deleted){if(selectedMessageIds.size){toggleMessageSelection(m);return;}const bounds=img.getBoundingClientRect();showMessageContextMenu(m,{clientX:bounds.right,clientY:bounds.top});return;}openChatImageWorkspace(img.dataset.openImage);});row.querySelectorAll('[data-copy-image]').forEach(btn=>btn.onclick=async e=>{e.stopPropagation();await copyChatImageToClipboard(btn.dataset.copyImage);});row.querySelectorAll('[data-chat-download]').forEach(a=>a.onclick=async e=>{e.preventDefault();e.stopPropagation();const r=await downloadChatAttachment({url:a.href,name:a.download||m.file?.name||'file'});if(r?.ok)showToast('File disimpan: '+r.path,4200);else if(!r?.canceled)showToast(r?.error||'Download gagal');});
  row.querySelector('.message-menu-arrow').onclick=e=>{e.preventDefault();e.stopPropagation();const menu=$('messageContextMenu');if(menu&&!menu.classList.contains('hidden')&&menu.dataset.messageId===String(m.id)){window.KCChatReactions?.close();menu.classList.add('hidden');return;}const rect=e.currentTarget.getBoundingClientRect();showMessageContextMenu(m,{clientX:rect.right,clientY:rect.bottom+6});};
  row.querySelector('.message-menu-emoji').onclick=e=>{e.preventDefault();e.stopPropagation();window.KCChatReactions?.open(m,e.currentTarget);};
  window.KCChatReactions?.update(row,m);
  row.querySelector('.select-action').onclick=e=>{e.stopPropagation();toggleMessageSelection(m);};
  row.querySelector('.reply-action').onclick=e=>{e.stopPropagation();clearMessageSelection();setReply(m);};
  row.querySelector('.edit-action')?.addEventListener('click',e=>{e.stopPropagation();clearMessageSelection();startEdit(m);});
  row.querySelector('.delete-action')?.addEventListener('click',e=>{e.stopPropagation();deleteMessage(m);});
  row.addEventListener('click',e=>{if(selectedMessageIds.size&&!e.target.closest('button,a,img,[data-jump]'))toggleMessageSelection(m);});
  row.addEventListener('dblclick',e=>{if(selectedMessageIds.size||!chatDoubleClickCanReply(e))return;e.preventDefault();setReply(m);});
  row.addEventListener('contextmenu',e=>{if(e.target.closest('[data-jump]'))return;e.preventDefault();e.stopPropagation();showMessageContextMenu(m,e);});
  row.querySelectorAll('[data-jump]').forEach(el=>el.onclick=()=>jumpToMessage(el.dataset.jump));row.querySelectorAll('[data-status-reply]').forEach(el=>el.onclick=e=>{e.stopPropagation();const id=el.dataset.statusReply,st=(statusItems||[]).find(x=>String(x.id)===String(id));if(!st)return showToast('Status ini sudah kedaluwarsa / dihapus.');openStatusWorkspace();setTimeout(()=>openStatusViewer(id),30);});
  if(notify&&!mine){sendRichNotification(m);const ext=((m.file?.name||'').split('.').pop()||'').toLowerCase();if(['ttf','otf'].includes(ext))showToast('Font otomatis masuk ke Galeri Font.');if(ext==='psd')showToast('File PSD otomatis masuk ke Galeri PSD.');}
}
async function downloadMessageImage(m){
 const r=await downloadChatAttachment({url:fullUrl(m.file.downloadUrl||`/download/${encodeURIComponent(m.file.stored||'')}`),name:m.file.name||'gambar.png'});
 if(r?.ok)showToast('File disimpan: '+r.path,4200);else if(!r?.canceled)showToast(r?.error||'Download gagal');
}
function showMessageContextMenu(m,e){const menu=$('messageContextMenu');if(!menu)return;messageContextTarget=m;menu.dataset.messageId=String(m.id);menu.querySelectorAll('[data-msg-action=copy-image],[data-msg-action=download-image]').forEach(b=>b.classList.toggle('hidden',!m.file||!isImage(m.file)||!!m.deleted));menu.querySelector('[data-msg-action=copy-text]')?.classList.toggle('hidden',!m.text||!!m.deleted);menu.querySelector('[data-msg-action=edit]')?.classList.toggle('hidden',!(m.sender===cfg.name||cfg?.isHost)||!!m.deleted);menu.querySelector('[data-msg-action=delete]')?.classList.toggle('hidden',!!m.deleted);menu.querySelector('[data-msg-action=save-sticker]')?.classList.toggle('hidden',!(m.sticker&&m.file?.url&&!m.deleted));menu.classList.remove('hidden');requestAnimationFrame(()=>{const r=menu.getBoundingClientRect();const x=Math.max(8,Math.min(innerWidth-r.width-8,e.clientX));const y=Math.max(8,Math.min(innerHeight-r.height-8,e.clientY));menu.style.left=x+'px';menu.style.top=y+'px';menu.classList.add('menu-open');window.KCChatReactions?.open(m,menu,{context:true});});}
function applyMessageReceipt(p){if(String(p?.conversationId)!==String(activeConversationId)||!p?.receipt)return;const m=messageStore.find(x=>String(x.id)===String(p.id));if(!m)return;m.receipt=p.receipt||m.receipt;if(p.readBy)m.readBy=p.readBy;saveChatCache(activeConversationId,messageStore);const row=[...$('messages').querySelectorAll('.msg-row[data-message-id]')].find(x=>String(x.dataset.messageId)===String(p.id)),mark=row?.querySelector('.message-receipt');if(!mark)return;mark.innerHTML=window.KCChatReceipts?.icon(!!(m.receipt.readAt||m.receipt.deliveredAt))||(m.receipt.readAt||m.receipt.deliveredAt?'✓✓':'✓');mark.classList.toggle('delivered',!!m.receipt.deliveredAt);mark.classList.toggle('read',!!m.receipt.readAt);mark.title=m.receipt.readAt?'Dibaca':m.receipt.deliveredAt?'Terkirim ke perangkat penerima':'Tersimpan di HOST';}
function setReply(m){editingMessage=null;currentReply=m;$('replyComposerName').textContent='Reply '+(m.sender===cfg.name?'pesan kamu':profileDisplayName(m.sender));$('replyComposerText').textContent=(messageDisplayText(m)||m.file?.name||'Pesan').slice(0,180);$('replyComposer').classList.remove('hidden');$('text').focus();}
function clearReply(){currentReply=null;editingMessage=null;$('replyComposer').classList.add('hidden');}
function startEdit(m){currentReply=null;editingMessage=m;$('replyComposerName').textContent='Edit pesan';$('replyComposerText').textContent='Ubah pesan lalu tekan kirim';$('replyComposer').classList.remove('hidden');$('text').value=m.text||'';$('text').focus();}
async function deleteMessage(m){
  const choice=await messageDeleteChoice({title:'Hapus pesan?',count:1});if(!choice)return;
  if(choice==='me'){hideMessagesLocal([m.id]);showToast('Pesan dihapus untuk kamu.');return;}
  if(!cfg?.isHost)return showToast('Hanya HOST yang dapat menghapus untuk semua orang.');
  socket.emit('delete-message',{id:m.id,scope:'everyone'},r=>{if(!r?.ok)showToast(r?.error||'Gagal menghapus pesan');else showToast('Pesan dihapus untuk semua orang.');});
}
$('cancelReply').onclick=clearReply;
$('selectionCancel')?.addEventListener('click',clearMessageSelection);
$('selectionCopy')?.addEventListener('click',copySelectedMessages);
$('selectionReply')?.addEventListener('click',replySelectedMessage);
$('selectionDelete')?.addEventListener('click',deleteSelectedMessages);
function jumpToMessage(id){const el=[...document.querySelectorAll('.msg-row')].find(x=>x.dataset.messageId===id);if(!el)return false;el.scrollIntoView({behavior:'smooth',block:'center'});el.classList.add('message-flash');setTimeout(()=>el.classList.remove('message-flash'),1400);return true;}
async function focusIncomingMessage(id){id=String(id||'');if(!id)return;if(jumpToMessage(id))return;try{const r=await kcFetch(serverBase+'/history');if(!r.ok)throw new Error('history gagal');const items=await r.json();const box=$('messages');box.innerHTML='<div class="date-pill">Hari ini</div>';items.forEach(m=>renderMessage(m,false));setTimeout(()=>{if(!jumpToMessage(id))showToast('Chat terbuka, tetapi pesan lama tidak ada di history aktif.');},80);}catch(e){showToast('Chat terbuka, tetapi pesan tag tidak bisa dimuat.');}}
if(window.kc?.onFocusMessage)window.kc.onFocusMessage(async payload=>{const data=(payload&&typeof payload==='object')?payload:{messageId:payload};const cid=String(data.conversationId||'');if(cid&&cid!==String(activeConversationId||'')){openConversation(cid);setTimeout(()=>focusIncomingMessage(data.messageId),220);}else await focusIncomingMessage(data.messageId);focusChatComposer(true);});
function updateUnread(){$('unread').textContent=unread;$('unread').classList.toggle('hidden',unread===0);}
function scrollBottom(smooth=false){requestAnimationFrame(()=>{const box=$('messages');if(!box)return;try{box.scrollTo({top:box.scrollHeight,behavior:smooth?'smooth':'auto'});}catch{box.scrollTop=box.scrollHeight;}setTimeout(()=>{newMessagesWhileScrolled=0;updateScrollToBottomButton();},smooth?220:20);});}
function resetComposerHeight(){const el=$('text');if(!el)return;el.style.height='38px';el.scrollTop=0;}

let fontLibraryCache=[],psdLibraryCache=[];
async function fetchAssetLibrary(kind){if(!serverBase)return[];try{const r=await kcFetch(serverBase+'/asset-library/'+kind,{cache:'no-store'}),j=await r.json();return j?.ok&&Array.isArray(j.items)?j.items:[];}catch(e){reportBug('asset-library-load-failed',e,'warning',{kind});return[];}}
function assetRowHtml(f,kind){const down=fullUrl(f.downloadUrl||`/download/${encodeURIComponent(f.stored||'')}`),view=fullUrl(f.url||'');return `<div class="asset-row"><div class="left"><div class="asset-icon">${kind}</div><div class="asset-meta"><b>${esc(f.name||kind)}</b><span>Dikirim oleh ${esc(f.sender||'-')} · ${fmtSize(f.size||0)} · ${new Date(f.ts||Date.now()).toLocaleString('id-ID')}</span></div></div><div class="asset-actions">${view?`<a href="${esc(view)}" target="_blank">Buka</a>`:''}<a href="${esc(down)}">Unduh</a><button data-sendtext="${esc(f.name||'')}" title="Salin nama file">Salin nama</button></div></div>`;}
function bindAssetButtons(root){root?.querySelectorAll('[data-sendtext]').forEach(btn=>btn.onclick=()=>{navigator.clipboard?.writeText(btn.dataset.sendtext||'').catch(()=>{});showToast('Nama file disalin.');});}
async function renderAssetLibraries(){
  [fontLibraryCache,psdLibraryCache]=await Promise.all([fetchAssetLibrary('fonts'),fetchAssetLibrary('psds')]);
  const fontList=$('fontLibraryList'), psdList=$('psdLibraryList');
  if(fontList){const groups=new Map();for(const it of fontLibraryCache){const base=String(it.name||'Font').replace(/\.(ttf|otf)$/i,'').replace(/[ _-]*(regular|italic|bold|medium|light|semibold|thin|black|heavy|ultralight)(italic)?$/i,'').trim()||it.name;if(!groups.has(base))groups.set(base,[]);groups.get(base).push(it);}fontList.innerHTML=groups.size?[...groups].map(([name,items])=>`<details class="font-family-group"><summary><b>${esc(name)}</b><span>${items.length} style</span></summary>${items.map(it=>assetRowHtml(it,'FONT')).join('')}</details>`).join(''):'<div class="empty-state">Belum ada font TTF/OTF yang dikirim.</div>';bindAssetButtons(fontList);}
  if(psdList){psdList.innerHTML=psdLibraryCache.length?psdLibraryCache.map(it=>assetRowHtml(it,'PSD')).join(''):'<div class="empty-state">Belum ada file PSD yang dikirim.</div>';bindAssetButtons(psdList);}
}
function openSimpleModal(id){$(id)?.classList.remove('hidden');}

$('fontLibrarySidebar').onclick=async()=>{openSimpleModal('fontLibraryModal');await renderAssetLibraries();};
$('psdLibrarySidebar').onclick=async()=>{openSimpleModal('psdLibraryModal');await renderAssetLibraries();};
$('closeFontLibrary').onclick=()=>$('fontLibraryModal').classList.add('hidden');
$('closePsdLibrary').onclick=()=>$('psdLibraryModal').classList.add('hidden');
let attachmentPreviewUrls=[], attachmentPreviewIndex=0;
function addChatFiles(files){const incoming=Array.from(files||[]);if(!incoming.length)return;if(!selectedFiles.length&&selectedFile)selectedFiles=[selectedFile];selectedFiles.push(...incoming);selectedFile=selectedFiles[0];attachmentPreviewIndex=selectedFiles.length-incoming.length;showFilePreview();if(activeConversationId)$('text').focus({preventScroll:true});}
$('fileInput').onchange=e=>{addChatFiles(e.target.files);e.target.value='';};
function showFilePreview(){
  attachmentPreviewUrls.forEach(url=>URL.revokeObjectURL(url));attachmentPreviewUrls=[];
  const root=$('filePreview');const previousScroll=root.querySelector('.chat-attachment-strip')?.scrollLeft||0;if(!selectedFiles.length&&selectedFile)selectedFiles=[selectedFile];
  if(!selectedFiles.length){root.classList.add('hidden');root.innerHTML='';return;}
  selectedFile=selectedFiles[0];attachmentPreviewIndex=Math.min(attachmentPreviewIndex,selectedFiles.length-1);
  root.classList.remove('hidden');attachmentPreviewUrls=selectedFiles.map(f=>String(f.type||'').startsWith('image/')?URL.createObjectURL(f):'');
  const current=selectedFiles[attachmentPreviewIndex],url=attachmentPreviewUrls[attachmentPreviewIndex];
  root.innerHTML=`<div class="chat-attachment-head"><b>${selectedFiles.length} file siap dikirim</b><button id="cancelFile" type="button">Batal semua</button></div><div class="chat-attachment-stage">${url?`<img src="${url}" alt="Preview lampiran">`:`<span>${esc(current.name)}</span>`}</div><div class="chat-attachment-strip">${selectedFiles.map((f,i)=>`<div class="chat-attachment-thumb ${i===attachmentPreviewIndex?'active':''}"><button type="button" data-preview-file="${i}" title="${esc(f.name)}">${attachmentPreviewUrls[i]?`<img src="${attachmentPreviewUrls[i]}" alt="${esc(f.name)}">`:'📎'}</button><button type="button" data-remove-file="${i}" aria-label="Hapus ${esc(f.name)}">×</button></div>`).join('')}<button id="addPreviewFile" type="button" title="Tambah gambar / file">＋</button></div><span>Paste lagi untuk menambah gambar. Tekan Enter untuk kirim. Caption opsional.</span>`;
  root.querySelectorAll('[data-preview-file]').forEach(btn=>btn.onclick=()=>{attachmentPreviewIndex=Number(btn.dataset.previewFile);showFilePreview();});
  root.querySelectorAll('[data-remove-file]').forEach(btn=>btn.onclick=()=>{selectedFiles.splice(Number(btn.dataset.removeFile),1);selectedFile=selectedFiles[0]||null;showFilePreview();});
  root.querySelector('[id=addPreviewFile]').onclick=()=>$('fileInput').click();
  const strip=root.querySelector('.chat-attachment-strip');strip.scrollLeft=previousScroll;
  requestAnimationFrame(()=>centerAttachmentThumbnail(strip,strip.querySelector('.active')));
  bindScrollableImage(root.querySelector('.chat-attachment-stage'),root.querySelector('.chat-attachment-stage img'));
  $('cancelFile').onclick=()=>{selectedFile=null;selectedFiles=[];sendingSticker=false;$('fileInput').value='';showFilePreview();};
}
const MAX_CHAT_FILE_BYTES=2*1024*1024*1024;
const CHUNK_UPLOAD_THRESHOLD=4*1024*1024;
let uploadUiName='',uploadUiIndex=1,uploadUiCount=1;
let lastUploadPaint = 0;
function setUploadProgress(done,total,label='Mengirim file'){
  const el=$('uploading');
  if(!el)return;
  const now=performance.now();
  if(done>0&&done<total&&now-lastUploadPaint<100)return;
  lastUploadPaint=now;
  if(!el.firstElementChild)el.innerHTML='<div class="upload-progress-card"><span class="upload-spinner"></span><div class="upload-progress-main"><div class="upload-progress-top"><b></b><span></span></div><div class="upload-progress-name"></div><div class="upload-progress-track"><i></i></div><small></small></div></div>';
  const pct=total?Math.min(100,Math.round(done/total*100)):0;
  el.querySelector('b').textContent=label;
  el.querySelector('.upload-progress-top span').textContent=(uploadUiCount>1?`${uploadUiIndex}/${uploadUiCount} · `:'')+pct+'%';
  el.querySelector('.upload-progress-name').textContent=uploadUiName||'File';
  el.querySelector('i').style.width=pct+'%';
  el.querySelector('small').textContent=`${fmtSize(done)} / ${fmtSize(total)}`;
}
function yieldChatUi(){
  return window.scheduler?.yield ? window.scheduler.yield() : new Promise(resolve=>setTimeout(resolve,0));
}
async function jsonFetch(url,options={}){const r=await kcFetch(url,options);let j={};try{j=await r.json()}catch{}if(!r.ok||j.ok===false)throw Object.assign(new Error(j.error||`HTTP ${r.status}`),{status:r.status,data:j});return j;}
async function uploadLargeFile(file, uploadRequest, showProgress){
  const init=await uploadRequest('/upload/init',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({name:file.name,size:file.size,mimetype:file.type||'application/octet-stream'})});
  let offset=Number(init.offset)||0;const chunkBytes=Number(init.chunkBytes)||8*1024*1024;const id=init.id;
  try{
    while(offset<file.size){
      const end=Math.min(file.size,offset+chunkBytes),blob=file.slice(offset,end);let sent=false,lastErr;
      for(let attempt=1;attempt<=4&&!sent;attempt++){
        try{const j=await uploadRequest('/upload/chunk/'+encodeURIComponent(id),{method:'PUT',headers:{'Content-Type':'application/octet-stream','X-Upload-Offset':String(offset)},body:blob});offset=Number(j.offset);sent=true;if(showProgress)setUploadProgress(offset,file.size,'Mengirim file besar');}
        catch(err){lastErr=err;if(err?.status===409&&Number.isSafeInteger(err?.data?.expectedOffset)){offset=err.data.expectedOffset;sent=true;break;}if(attempt<4)await new Promise(r=>setTimeout(r,250*attempt));}
      }
      if(!sent)throw lastErr||new Error('Chunk gagal dikirim');
      await yieldChatUi();
    }
    if(showProgress)setUploadProgress(file.size,file.size,'Memverifikasi file');
    return await uploadRequest('/upload/finish',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id})});
  }catch(err){try{await uploadRequest('/upload/cancel',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id})})}catch{}throw err;}
}
async function waitForUploadSession(timeout=15000){const started=Date.now();while(Date.now()-started<timeout){if(socket?.connected&&sessionReady&&sessionToken&&serverBase)return true;await new Promise(r=>setTimeout(r,180));}return false;}
async function uploadFile(file = selectedFile, showProgress = true){if(!file)return null;if(file.size>MAX_CHAT_FILE_BYTES)throw new Error('Ukuran file maksimal 2 GB');if(!await waitForUploadSession()){requestFreshDiscovery('upload-session-not-ready',0);throw new Error('Koneksi baru selesai reconnect. Sesi sedang dipulihkan, coba kirim lagi sebentar.');}const origin=serverBase, token=sessionToken, scope=outboxScope();
const uploadRequest=(path,options)=>{
  const headers=new Headers(options.headers);
  headers.set('Authorization','Bearer '+token);
  return jsonFetch(origin+path,{...options,headers});
};
if(showProgress){uploadUiName=file.name||'File';$('uploading').classList.remove('hidden');setUploadProgress(0,file.size);}try{let j;if(file.size>=CHUNK_UPLOAD_THRESHOLD){j=await uploadLargeFile(file,uploadRequest,showProgress);}else{const fd=new FormData();fd.append('file',file);j=await uploadRequest('/upload',{method:'POST',body:fd});}if(outboxScope()!==scope)throw Error('HOST atau profil berubah saat upload. Kirim ulang lampiran ke chat yang benar.');if(!j.file?.stored)throw Error('HOST tidak mengembalikan lampiran yang sudah tersimpan.');if(showProgress&&j.duplicate)showToast('File sama terdeteksi — server memakai file yang sudah ada.');return j.file;}finally{if(showProgress){$('uploading').classList.add('hidden');$('uploading').innerHTML='Mengirim file…';}}}
function socketSend(payload,originalFile){return queueMessage(payload,originalFile);}
let attachmentSending = false;
async function send(event, stickerFile = null) {
  const permissions = myPolicy.permissions || {};
  if (permissions.banned || permissions.sendChat === false) return showToast('Kirim chat dibatasi HOST.');
  const conversationId = activeConversationId;
  if (!conversationId) return showToast('Pilih chat dulu.');
  const composer = $('text'), button = $('send');
  if (button.disabled) return;
  const draft = composer.value, text = stickerFile ? '' : composerMentionText(draft).trim();
  const editing = stickerFile ? null : editingMessage, reply = stickerFile && editingMessage ? null : currentReply;
  const sticker = Boolean(stickerFile) || sendingSticker;
  const files = stickerFile ? [stickerFile] : selectedFiles.length ? [...selectedFiles] : selectedFile ? [selectedFile] : [];
  if (files.length && permissions.sendFiles === false) return showToast('Kirim file dibatasi HOST.');
  if (files.length && attachmentSending) return showToast('File sebelumnya masih dikirim. Pesan teks tetap bisa dikirim.');
  if (files.length && (!socket?.connected || !sessionReady || !sessionToken)) return showToast('Tunggu HOST terhubung sebelum mengirim lampiran.');
  if (editing ? !text || !socket?.connected : !text && !files.length) return;
  button.disabled = true;
  let ownsButton = true;
  let ownsUpload = false;
  let sentFiles = 0;
  try {
    if (!editing && text.toLowerCase().includes('@everyone')) {
      const yes = await appConfirm({title:'Kirim ke @everyone?',text:'Pesan ini akan menandai semua orang.',ok:'Kirim ke Everyone'});
      if (!yes) return;
    }
    if (activeConversationId !== conversationId) return showToast('Chat berubah. Buka chat tujuan untuk mengirim.');
    emitTypingState(false);
    if (editing) {
      await new Promise((resolve,reject) => socket.emit('edit-message',{id:editing.id,text},
        result => result?.ok ? resolve(result) : reject(Error(result?.error || 'Gagal edit pesan'))));
      if (activeConversationId === conversationId && composer.value === draft && editingMessage === editing) {
        composer.value = '';
        resetComposerHeight();
        clearReply();
      }
      return;
    }
    const lower = text.toLowerCase();
    const workTag = lower.includes('@tescetak') ? 'print-test' : lower.includes('@revisidesain') ? 'design-revision' : lower.includes('@mintadesain') ? 'design-request' : null;
    const payload = {text,workTag,replyToId:reply?.id || null,sticker,conversationId};
    if (!files.length) {
      await socketSend(payload);
      if (activeConversationId === conversationId && composer.value === draft) {
        composer.value = '';
        resetComposerHeight();
        if (currentReply === reply) clearReply();
      }
      return;
    }
    attachmentSending = ownsUpload = true;
    if (!stickerFile && composer.value === draft) {
      composer.value = '';
      resetComposerHeight();
    }
    if (reply && currentReply === reply) clearReply();
    if (!stickerFile) {
      const pending = selectedFiles.filter(file => !files.includes(file));
      selectedFile = pending[0] || null;
      selectedFiles = pending;
      sendingSticker = false;
      $('fileInput').value = '';
      showFilePreview();
    }
    button.disabled = ownsButton = false;
    await yieldChatUi();
    uploadUiCount = files.length;
    const attachmentScope = outboxScope();
    for (const [index, input] of files.entries()) {
      if (outboxScope() !== attachmentScope) throw Error('HOST atau profil berubah. Lampiran berikutnya belum dikirim.');
      uploadUiIndex = index + 1;
      const file = await uploadFile(input);
      await socketSend(index === 0 ? {...payload,file} : {text:'',file,sticker,conversationId},input);
      sentFiles++;
    }
  } catch (error) {
    reportBug('send-or-upload-failed',error,'error',{fileCount:files.length});
    if (ownsUpload && activeConversationId === conversationId) {
      if (!stickerFile && !composer.value && sentFiles === 0) {
        composer.value = draft;
        resetComposerHeight();
        if (!currentReply && reply) setReply(reply);
      }
      if (!selectedFile && !selectedFiles.length) {
        addChatFiles(files.slice(sentFiles));
        sendingSticker = sticker;
      }
    }
    showToast('Gagal mengirim: ' + error.message,5000);
  } finally {
    if (ownsUpload) attachmentSending = false;
    if (ownsButton) button.disabled = false;
  }
}
$('send').onclick=send;

function mentionContext(){const el=$('text'),before=el.value.slice(0,el.selectionStart),m=before.match(/(^|\s)@([^\s@]*)$/);if(!m)return null;return{query:(m[2]||'').toLowerCase(),start:el.selectionStart-(m[2]||'').length-1,end:el.selectionStart};}
function conversationMentionPeople(conv=activeConversation()){
  if(!conv)return[];
  const names=conv.type==='dm'?(conv.members||[]).filter(n=>profileKeyLocal(n)!==profileKeyLocal(cfg.name)):(conv.type==='group'?(conv.members||[]).filter(n=>profileKeyLocal(n)!==profileKeyLocal(cfg.name)):(onlineUsers||[]).map(u=>u.name).filter(n=>profileKeyLocal(n)!==profileKeyLocal(cfg.name)));
  const pool=[...(onlineUsers||[]),...(editorDirectory||[])],by=new Map();for(const u of pool){const k=profileKeyLocal(u.name);if(k&&!by.has(k))by.set(k,u);}
  return names.map(name=>by.get(profileKeyLocal(name))||{name,pc:conv.type==='dm'?'Tag pribadi':'Anggota grup'}).filter(Boolean);
}
function mentionChoices(){const c=mentionContext();if(!c)return[];const muted=(myPolicy.mutedUntil||0)>Date.now(),conv=activeConversation(),isPrivate=conv?.type==='dm';const specials=[{name:'tescetak',pc:(isPrivate?'Tag pribadi':'Tag grup')+' · Tes Cetak',workTag:'print-test'},{name:'revisidesain',pc:(isPrivate?'Tag pribadi':'Tag grup')+' · Revisi Desain',workTag:'design-revision'},{name:'mintadesain',pc:(isPrivate?'Tag pribadi':'Tag grup')+' · Minta Desain',workTag:'design-request'}];if(!isPrivate&&!muted&&(cfg.isHost||myPolicy.permissions?.everyone))specials.push({name:'everyone',pc:'Tag grup · semua anggota'});if(!isPrivate&&!muted&&(cfg.isHost||myPolicy.permissions?.editor))specials.push({name:'editor',pc:'Tag grup · semua editor di grup'});let people=[];if(!muted&&(cfg.isHost||myPolicy.permissions?.personal))people=conversationMentionPeople(conv);const seen=new Set();return[...specials,...people].filter(u=>{const n=String(u.name||'').trim();if(!n)return false;const k=n.toLowerCase();if(seen.has(k))return false;seen.add(k);return!c.query||k.includes(c.query)||(!specials.includes(u)&&profileKeyLocal(profileDisplayName(n)).includes(c.query));}).slice(0,20);}
function mentionPickerSubtitle(user, choices){
  if(user.workTag||['everyone','editor'].includes(user.name))return user.pc||'';
  const display=profileKeyLocal(profileDisplayName(user.name));
  const pc=profileKeyLocal(user.pc);
  const needsAccount=choices.some(other=>other!==user&&profileKeyLocal(profileDisplayName(other.name))===display
    &&(!pc||profileKeyLocal(other.pc)===pc));
  return [user.pc,needsAccount?'Akun '+user.name:''].filter(Boolean).join(' · ')||'';
}
function updateMentionPicker(){const picker=$('mentionPicker');if(!picker)return;const choices=mentionChoices();if(!choices.length){picker.classList.add('hidden');picker.innerHTML='';picker._renderHtml='';mentionIndex=0;return;}mentionIndex=Math.max(0,Math.min(mentionIndex,choices.length-1));const pickerHtml=choices.map((u,i)=>`<button type="button" class="mention-option ${i===mentionIndex?'active':''}" data-i="${i}">${currentProfileAvatar(u.name,u.avatarUrl)?avatarHtml(currentProfileAvatar(u.name,u.avatarUrl),u.name,'mention-avatar'):`<span class="mention-avatar">${esc((u.name||'?')[0].toUpperCase())}</span>`}<span><b>@${esc(profileDisplayName(u.name))}</b><small>${esc(mentionPickerSubtitle(u,choices))}</small></span></button>`).join('');if(picker._renderHtml!==pickerHtml){picker.innerHTML=pickerHtml;picker._renderHtml=pickerHtml;}picker.classList.remove('hidden');picker.querySelectorAll('.mention-option').forEach(btn=>btn.onmousedown=e=>{e.preventDefault();pickMention(Number(btn.dataset.i));});}
let composerMentions=[];
let mentionDraft='';
let mentionConversation;
function syncComposerMentions(value){
  if(mentionConversation!==activeConversationId){composerMentions=[];mentionConversation=activeConversationId;}
  if(value===mentionDraft)return;
  let start=0,end=0;
  while(start<Math.min(value.length,mentionDraft.length)&&value[start]===mentionDraft[start])start++;
  while(end<Math.min(value.length,mentionDraft.length)-start&&value[value.length-1-end]===mentionDraft[mentionDraft.length-1-end])end++;
  const oldEnd=mentionDraft.length-end,delta=value.length-mentionDraft.length;
  composerMentions=composerMentions.filter(item=>item.end<=start||item.start>=oldEnd).map(item=>
    item.end<=start?item:{...item,start:item.start+delta,end:item.end+delta});
  mentionDraft=value;
}
function composerMentionText(value){
  syncComposerMentions(value);
  for(const item of [...composerMentions].reverse()){
    if(!/[\p{L}\p{N}_]/u.test(value.slice(item.end,item.end+1)))
      value=value.slice(0,item.start)+'@'+item.name+value.slice(item.end);
  }
  return value;
}
function refreshComposerMentions(key){
  const el=$('text');if(!el)return;
  syncComposerMentions(el.value);
  let start=el.selectionStart,end=el.selectionEnd;
  for(const item of [...composerMentions].reverse()){
    if(profileKeyLocal(item.name)!==key)continue;
    const text='@'+profileDisplayName(item.name),delta=text.length-(item.end-item.start);
    if(!delta&&el.value.slice(item.start,item.end)===text)continue;
    el.value=el.value.slice(0,item.start)+text+el.value.slice(item.end);
    const move=pos=>pos>=item.end?pos+delta:pos>item.start?item.start+text.length:pos;
    start=move(start);end=move(end);
    for(const next of composerMentions)if(next.start>item.start){next.start+=delta;next.end+=delta;}
    item.end=item.start+text.length;
  }
  mentionDraft=el.value;
  el.setSelectionRange(start,end);
}
function pickMention(i){
  const el=$('text'),c=mentionContext(),user=mentionChoices()[i];if(!c||!user)return;
  syncComposerMentions(el.value);
  const special=user.workTag||['everyone','editor'].includes(user.name);
  const label='@'+(special?user.name:profileDisplayName(user.name));
  el.value=el.value.slice(0,c.start)+label+' '+el.value.slice(c.end);
  syncComposerMentions(el.value);
  composerMentions=composerMentions.filter(item=>item.end<=c.start||item.start>=c.start+label.length+1);
  if(!special)composerMentions.push({name:user.name,start:c.start,end:c.start+label.length});
  composerMentions.sort((a,b)=>a.start-b.start);
  const pos=c.start+label.length+1;
  el.focus();el.setSelectionRange(pos,pos);$('mentionPicker').classList.add('hidden');mentionIndex=0;
}
$('text')?.addEventListener('input',event=>syncComposerMentions(event.target.value));

document.addEventListener('keydown', event => {
  if (event.key !== 'Enter' || event.defaultPrevented || event.repeat || event.isComposing ||
      event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) return;
  if (!activeConversationId || !selectedFile && !selectedFiles.length || $('filePreview').classList.contains('hidden')) return;
  if (document.querySelector('.modal:not(.hidden), dialog[open]')) return;
  if (event.target.closest?.('input,textarea,button,a,select,[contenteditable],[role="button"],[role="menuitem"]')) return;
  event.preventDefault();
  void send();
});
$('text').onkeydown=e=>{if(e.isComposing||e.keyCode===229)return;const picker=$('mentionPicker'),open=picker&&!picker.classList.contains('hidden'),choices=mentionChoices();if(open&&e.key==='ArrowDown'){e.preventDefault();mentionIndex=(mentionIndex+1)%choices.length;updateMentionPicker();return;}if(open&&e.key==='ArrowUp'){e.preventDefault();mentionIndex=(mentionIndex-1+choices.length)%choices.length;updateMentionPicker();return;}if(open&&(e.key==='Tab'||e.key==='Enter')){e.preventDefault();pickMention(mentionIndex);return;}if(open&&e.key==='Escape'){e.preventDefault();picker.classList.add('hidden');return;}if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();send();}};
$('text').addEventListener('input',e=>{e.target.style.height='38px';e.target.style.height=Math.min(e.target.scrollHeight,140)+'px';mentionIndex=0;updateMentionPicker();if(String(e.target.value||'').trim()){emitTypingState(true);scheduleTypingStop();}else emitTypingState(false);});
$('text').addEventListener('click',updateMentionPicker);$('text').addEventListener('blur',()=>{emitTypingState(false);setTimeout(()=>$('mentionPicker')?.classList.add('hidden'),150);});
let clipboardImageBase64='',clipboardResultBase64='';
async function readClipboardImage(){const r=await window.kc.getClipboardImage();if(!r?.ok){$('clipboardStatus').textContent=r?.error||'Clipboard tidak berisi gambar.';return null;}clipboardImageBase64=r.base64;clipboardResultBase64=r.base64;$('clipboardPreview').src=r.dataUrl;$('clipboardStatus').textContent=`Gambar clipboard ${r.width}×${r.height}px siap.`;$('clipboardModal').querySelectorAll('[data-clip-tool]').forEach(b=>b.disabled=false);$('copyClipboardResult').disabled=false;return r;}
function clipboardFile(){if(!clipboardImageBase64)return null;const bin=atob(clipboardImageBase64),arr=new Uint8Array(bin.length);for(let i=0;i<bin.length;i++)arr[i]=bin.charCodeAt(i);return new File([arr],`clipboard-${Date.now()}.png`,{type:'image/png'});}
if($('clipboardToolsAction'))$('clipboardToolsAction').onclick=async()=>{$('attachMenu').classList.add('hidden');$('clipboardModal').classList.remove('hidden');await readClipboardImage();};
if($('closeClipboard'))$('closeClipboard').onclick=()=>$('clipboardModal').classList.add('hidden');if($('readClipboard'))$('readClipboard').onclick=readClipboardImage;
$('clipboardModal')?.querySelectorAll('[data-clip-tool]').forEach(b=>b.onclick=async()=>{const f=clipboardFile();if(!f)return showToast('Ambil gambar clipboard dulu.');const tool=b.dataset.clipTool;$('clipboardModal').classList.add('hidden');if(tool==='removebg'){removeBgFiles=[f];removeBgFile=f;await rbLoadFile(f);$('removeBgModal').classList.remove('hidden');}else{const clipDir=await window.kc.chooseVectorOutput();if(!clipDir)return showToast('Pilih folder sementara untuk Clipboard Tools.');const tmp=await window.kc.writeBase64File({path:clipDir.replace(/[\\/]$/,'')+`/KC-clipboard-${Date.now()}.png`,base64:clipboardImageBase64});if(!tmp?.ok)return showToast(tmp?.error||'Gagal membuat file clipboard');if(tool==='upscale'){upscaleInputPaths=[tmp.path];upscaleInputPath=tmp.path;const pv=await window.kc.readLocalImagePreview(tmp.path);$('upscalePreview').src=pv?.dataUrl||'';$('upscaleModal').classList.remove('hidden');}if(tool==='vector'){vectorInputPaths=[tmp.path];vectorInputPath=tmp.path;const pv=await window.kc.readLocalImagePreview(tmp.path);$('vectorInputPreview').src=pv?.dataUrl||'';$('vectorModal').classList.remove('hidden');}if(tool==='head'){headCropFiles=[tmp.path];headCropFileIndex=0;await loadHeadCropImage(tmp.path);$('headCropModal').classList.remove('hidden');}}});
if($('copyClipboardResult'))$('copyClipboardResult').onclick=async()=>{const r=await window.kc.setClipboardPng(clipboardResultBase64||clipboardImageBase64);showToast(r?.ok?'PNG disalin ke clipboard.':(r?.error||'Copy clipboard gagal'));};
window.addEventListener('dragover',e=>e.preventDefault());window.addEventListener('drop',e=>{e.preventDefault();const files=[...(e.dataTransfer.files||[])];if(files.length){addChatFiles(files);}});window.addEventListener('paste',e=>{const files=[...(e.clipboardData?.files||[])];if(files.length){e.preventDefault();addChatFiles(files);}});window.addEventListener('focus',()=>{unread=0;updateUnread();});

$('themeButton').onclick=()=>{const current=document.documentElement.dataset.theme==='light'?'light':'dark';applyTheme(current==='light'?'dark':'light',true);};

async function loadMyProfile(){try{const r=await kcFetch(serverBase+'/profile/'+encodeURIComponent(cfg.name)),j=await r.json();myProfile=j.profile;setAvatar($('myAvatarButton'),myProfile?.avatarUrl,cfg.name);setAvatar($('railProfile'),myProfile?.avatarUrl,cfg.name);renderProfilePreview();renderDiscordProfileBar();}catch{setAvatar($('railProfile'),null,cfg?.name||'Editor');renderDiscordProfileBar();}}
function renderProfilePreview(){refreshProfileNameUI();const el=$('profilePreview');el.dataset.profilePhoto=myProfile?.avatarUrl||'';el.dataset.profileName=cfg.name;if(myProfile?.avatarUrl){el.classList.add('has-photo');el.style.backgroundImage=`url("${fullUrl(myProfile.avatarUrl)}")`;el.querySelector('span').textContent='';}else{el.classList.remove('has-photo');el.style.backgroundImage='';el.querySelector('span').textContent=cfg.name[0].toUpperCase();}}
$('myAvatarButton')?.addEventListener('click',()=>{renderProfilePreview();renderMyActivityPreview();$('profileModal').classList.remove('hidden');});$('closeProfile')?.addEventListener('click',()=>$('profileModal').classList.add('hidden'));$('chooseAvatar')?.addEventListener('click',()=>$('avatarInput').click());$('shareActivity')?.addEventListener('change',e=>setActivityShareEnabled(!!e.target.checked));
$('teamRoleSelect')?.addEventListener('change',async e=>{cfg.teamRole=normalizeTeamRole(e.target.value);await window.kc.saveConfig({teamRole:cfg.teamRole});hello();renderMyActivityPreview();requestConversationList();showToast('Role kerja disimpan: '+cfg.teamRole);});
[['workTagPrintTest','print-test'],['workTagDesignRevision','design-revision'],['workTagDesignRequest','design-request']].forEach(([id,k])=>$(id)?.addEventListener('change',async()=>{const prefs=myWorkTagPrefs();prefs[k]=!!$(id).checked;cfg.workTagPrefs=prefs;await window.kc.saveConfig({workTagPrefs:prefs});hello();showToast('Pilihan tag pekerjaan disimpan.');}));
$('avatarInput').onchange=async e=>{const f=e.target.files?.[0];e.target.value='';if(!f)return;try{const cropped=await window.KCPhotoEditor.crop(f,true);if(!cropped)return;const fd=new FormData();fd.append('avatar',cropped,'profil.png');const r=await kcFetch(serverBase+'/profile/'+encodeURIComponent(cfg.name)+'/avatar',{method:'POST',body:fd}),j=await r.json();if(!r.ok||!j.ok)throw Error(j.error||'Gagal menyimpan foto profil');myProfile=j.profile;setAvatar($('myAvatarButton'),myProfile.avatarUrl,cfg.name);setAvatar($('railProfile'),myProfile.avatarUrl,cfg.name);renderProfilePreview();renderDiscordProfileBar();hello();showToast('Foto profil diperbarui.');}catch(err){reportBug('profile-photo-failed',err,'warning',{});showToast(err.message||'Foto gagal diproses.');}};

let pendingStorageRoot='';
async function hostAdminRequest(route,options={}){if(!cfg?.isHost){if(route.startsWith('/admin/bledar-videos')&&myPolicy?.cohost?.manageVideos)return kcFetch(serverBase+route,{...options,headers:{...options.headers,Authorization:'Bearer '+sessionToken}});throw new Error('Izin pengelolaan belum diberikan HOST');}const primary=await window.kc.startHostPrimary();if(!primary?.ok||!primary.address)throw new Error(primary?.error||'Listener HOST lokal belum siap');const headers=new Headers(options.headers||{});if(!headers.has('Authorization'))headers.set('Authorization','Bearer '+(cfg.adminToken||''));return kcFetch(primary.address+route,{...options,headers});}
async function hostStorageRequest(route,options={}){return hostAdminRequest(route,options);}
async function hostStorageJson(route,options={}){const r=await hostStorageRequest(route,options);let j={};try{j=await r.json()}catch{}if(!r.ok||j?.ok===false)throw Object.assign(new Error(j?.error||`HTTP ${r.status}`),{status:r.status,data:j});return j;}
async function loadStorageLocation(){if(!cfg?.isHost)return;try{const j=await hostStorageJson('/storage/location',{cache:'no-store'});$('storageActivePath').textContent=j.storageRoot||j.uploadDir||'-';$('storageActivePath').title=j.storageRoot||j.uploadDir||'';$('storageLocationBadge').textContent=j.isDefault?'Default AppData':'Custom Storage';$('storageDriveInfo').textContent=(j.freeBytes!=null?`Sisa drive ${fmtSize(j.freeBytes)} · `:'')+`uploads: ${j.uploadDir}`;$('storageLocationSection').classList.toggle('migrating',!!j.migrating);if(j.migrating)$('storageMigrationStatus').textContent='Storage sedang dipindahkan...';}catch(e){$('storageMigrationStatus').textContent='Gagal membaca storage HOST lokal: '+e.message;}}
$('chooseStorageLocation').onclick=async()=>{if(!cfg?.isHost)return;const p=await window.kc.chooseStorageFolder();if(!p)return;pendingStorageRoot=p;$('storageMigrationStatus').textContent='Tujuan baru: '+p+'\\uploads';$('migrateStorage').disabled=false;};
$('migrateStorage').onclick=async()=>{if(!cfg?.isHost||!pendingStorageRoot)return;const cleanup=$('cleanupOldStorage').checked;const yes=await appConfirm({title:'Pindahkan Shared Storage?',text:`File attachment akan disalin ke ${pendingStorageRoot}\\uploads lalu diverifikasi checksum. ${cleanup?'Setelah valid, file lama akan dihapus.':'File lama TIDAK dihapus dulu.'} Selama migrasi upload baru ditahan sementara.`,ok:'Mulai Pindah'});if(!yes)return;$('migrateStorage').disabled=true;$('chooseStorageLocation').disabled=true;$('storageLocationSection').classList.add('migrating');$('storageMigrationStatus').textContent='Menyalin dan memverifikasi file...';try{const j=await hostStorageJson('/admin/storage/migrate',{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+(cfg.adminToken||'')},body:JSON.stringify({targetRoot:pendingStorageRoot,cleanupSource:cleanup})});$('storageMigrationStatus').textContent=`Selesai. ${j.copied||0} file diverifikasi${j.deleted?' · '+j.deleted+' file lama dihapus':''}${j.warnings?.length?' · ada '+j.warnings.length+' peringatan':''}.`;pendingStorageRoot='';showToast('Shared Storage HOST berhasil dipindahkan.');await Promise.all([loadStorageLocation(),loadStorage()]);}catch(e){reportBug('storage-migration-failed',e,'error',{targetRoot:pendingStorageRoot});$('storageMigrationStatus').textContent='Migrasi gagal: '+e.message+'. Storage lama tetap aktif.';showToast('Pindah storage gagal. Lokasi lama tetap aman.');}finally{$('chooseStorageLocation').disabled=false;$('storageLocationSection').classList.remove('migrating');$('migrateStorage').disabled=!pendingStorageRoot;}};
$('storageButton').onclick=async()=>{$('storageModal').classList.remove('hidden');await Promise.all([loadStorage(),loadStorageLocation()]);};$('closeStorage').onclick=()=>$('storageModal').classList.add('hidden');$('refreshStorage').onclick=loadStorage;
$('storageFileSearch').oninput=renderStorageFiles;
$('selectAllStorage').onclick=()=>{const rows=storageFilteredFiles();const all=rows.length&&rows.every(f=>selectedStorageFiles.has(f.stored));rows.forEach(f=>all?selectedStorageFiles.delete(f.stored):selectedStorageFiles.add(f.stored));renderStorageFiles();};
$('deleteSelectedStorage').onclick=async()=>{const keys=[...selectedStorageFiles];if(!keys.length)return;if(!await appConfirm({title:'Hapus file terpilih?',text:`${keys.length} file akan dihapus dari storage HOST. Attachment yang masih dipakai chat akan dilepas dari pesannya.`,ok:'Hapus Terpilih'}))return;for(const key of keys){try{await hostStorageJson('/storage/file/'+encodeURIComponent(key),{method:'DELETE'});}catch(e){showToast('Gagal menghapus '+key+': '+e.message);}}selectedStorageFiles.clear();await loadStorage();};
let storageFiles=[],selectedStorageFiles=new Set();
function storageFilteredFiles(){const q=String($('storageFileSearch')?.value||'').trim().toLowerCase();return storageFiles.filter(f=>!q||[f.name,f.stored,f.mimetype].filter(Boolean).join(' ').toLowerCase().includes(q));}
function updateStorageSelected(){const n=selectedStorageFiles.size,total=storageFiles.filter(f=>selectedStorageFiles.has(f.stored)).reduce((s,f)=>s+Number(f.size||0),0);if($('storageSelectedInfo'))$('storageSelectedInfo').textContent=`${n} file dipilih${n?' · '+fmtSize(total):''}`;if($('deleteSelectedStorage'))$('deleteSelectedStorage').disabled=!n;}
function renderStorageFiles(){const rows=storageFilteredFiles();$('largestFiles').innerHTML=rows.length?rows.map(f=>{const ref=f.messageRefs?.[0];return `<div class="largest-row storage-file-row" data-stored="${esc(f.stored)}"><label class="storage-check"><input type="checkbox" data-storage-check="${esc(f.stored)}" ${selectedStorageFiles.has(f.stored)?'checked':''}><span></span></label><div class="storage-file-meta"><b title="${esc(f.name||f.stored)}">${esc(f.name||f.stored)}</b><span>${f.referenced?'Dipakai chat · '+(f.messageRefs?.length||1)+' pesan':'Tidak terpakai'}${f.mimetype?' · '+esc(f.mimetype):''}</span></div><strong>${fmtSize(f.size)}</strong><div class="storage-row-actions">${ref?`<button class="secondary goto-storage-message" data-message="${esc(ref.id)}">Go to Message</button>`:''}<button class="danger delete-storage-file" data-delete-storage="${esc(f.stored)}">Hapus</button></div></div>`;}).join(''):'<div class="empty-state">File tidak ditemukan.</div>';$('largestFiles').querySelectorAll('[data-storage-check]').forEach(cb=>cb.onchange=()=>{cb.checked?selectedStorageFiles.add(cb.dataset.storageCheck):selectedStorageFiles.delete(cb.dataset.storageCheck);updateStorageSelected();});$('largestFiles').querySelectorAll('[data-delete-storage]').forEach(btn=>btn.onclick=()=>deleteStorageFile(btn.dataset.deleteStorage));$('largestFiles').querySelectorAll('.goto-storage-message').forEach(btn=>btn.onclick=()=>goToStorageMessage(btn.dataset.message));updateStorageSelected();}
async function goToStorageMessage(id){$('storageModal').classList.add('hidden');if(jumpToMessage(id)!==false)return;try{const items=await hostStorageJson('/history');const box=$('messages');box.innerHTML='<div class="date-pill">Hari ini</div>';items.forEach(m=>renderMessage(m,false));setTimeout(()=>jumpToMessage(id),50);}catch{showToast('Pesan tidak ditemukan di history aktif.');}}
async function deleteStorageFile(stored){const f=storageFiles.find(x=>x.stored===stored);if(!f)return;const msg=f.referenced?`File ini dipakai ${f.messageRefs?.length||1} pesan. Attachment akan dilepas dari pesan, tapi teks chat tetap ada.`:'File ini tidak dipakai chat dan akan dihapus permanen.';if(!await appConfirm({title:'Hapus file storage?',text:msg,ok:'Hapus File'}))return;try{const j=await hostStorageJson('/storage/file/'+encodeURIComponent(stored),{method:'DELETE'});selectedStorageFiles.delete(stored);showToast(`File dihapus · ${fmtSize(j.freed||0)} dibebaskan${j.detached?' · '+j.detached+' attachment dilepas':''}.`);await loadStorage();}catch(e){showToast('Gagal menghapus: '+e.message);}}
async function loadStorage(){$('storageSummary').innerHTML='<div class="storage-loading">Menghitung storage…</div>';try{const j=await hostStorageJson('/storage',{cache:'no-store'}),s=j.storage;storageFiles=s.largestFiles||[];for(const key of [...selectedStorageFiles])if(!storageFiles.some(f=>f.stored===key))selectedStorageFiles.delete(key);$('storageSummary').innerHTML=`<div class="stat-card"><span>Total file chat</span><b>${fmtSize(s.uploadBytes)}</b><small>${s.uniqueFiles} file unik</small></div><div class="stat-card"><span>Hemat dedup</span><b>${fmtSize(s.bytesSaved)}</b><small>${s.duplicatesAvoided} duplikat tidak disimpan</small></div><div class="stat-card"><span>Chat tersimpan</span><b>${s.messageCount}</b><small>seluruh riwayat database</small></div><div class="stat-card"><span>File orphan</span><b>${s.orphanFiles}</b><small>${fmtSize(s.orphanBytes)} bisa dibersihkan</small></div>`;renderStorageFiles();}catch{$('storageSummary').innerHTML='<div class="storage-loading">Gagal membaca storage.</div>';}}
$('cleanupStorage').onclick=async()=>{try{const preview=await hostStorageJson('/storage/cleanup/preview',{cache:'no-store'});if(!preview.items.length)return showToast('Tidak ada lampiran yatim yang memenuhi syarat.');if(!await appConfirm({title:'Pindahkan ke karantina?',text:`${preview.items.length} file tanpa referensi, total ${fmtSize(preview.items.reduce((n,f)=>n+f.size,0))}. File dapat dipulihkan dari System → Pulihkan lampiran.`,ok:'Karantina'}))return;const j=await hostStorageJson('/storage/cleanup',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({confirmed:true,stored:preview.items.map(f=>f.stored)})});showToast(j.removed+' file dipindahkan ke karantina.');loadStorage();}catch(e){showToast(e.message)}};
$('openDataFolder').onclick=async()=>{const r=await window.kc.openServerData();if(!r.ok)alert(r.error+'\n\nBuka tombol ini dari PC SERVER.');};

// CLIENT server picker: LAN health scan + explicit, persistent selection.
async function scanLanServers(){
  const status=$('serverPickerStatus'),list=$('serverPickerList');if(status)status.textContent='Memindai server ChatX yang aktif di LAN…';if(list)list.innerHTML='<div class="empty-state">Memindai jaringan kantor…</div>';
  try{
    const rows=await window.kc.discoverServers();serverPickerRows=Array.isArray(rows)?rows:[];
    // Remember the selected PC by hostname as well as address so DHCP changes
    // can be reconciled the next time the client scans the LAN.
    const reconciled=window.KCServerPicker?.reconcileManualSelection({manualAddress:cfg?.manualServerAddress,manualPcName:cfg?.manualServerPcName,servers:serverPickerRows});
    if(reconciled?.changed){cfg.manualServerAddress=reconciled.address;await window.kc.saveConfig({manualServerAddress:reconciled.address});}
    if(status)status.textContent=serverPickerRows.length?`${serverPickerRows.length} server terdeteksi di LAN`:'Tidak menemukan server ChatX aktif di LAN.';
    renderLanServerRows();
  }catch(error){if(status)status.textContent='Pencarian server gagal: '+(error?.message||String(error));if(list)list.innerHTML='<div class="empty-state">Periksa koneksi LAN, lalu coba lagi.</div>';}
}
let serverPickerRows=[];
function renderLanServerRows(){
  const list=$('serverPickerList');if(!list)return;
  if(!serverPickerRows.length){list.innerHTML='<div class="empty-state">Tidak ada server aktif yang ditemukan.</div>';return;}
  list.innerHTML=serverPickerRows.map(row=>{const name=String(row.pcName||row.ip||'PC tidak dikenal'),kind='HOST UTAMA',current=window.KCServerPicker?.sameServer(row.address,serverBase),manual=window.KCServerPicker?.sameServer(row.address,cfg?.manualServerAddress),badge=current?'TERHUBUNG':manual?'PILIHAN CLIENT':'HOST UTAMA';return `<button type="button" class="server-choice ${current?'current':''}" data-server-address="${esc(row.address)}" data-server-name="${esc(name)}"><span class="server-choice-main"><b>${esc(name)}</b><small>${kind} · ${esc(row.address)} · v${esc(row.version||'?')}</small></span><span class="server-choice-badge ${row.temporary?'temporary':''}">${badge}</span></button>`;}).join('');
  list.querySelectorAll('[data-server-address]').forEach(button=>button.onclick=()=>selectLanServer(button.dataset.serverAddress,button.dataset.serverName));
  const active=serverPickerRows.find(row=>window.KCServerPicker?.sameServer(row.address,serverBase));
  if($('serverPickerCurrent'))$('serverPickerCurrent').textContent=active?`Koneksi sekarang: ${active.pcName} · HOST utama · ${active.address}`:serverBase?`Koneksi sekarang: ${serverBase}${sessionReady?' · tersambung':' · belum tersambung'}`:'Belum tersambung ke server.';
}
async function selectLanServer(address,pcName){
  const target=window.KCServerPicker?.chooseTarget({manualAddress:address});if(!target)return showToast('Alamat server tidak valid. Refresh daftar server.');
  if(window.KCServerPicker.sameServer(target,serverBase)&&sessionReady){$('serverPickerModal').classList.add('hidden');return showToast(`Client sudah tersambung ke ${pcName}.`);}
  const yes=await appConfirm({title:'Pindah koneksi ke HOST PC ini?',text:`Pilih ${pcName} sebagai server client. Isi chat bisa berbeda bila PC ini bukan HOST utama. Data di server lama tidak dihapus.`,ok:'Pindah Server'});if(!yes)return;
  cfg.manualServerAddress=target;cfg.manualServerPcName=String(pcName||'').slice(0,80);await window.kc.saveConfig({manualServerAddress:target,manualServerPcName:cfg.manualServerPcName});$('serverPickerModal').classList.add('hidden');showToast(`Mencoba tersambung ke ${pcName}…`);requestFreshDiscovery('manual-server-selection',250);
}
$('railServerPicker')?.addEventListener('click',()=>{$('serverPickerModal').classList.remove('hidden');renderLanServerRows();scanLanServers();});
$('closeServerPicker')?.addEventListener('click',()=>$('serverPickerModal').classList.add('hidden'));
$('refreshServerList')?.addEventListener('click',scanLanServers);
$('autoServerSelection')?.addEventListener('click',async()=>{await window.kc.saveConfig({manualServerAddress:'',manualServerPcName:''});cfg.manualServerAddress='';cfg.manualServerPcName='';$('serverPickerModal').classList.add('hidden');showToast('Pemilihan server manual dihapus. Client kembali mencari HOST utama otomatis.');requestFreshDiscovery('manual-server-cleared',250);});
// Legacy allowTemporaryServer config is ignored in v2.2.13.

// Notification settings are local per PC.
function notifSettings(){return{...notifDefaults,...(cfg.notificationSettings||{})};}
$('notificationButton').onclick=()=>{const n=notifSettings();$('normalNotifSize').value=n.normalSize;$('normalNotifSound').value=n.normalSound;$('notifPosition').value=n.position;$('notifStyle').value=n.style||'card';$('notificationModal').classList.remove('hidden');};
$('closeNotificationSettings').onclick=()=>$('notificationModal').classList.add('hidden');
$('saveNotificationSettings').onclick=async()=>{const notificationSettings={normalSize:$('normalNotifSize').value,normalSound:$('normalNotifSound').value,position:$('notifPosition').value,style:$('notifStyle').value};await window.kc.saveConfig({notificationSettings});cfg.notificationSettings=notificationSettings;$('notificationModal').classList.add('hidden');showToast('Pengaturan notifikasi disimpan untuk PC ini.');};

// Host-only moderation. Admin token only exists in the renderer of the server PC.
function adminHeaders(){return{'Content-Type':'application/json','Authorization':'Bearer '+(cfg.isHost?cfg.adminToken:sessionToken)};}
function showModeration() {
  const dialog = $('adminModal');
  if (dialog.open) return;
  dialog.classList.remove('hidden');
  dialog.show();
  $('closeAdmin').focus({preventScroll:true});
}
$('adminButton').onclick = () => {
  showModeration();
};
function closeModeration() {
  $('adminModal').classList.add('hidden');
  $('adminModal').close();
  moderationController?.abort();
}
$('closeAdmin').onclick = closeModeration;
$('adminModal').addEventListener('cancel', event => {
  event.preventDefault();
  closeModeration();
});
$('refreshAdmin').onclick = () => loadAdmin(true);
$('loadModeration').onclick = () => loadAdmin(true);
$('updateManagerButton').onclick=()=>{
  $('hostQuickMenu')?.classList.add('hidden');
  $('clientUpdateModal').classList.remove('hidden');
  $('chooseClientUpdate').focus({preventScroll:true});
  $('publishedUpdateStatus').textContent='Memeriksa status…';
  $('rollbackVersion').replaceChildren(new Option('Memuat riwayat update…',''));
  void loadClientUpdateStatus().catch(error=>{$('publishedUpdateStatus').textContent=error.message;});
  void loadRollbackHistory();
};
$('closeClientUpdate').onclick=()=>$('clientUpdateModal').classList.add('hidden');
let moderationLoad = null;
let moderationSnapshotRequest;
let moderationController;
let moderationLoadedAt = 0;
let moderationInvalidation = 0;
async function loadAdmin(force = false) {
  if (!cfg?.isHost) return;
  if (!$('adminModal').open) return;
  if (moderationLoad) return moderationLoad;
  if (force) { moderationInvalidation++; moderationLoadedAt = 0; }
  if (moderationLoadedAt && Date.now() - moderationLoadedAt < 15000) return;
  const users = $('adminUsers');
  const revision = moderationInvalidation;
  const status = $('moderationStatus');
  const started = performance.now();
  const controller = new AbortController();
  moderationController = controller;
  status.textContent = 'Memuat pengaturan dan daftar anggota…';
  status.classList.remove('hidden');
  const loadButton = $('loadModeration');
  loadButton.disabled = true;
  moderationLoad = (async () => {
    try {
      const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]);
      let data;
      if (window.kc.loadHostModeration) {
        moderationSnapshotRequest ??= window.kc.loadHostModeration()
          .finally(() => { moderationSnapshotRequest = undefined; });
        let abort;
        try {
          data = await Promise.race([moderationSnapshotRequest, new Promise((_, reject) => {
            abort = () => reject(signal.reason);
            signal.addEventListener('abort', abort, { once: true });
          })]);
        } finally {
          signal.removeEventListener('abort', abort);
        }
      } else {
        const response = await hostAdminRequest('/admin/moderation', { headers: adminHeaders(), cache: 'no-store', signal });
        data = await response.json();
        if (!response.ok || !data.ok) throw Error(data.error || 'Moderation belum bisa dimuat.');
      }
      if (controller.signal.aborted || revision !== moderationInvalidation) return;
      techLog('moderation-data-received', { users: data.users?.length, roles: data.roles?.length, elapsedMs: Math.round(performance.now() - started) });
      await renderAdmin(data);
      $('adminModal').classList.add('moderation-ready');
      status.classList.add('hidden');
      techLog('moderation-rendered');
    } catch (error) {
      if (controller.signal.aborted || revision !== moderationInvalidation) return;
      const message = error.name === 'TimeoutError' ? 'HOST terlalu lama merespons. Klik Muat pengaturan & anggota untuk coba lagi.' : error.message || String(error);
      techLog('moderation-load-failed', { reason: message, elapsedMs: Math.round(performance.now() - started) });
      status.textContent = message;
      if (users.querySelector('.admin-user')) showToast(message);
      else users.textContent = message;
    }
  })();
  try {
    await moderationLoad;
  } finally {
    loadButton.disabled = false;
    moderationLoad = null;
    moderationController = undefined;
  }
}
function fmtMute(until){if(!until||until<=Date.now())return'Tidak diban';return'Diban sampai '+new Date(until).toLocaleString('id-ID');}
function moderationRoleLabel(p={}){return p.moderationRole==='DEVELOPER'?'Developer':p.moderationRole==='ADMIN'?'Admin':p.moderationRole==='CUSTOM'?(p.customRoleName||'Custom'):'Editor';}
const featureLabels={sendChat:'Kirim chat',sendFiles:'Kirim file',downloadFiles:'Unduh file',designAI:'Alat desain / AI',tagPrintTest:'Tag Tes Cetak',tagDesignRevision:'Tag Revisi Desain',tagDesignRequest:'Tag Minta Desain'};
function applyFeaturePolicy(){
 const permissions={...(myPolicy.permissions||{})};if(permissions.banned)for(const key of Object.keys(featureLabels))permissions[key]=false;
 if(permissions.designAI===false){$('toolDock')?.classList.add('hidden');for(const id of ['headCropModal','removeBgModal','vectorModal','upscaleModal'])$(id)?.classList.add('hidden');}
 for(const [id,key]of [['railTools','designAI'],['fileInput','sendFiles']]){const node=$(id);if(node){node.disabled=permissions[key]===false;node.title=permissions[key]===false?'Akses dibatasi HOST':node.title;}}
}
window.kc.onFeaturePolicy?.(permissions=>{myPolicy={...myPolicy,permissions:{...myPolicy.permissions,...permissions}};applyFeaturePolicy();});
document.addEventListener('click',event=>{const control=event.target.closest?.('#railTools,#toolDock .tool-item,#headCropButton,#removeBgButton,#vectorButton,#upscaleButton,[data-tool]');if(!control)return;const key='designAI';if(myPolicy.permissions?.banned||myPolicy.permissions?.[key]===false){event.preventDefault();event.stopImmediatePropagation();showToast('Akses dibatasi HOST.');}},true);
function renderRoleManager(data){
 let box=$('adminRoleManager');if(!box){box=document.createElement('div');box.id='adminRoleManager';box.className='profile-setting-block';$('adminUsers').before(box);}
 const roles=data.roles||[],labels={personal:'Tag nama',editor:'@editor',everyone:'@everyone',...featureLabels};
 box.innerHTML=`<b>Kelola role</b><small>Ubah role bawaan atau buat role baru. Menyimpan role memperbarui izin semua anggota role tersebut; catatan tetap disimpan.</small><select id="managedRole"><option value="">Tambah role baru</option>${roles.map(r=>`<option value="${esc(r.id)}">${esc(r.name)}</option>`).join('')}</select><input id="managedRoleName" maxlength="40" placeholder="Nama role, contoh: QC"><div class="permission-row">${Object.entries(labels).map(([key,label])=>`<label><input type="checkbox" data-role-perm="${key}" ${['editor','everyone'].includes(key)?'':'checked'}> ${label}</label>`).join('')}</div><div class="permission-row"><label>Level notif <select id="managedRoleLevel"><option value="standard">Standar</option><option value="notice" selected>Lumayan</option><option value="medium">Sedang</option><option value="fullscreen">Full Layar</option></select></label><label>Full Layar <select id="managedRoleMode"><option value="video">Video BLEDAR</option><option value="open-chat">Buka Chat</option></select></label><label><input id="managedRoleMute" type="checkbox" checked> Jebol mute aplikasi</label><label><input id="managedRoleWindows" type="checkbox"> Popup di atas Windows</label></div><button id="saveManagedRole" class="primary">Simpan role</button>`;
 $('managedRole').onchange=()=>{const role=roles.find(r=>r.id===$('managedRole').value);$('managedRoleName').value=role?.name||'';box.querySelectorAll('[data-role-perm]').forEach(input=>input.checked=role?role.permissions?.[input.dataset.rolePerm]!==false&&!(['editor','everyone'].includes(input.dataset.rolePerm)&&!role.permissions?.[input.dataset.rolePerm]):!['editor','everyone'].includes(input.dataset.rolePerm));$('managedRoleLevel').value=role?.mentionLevel||'notice';$('managedRoleMode').value=role?.fullscreenMode||'video';$('managedRoleMute').checked=role?.bypassAppMute!==false;$('managedRoleWindows').checked=role?.bypassWindowsMute===true;};
 $('saveManagedRole').onclick=async()=>{const button=$('saveManagedRole'),permissions={};box.querySelectorAll('[data-role-perm]').forEach(input=>permissions[input.dataset.rolePerm]=input.checked);button.disabled=true;try{const r=await hostAdminRequest('/admin/moderation/role',{method:'POST',headers:adminHeaders(),body:JSON.stringify({id:$('managedRole').value||undefined,name:$('managedRoleName').value,permissions,mentionLevel:$('managedRoleLevel').value,fullscreenMode:$('managedRoleMode').value,bypassAppMute:$('managedRoleMute').checked,bypassWindowsMute:$('managedRoleWindows').checked})}),j=await r.json();if(!r.ok||!j.ok)throw Error(j.error||'Role gagal disimpan');renderAdmin(j);showToast('Role dan izin anggotanya disimpan.');}catch(error){showToast(error.message);}finally{button.disabled=false;}};
}
let moderationRender=0;
async function renderAdmin(data){
  data = {...data, users: data.users.filter(user => user.online === true)};
  moderationLoadedAt = Date.now();
  const revision=++moderationRender;
  renderRoleManager(data);
  $('hostHandoverSection')?.classList.toggle('hidden',!cfg?.isHost);
  const spam=data.tagSpamPolicy||{maxPer10Seconds:2,maxPerMinute:6,blockSeconds:30};if($('spamTagsBurst'))$('spamTagsBurst').value=String(spam.maxPer10Seconds);if($('spamTagsMinute'))$('spamTagsMinute').value=String(spam.maxPerMinute);if($('spamBlockSeconds'))$('spamBlockSeconds').value=String(spam.blockSeconds);if($('saveTagSpamPolicy'))$('saveTagSpamPolicy').onclick=saveTagSpamPolicy;
  const users=$('adminUsers');
  const renderUser=u=>{const p=u.policy||{},per=p.permissions||{},host=u.isHost,level=p.mentionLevel||'notice',role=p.moderationRole||(host?'DEVELOPER':'EDITOR');return`<div class="admin-user ${u.online?'client-online':''}" data-device="${esc(u.deviceId)}" data-profile="${esc(u.name)}"><div class="admin-user-head">${avatarHtml(u.avatarUrl,u.name,'person-avatar')}<div><b>${esc(profileDisplayName(u.name))} ${host?'<em class="host-badge">HOST</em>':''} <em class="moderation-role-badge">${esc(host?'HOST':Object.values(p.cohost||{}).some(Boolean)?'Co-HOST':'Anggota')}</em> <em class="client-version-badge ${u.online?'online':''}">${u.online?'AKTIF':'OFFLINE'} · v${esc(u.version||'?')}</em></b><span>${esc(u.pc||'')} · ${fmtMute(p.mutedUntil)}</span></div></div><div class="moderation-role-row"><label><span>Preset izin anggota</span><select class="moderation-role-select" ${host?'disabled':''}><option value="DEVELOPER" ${role==='DEVELOPER'?'selected':''} ${host?'':'disabled'}>Developer · HOST saja</option>${(data.roles||[{id:"ADMIN",name:"Prioritas"},{id:"EDITOR",name:"Editor"}]).map(r=>`<option value="${esc(r.id)}" ${(p.roleTemplateId||role)===r.id?"selected":""}>${esc(r.name)}</option>`).join("")}<option value="CUSTOM" ${role==='CUSTOM'&&!String(p.roleTemplateId||'').startsWith('role-')?'selected':''}>Custom manual</option></select></label><label class="custom-role-label ${role==='CUSTOM'?'':'hidden'}"><span>Nama role custom</span><input class="custom-role-name" maxlength="40" value="${esc(p.customRoleName||'')}" placeholder="Contoh: QC Senior" ${host?'disabled':''}></label><small>${host?'Developer terkunci: akses penuh khusus PC HOST. Level notif dan mode Full Layar tetap bisa diubah.':'Admin/Editor memberi preset awal, lalu semua izin di bawah tetap bisa disesuaikan per orang.'}</small></div><div class="permission-row"><label><input type="checkbox" data-perm="banned" ${per.banned?"checked":""} ${host?"disabled":""}> Ban seluruh akses akun</label>${Object.entries(featureLabels).map(([key,label])=>`<label><input type="checkbox" data-perm="${key}" ${per[key]!==false?"checked":""} ${host?"disabled":""}> ${label}</label>`).join("")}<label><input type="checkbox" data-perm="personal" ${per.personal?'checked':''} ${host?'disabled':''}> Tag nama</label><label><input type="checkbox" data-perm="editor" ${per.editor?'checked':''} ${host?'disabled':''}> @editor</label><label><input type="checkbox" data-perm="everyone" ${per.everyone?'checked':''} ${host?'disabled':''}> @everyone</label><label class="priority-toggle"><input type="checkbox" data-policy="tagVideoEnabled" ${p.tagVideoEnabled?'checked':''}> Video saat pengguna ini men-tag</label><label class="priority-toggle"><input type="checkbox" data-policy="bypassAppMute" ${p.bypassAppMute?'checked':''} ${host?'disabled':''}> Tag jebol mute aplikasi/chat</label><label class="priority-toggle"><input type="checkbox" data-policy="bypassWindowsMute" ${p.bypassWindowsMute?'checked':''} ${host?'disabled':''}> Tag paksa popup di atas Windows</label><label class="level-label"><span>Kekuatan notif tag</span><select class="mention-level-select"><option value="standard" ${level==='standard'?'selected':''}>Standar</option><option value="notice" ${level==='notice'?'selected':''}>Lumayan</option><option value="medium" ${level==='medium'?'selected':''}>Sedang</option><option value="fullscreen" ${level==='fullscreen'?'selected':''}>Full Layar</option></select></label><label class="level-label fullscreen-mode-label"><span>Saat Full Layar</span><select class="fullscreen-mode-select"><option value="video" ${(p.fullscreenMode||'video')==='video'?'selected':''}>Video BLEDAR</option><option value="open-chat" ${p.fullscreenMode==='open-chat'?'selected':''}>Popup tanpa video</option></select></label><select class="ban-select" ${host?'disabled':''}><option value="0">Ban mention...</option><option value="5">5 menit</option><option value="15">15 menit</option><option value="60">1 jam</option><option value="480">8 jam</option><option value="1440">1 hari</option><option value="clear">Cabut ban</option></select></div></div>`;};
  users.replaceChildren();
  if(!data.users.length)users.textContent='Belum ada anggota online.';
  let shown = 0;
  const more = document.createElement('button');
  more.type = 'button';
  more.className = 'secondary';
  const appendPage = () => {
    if (revision !== moderationRender) return;
    more.remove();
    const template = document.createElement('template');
    template.innerHTML = data.users.slice(shown, shown + 6).map(renderUser).join('');
    shown += template.content.childElementCount;
    users.append(template.content);
    if (shown < data.users.length) {
      more.textContent = 'Tampilkan anggota lain (' + (data.users.length - shown) + ')';
      users.append(more);
    }
  };
  more.onclick = appendPage;
  if (data.users.length) {
    more.textContent = 'Tampilkan anggota (' + data.users.length + ')';
    users.append(more);
  }
  $('auditList').innerHTML=(data.audit||[]).slice(0,30).map(a=>`<div class="audit-row"><b>${esc(a.actor)}</b><span>${a.type==='group-mention'?'menggunakan group mention':'mengubah moderation'} · ${new Date(a.ts).toLocaleString('id-ID')}</span></div>`).join('')||'<div class="empty-state">Belum ada audit.</div>';
}
$('adminUsers').addEventListener('change',event=>{
  const input=event.target;
  if(input.matches('[data-perm]'))void updatePermission(input);
  else if(input.matches('[data-policy]'))void updatePolicyToggle(input);
  else if(input.matches('.moderation-role-select'))void updateModerationRole(input);
  else if(input.matches('.custom-role-name'))void updateCustomRoleName(input);
  else if(input.matches('.ban-select'))void applyBan(input);
  else if(input.matches('.mention-level-select'))void updateMentionLevel(input);
  else if(input.matches('.fullscreen-mode-select'))void updateFullscreenMode(input);
});
async function saveTagSpamPolicy(){const body={maxPer10Seconds:Number($('spamTagsBurst')?.value||2),maxPerMinute:Number($('spamTagsMinute')?.value||6),blockSeconds:Number($('spamBlockSeconds')?.value||30)};try{const r=await hostAdminRequest('/admin/moderation/tag-spam',{method:'POST',headers:adminHeaders(),body:JSON.stringify(body)}),j=await r.json();if(!r.ok||!j.ok)throw Error(j.error||'Pengaturan gagal disimpan');renderAdmin({...j,users:j.users||[]});showToast('Batas spam tag diperbarui.');}catch(e){showToast(e.message||'Pengaturan spam gagal disimpan');}}
async function moderationPost(card,payload){const deviceId=card.dataset.device,profileName=card.dataset.profile,r=await hostAdminRequest('/admin/moderation/user',{method:'POST',headers:adminHeaders(),body:JSON.stringify({deviceId,profileName,...payload})}),j=await r.json();if(!r.ok||!j.ok)throw new Error(j.error||'Gagal mengubah moderation');renderAdmin(j);return j;}
async function updatePermission(input){const card=input.closest('.admin-user');try{await moderationPost(card,{permissions:{[input.dataset.perm]:input.checked}});}catch(e){alert(e.message);input.checked=!input.checked;}}
async function updatePolicyToggle(input){const card=input.closest('.admin-user');try{await moderationPost(card,{[input.dataset.policy]:input.checked});showToast('Prioritas tag diperbarui.');}catch(e){alert(e.message);input.checked=!input.checked;}}
async function updateModerationRole(sel){const card=sel.closest('.admin-user'),custom=card.querySelector('.custom-role-name')?.value||'';try{await moderationPost(card,sel.value==='CUSTOM'?{moderationRole:'CUSTOM',customRoleName:custom,applyRoleDefaults:true}:{roleTemplateId:sel.value});showToast('Role moderation diperbarui. Izin masih bisa diedit manual.');}catch(e){alert(e.message);await loadAdmin();}}
async function updateCustomRoleName(input){const card=input.closest('.admin-user');if(!input.value.trim())input.value='Custom';try{await moderationPost(card,{moderationRole:'CUSTOM',customRoleName:input.value.trim()});showToast('Nama role custom disimpan.');}catch(e){alert(e.message);}}
async function updateFullscreenMode(sel){const card=sel.closest('.admin-user');try{await moderationPost(card,{fullscreenMode:sel.value});showToast('Mode Full Layar diperbarui.');}catch(e){alert(e.message);}}
async function updateMentionLevel(sel){const card=sel.closest('.admin-user');try{await moderationPost(card,{mentionLevel:sel.value});showToast('Level notifikasi mention diperbarui.');}catch(e){alert(e.message);}}
async function applyBan(sel){const card=sel.closest('.admin-user'),val=sel.value;if(val==='0')return;const mutedUntil=val==='clear'?0:Date.now()+Number(val)*60000;try{await moderationPost(card,{mutedUntil});}catch(e){alert(e.message);}}


// v1.1.0 HOST Bug Center
function diagHeaders(){return{'Content-Type':'application/json','Authorization':'Bearer '+(cfg.adminToken||'')}}
const creativeJobs=[];let creativeJobSeq=0;
function jobStart(type,label,total=1){const j={id:++creativeJobSeq,type,label,total,current:0,status:'running',startedAt:Date.now(),detail:''};creativeJobs.unshift(j);renderJobs();return j;}
function jobUpdate(j,current,detail=''){if(!j)return;j.current=current;j.detail=detail;renderJobs();}
function jobDone(j,ok=true,detail=''){if(!j)return;j.status=ok?'done':'failed';j.current=j.total;j.detail=detail||j.detail;j.endedAt=Date.now();renderJobs();}
function renderJobs(){const box=$('jobList');if(!box)return;box.innerHTML=creativeJobs.slice(0,50).map(j=>`<div class="job-item"><div class="job-main"><b>${esc(j.label)}</b><span>${esc(j.detail||j.type)} · ${j.status}</span></div><progress class="job-progress" max="${j.total||1}" value="${Math.min(j.current,j.total)||0}"></progress></div>`).join('')||'<div class="empty-state">Belum ada job.</div>';}
function applyPerformanceProfile(profile,save=false){profile=['low','balanced','high'].includes(profile)?profile:'balanced';if($('performanceProfile'))$('performanceProfile').value=profile;const parallel=profile==='high';['vectorParallel','upscaleParallel','removeBgParallel','headCropParallel'].forEach(id=>{if($(id))$(id).checked=parallel;});if(profile==='low'){if($('upscaleScale'))$('upscaleScale').value='2';if($('upscaleModel'))$('upscaleModel').value='lite';}else if(profile==='balanced'){if($('upscaleScale'))$('upscaleScale').value='2';if($('upscaleModel'))$('upscaleModel').value='standard';}else{if($('upscaleScale'))$('upscaleScale').value='4';if($('upscaleModel'))$('upscaleModel').value='fidelity';}if(save)window.kc.saveConfig({performanceProfile:profile});}
if($('performanceProfile'))$('performanceProfile').onchange=e=>{applyPerformanceProfile(e.target.value,true);showToast('Profil performa disimpan di PC ini.');};
async function loadSystem(){try{const s=await window.kc.getSystemStats();applyPerformanceProfile(s.performanceProfile||'balanced',false);const memUsed=(s.totalMem||0)-(s.freeMem||0),pct=s.totalMem?Math.round(memUsed/s.totalMem*100):0,gpu=(s.gpus||[])[0];$('systemStats').innerHTML=`<div class="system-stat"><span>CPU</span><b>${esc(s.cpu||'CPU')}</b><small>${s.cpuCount||0} thread</small></div><div class="system-stat"><span>RAM</span><b>${pct}%</b><small>${fmtSize(memUsed)} / ${fmtSize(s.totalMem||0)}</small></div><div class="system-stat"><span>GPU</span><b>${esc(gpu?.name||'Tidak terbaca')}</b><small>${gpu?.vram?fmtSize(gpu.vram):'VRAM tidak terbaca'}</small></div><div class="system-stat"><span>Creative Queue</span><b>${s.creative?.busy?'1 aktif':'Idle'}</b><small>${s.creative?.queued||0} menunggu</small></div>`;$('backupSection')?.classList.toggle('hidden',!cfg.isHost);if(cfg.isHost){$('backupPath').textContent=s.backup?.backupDir||'Belum ada folder backup.';$('autoBackup').checked=!!s.backup?.autoBackup;$('backupKeep').value=String(s.backup?.backupKeep||7);$('backupStatus').textContent=s.backup?.autoBackup?'Otomatis aktif':'Manual';const r=await hostAdminRequest('/admin/system-status',{headers:diagHeaders(),cache:'no-store'}),j=await r.json();if(j.ok){$('clientSummary').textContent=`${(j.online||[]).length} online`;$('systemClients').innerHTML=(j.known||[]).slice(0,30).map(u=>`<div class="system-client ${u.online?'':'offline'}"><div><b>${esc(u.name||'Editor')}${u.isHost?' · HOST':''}</b><small>${esc(u.pc||'')} · v${esc(u.version||'?')}</small></div><span class="client-state ${u.online?'online':'offline'}">${u.online?'ONLINE':'OFFLINE'}</span></div>`).join('')||'<div class="empty-state">Belum ada client.</div>';}}renderJobs();if($('statusJobs'))$('statusJobs').textContent=`Job: ${s.creative?.busy?'aktif':'idle'}${s.creative?.queued?` +${s.creative.queued} antre`:''}`;if($('statusBackup'))$('statusBackup').textContent=`Backup: ${s.backup?.autoBackup?'auto':'manual'}`;}catch(e){showToast('System Dashboard gagal: '+e.message)}}
async function runSystemHealth(){const btn=$('runHealthCheck');btn.disabled=true;$('healthSummary').textContent='Memeriksa...';try{const r=await window.kc.runHealthCheck();$('healthList').innerHTML=(r.checks||[]).map(c=>`<div class="health-item ${c.ok?'ok':'bad'}"><div><strong>${c.ok?'✓':'✕'} ${esc(c.name)}</strong><small>${esc(c.detail||'')}</small></div></div>`).join('');const bad=(r.checks||[]).filter(x=>!x.ok).length;$('healthSummary').textContent=bad?`${bad} perlu perhatian`:'Semua inti OK';}catch(e){$('healthSummary').textContent=e.message}finally{btn.disabled=false}}
if($('systemButton'))$('systemButton').onclick=async()=>{$('systemModal').classList.remove('hidden');await loadSystem();};if($('closeSystem'))$('closeSystem').onclick=()=>$('systemModal').classList.add('hidden');if($('refreshSystem'))$('refreshSystem').onclick=loadSystem;
if($('forceReconnect'))$('forceReconnect').onclick=async()=>{try{$('recoveryStatus').textContent='Mencari HOST...';if(socket){try{socket.disconnect()}catch{}}serverBase='';await connectLoop();$('recoveryStatus').textContent='Discovery ulang dijalankan';showToast('Reconnect / discovery ulang dijalankan.');}catch(e){$('recoveryStatus').textContent='Gagal';showToast(e.message)}};
if($('reloadUiSafe'))$('reloadUiSafe').onclick=async()=>{const r=await window.kc.reloadUiSafe();if(!r?.ok)showToast(r?.error||'Reload gagal');};
if($('verifyLatestBackup'))$('verifyLatestBackup').onclick=async()=>{if(!cfg.isHost)return showToast('Verifikasi backup hanya di HOST.');$('recoveryStatus').textContent='Memverifikasi backup...';const r=await window.kc.verifyLatestBackup();$('recoveryStatus').textContent=r?.ok?`Backup OK · ${r.fileCount} file`:'Backup perlu dicek';showToast(r?.ok?'Backup terakhir terverifikasi.':(r?.error||'Verifikasi gagal'));};if($('runHealthCheck'))$('runHealthCheck').onclick=runSystemHealth;if($('clearJobs'))$('clearJobs').onclick=()=>{for(let i=creativeJobs.length-1;i>=0;i--)if(creativeJobs[i].status!=='running')creativeJobs.splice(i,1);renderJobs();};
if($('chooseBackupDir'))$('chooseBackupDir').onclick=async()=>{const dir=await window.kc.chooseBackupFolder();if(!dir)return;await window.kc.saveConfig({backupDir:dir,autoBackup:$('autoBackup').checked,backupKeep:Number($('backupKeep').value||7)});$('backupPath').textContent=dir;showToast('Folder backup disimpan.');};if($('backupNow'))$('backupNow').onclick=async()=>{const dir=$('backupPath').textContent;if(!dir||dir.includes('Belum ada'))return showToast('Pilih folder backup dulu.');$('backupNow').disabled=true;const r=await window.kc.runBackupNow({dir});$('backupNow').disabled=false;if(r?.ok){$('backupStatus').textContent='Backup terbaru selesai';showToast('Backup selesai.')}else showToast(r?.error||'Backup gagal');};if($('autoBackup'))$('autoBackup').onchange=()=>window.kc.saveConfig({autoBackup:$('autoBackup').checked,backupKeep:Number($('backupKeep').value||7),backupDir:$('backupPath').textContent.includes('Belum ada')?'':$('backupPath').textContent});if($('backupKeep'))$('backupKeep').onchange=()=>window.kc.saveConfig({backupKeep:Number($('backupKeep').value||7)});

$('diagnosticsButton').onclick=async()=>{$('diagnosticsModal').classList.remove('hidden');await loadDiagnostics();};
$('closeDiagnostics').onclick=()=>$('diagnosticsModal').classList.add('hidden');
$('refreshDiagnostics').onclick=loadDiagnostics;$('diagFilter').onchange=loadDiagnostics;
function diagTime(ts){return ts?new Date(ts).toLocaleString('id-ID'):'—'}
function diagRecovery(g){if(Number(String(g.version||'0').split('.')[0])<7)return 'Update CLIENT ke v7, lalu periksa apakah error muncul lagi. Riwayat ini tidak dihapus otomatis.';if(/socket/.test(g.type||''))return 'Pastikan HOST aktif dan gunakan Perbaiki koneksi.';if(/image format|thumbnail|preview/i.test(g.message||''))return 'Gunakan PNG/JPG atau file Photoshop yang menyimpan thumbnail.';return 'Ekspor Support Log bila kejadian masih berulang.';}
function renderDiagnostics(data){$('diagOpen').textContent=data.open||0;$('diagTotal').textContent=data.totalReports||0;$('diagResolved').textContent=data.resolved||0;$('diagLast').textContent=data.lastReportAt?new Date(data.lastReportAt).toLocaleTimeString('id-ID',{hour:'2-digit',minute:'2-digit'}):'—';const f=$('diagFilter').value;const versionFilter=$('diagVersion')?.value||'all';const groups=(data.groups||[]).filter(g=>(f==='all'||(f==='resolved'?g.status==='resolved':g.status!=='resolved'))&&(versionFilter==='all'||(versionFilter==='v7'?Number(String(g.version||'0').split('.')[0])>=7:Number(String(g.version||'0').split('.')[0])<7)));$('diagnosticsList').innerHTML=groups.map(g=>`<div class="diag-item ${g.status==='resolved'?'resolved':''}"><div class="diag-head"><div><span class="diag-severity ${esc(g.severity||'error')}">${esc((g.severity||'error').toUpperCase())}</span><b>${esc(g.type||'Bug')}</b><em>${g.count||1}×</em></div><span>${diagTime(g.lastSeen)}</span></div><div class="diag-version-badge">${Number(String(g.version||'0').split('.')[0])>=7?'Laporan v7':'Riwayat versi sebelum v7 · periksa versi CLIENT'}</div><div class="diag-message">${esc(String(g.message||'').slice(0,240))}${String(g.message||'').length>240?'<details><summary>Detail pesan</summary><pre>'+esc(g.message)+'</pre></details>':''}</div><div class="diag-meta">${esc((g.role||'').toUpperCase())} · ${esc(g.pc||'PC?')} · ${esc(g.user||'')} · v${esc(g.version||'?')} · ${esc(g.source||'')}</div>${g.stack?`<details><summary>Stack trace</summary><pre>${esc(g.stack)}</pre></details>`:''}<p class="diag-recovery">${esc(diagRecovery(g))}</p><div class="diag-actions"><button data-diag-status="${esc(g.signature)}" data-next="${g.status==='resolved'?'open':'resolved'}" class="${g.status==='resolved'?'secondary':'primary'}">${g.status==='resolved'?'Buka lagi':'Tandai selesai'}</button></div></div>`).join('')||'<div class="empty-state">Tidak ada laporan di filter ini. 🎉</div>';$('diagnosticsList').querySelectorAll('[data-diag-status]').forEach(b=>b.onclick=async()=>{const r=await hostAdminRequest('/admin/diagnostics/status',{method:'POST',headers:diagHeaders(),body:JSON.stringify({signature:b.dataset.diagStatus,status:b.dataset.next})}),j=await r.json();if(!r.ok||!j.ok)return showToast(j.error||'Gagal mengubah status');renderDiagnostics(j);});}
async function loadDiagnostics(){if(!cfg.isHost)return;try{const r=await hostAdminRequest('/admin/diagnostics',{headers:diagHeaders(),cache:'no-store'}),j=await r.json();if(!r.ok||!j.ok)throw new Error(j.error||'Gagal membuka Bug Center');renderDiagnostics(j);}catch(e){$('diagnosticsList').innerHTML=`<div class="empty-state">${esc(e.message)}</div>`;}}
$('clearResolvedDiagnostics').onclick=async()=>{const yes=await appConfirm({title:'Hapus laporan selesai?',text:'Bug yang masih aktif tidak akan dihapus.',ok:'Hapus yang selesai'});if(!yes)return;const r=await hostAdminRequest('/admin/diagnostics/clear-resolved',{method:'POST',headers:diagHeaders(),body:'{}'}),j=await r.json();if(j.ok){renderDiagnostics(j);showToast(`${j.removed||0} grup bug selesai dihapus.`)}};
if($('openLogFolder'))$('openLogFolder').onclick=async()=>{const r=await window.kc.openLogFolder();if(!r?.ok)showToast(r?.error||'Gagal membuka folder log');};
if($('exportSupportLog'))$('exportSupportLog').onclick=async()=>{const r=await window.kc.exportSupportLog();if(r?.ok)showToast('Support Log berhasil diexport.');else if(!r?.canceled)showToast(r?.error||'Export Support Log gagal');};
$('exportDiagnostics').onclick=async()=>{try{const r=await hostAdminRequest('/admin/diagnostics/export',{headers:{'Authorization':'Bearer '+(cfg.adminToken||'')}});if(!r.ok)throw new Error('Export gagal');const blob=await r.blob(),a=document.createElement('a');a.href=URL.createObjectURL(blob);a.download=`ChatX-Chat-Diagnostics-${Date.now()}.json`;a.click();setTimeout(()=>URL.revokeObjectURL(a.href),1000);}catch(e){showToast(e.message)}};

function clientUpdateFeed(){
  return window.kc.supportsRustClientUpdates && $('updateClientChannel').value==='rust'
    ? '/rust-client-update' : '/client-update';
}
function rustInstallerVersion(name){
  return /^ChatX-CLIENT-SETUP-v(\d+\.\d+\.\d+)-Andromeda\.exe$/i.exec(name||'')?.[1];
}
function migrationFeedVersion(version){
  const [major,minor,patch]=version.split('.');
  return [Number(major)+7,minor,patch].join('.');
}
function updatePublishedStatus(status){
  const appVersion=rustInstallerVersion(status?.originalName);
  $('publishedUpdateStatus').textContent=status?.available
    ? `Aktif: ${appVersion ? 'ChatX Rust v'+appVersion+(appVersion!==status.version?' · Feed Electron v'+status.version:'') : 'v'+status.version} · ${fmtSize(status.size||0)}` : 'Belum ada update dipublish';
}
let updateStatusRequest=0;
async function loadClientUpdateStatus(){
  const feed=clientUpdateFeed(),request=++updateStatusRequest;
  const response=await hostAdminRequest(feed+'/status',{cache:'no-store'});
  if(!response.ok)throw Error('Gagal membaca status update');
  const status=await response.json();
  if(updateStatusRequest===request&&clientUpdateFeed()===feed)updatePublishedStatus(status);
}
async function loadRollbackHistory(){
  if(!cfg?.isHost)return;
  const feed=clientUpdateFeed();
  try{
    const response=await hostAdminRequest('/admin'+feed+'/history',{headers:diagHeaders(),cache:'no-store'});
    const data=await response.json();
    if(!data.ok)throw Error(data.error||'Gagal membaca riwayat');
    if(clientUpdateFeed()!==feed)return;
    $('rollbackVersion').innerHTML='<option value="">Riwayat update sebelumnya...</option>'+
      (data.items||[]).map(item=>`<option value="${esc(item.key)}">v${esc(item.version||'?')} · ${new Date(item.publishedAt||0).toLocaleString('id-ID')}</option>`).join('');
  }catch(error){showToast(error.message);}
}
$('refreshRollback').onclick=loadRollbackHistory;
$('rollbackClientUpdate').onclick=async()=>{
  const key=$('rollbackVersion').value,feed=clientUpdateFeed(),channel=$('updateClientChannel');
  if(!key)return showToast('Pilih versi rollback dulu.');
  channel.disabled=$('rollbackClientUpdate').disabled=$('publishClientUpdate').disabled=true;
  try{
    const yes=await appConfirm({title:'Rollback update CLIENT?',text:'Versi terpilih akan dipublikasikan kembali ke jalur CLIENT yang dipilih.',ok:'Rollback'});
    if(!yes)return;
    const backup=await window.kc.preUpdateBackup();
    if(!backup?.ok)throw Error(backup?.error||'Backup sebelum rollback gagal');
    const response=await hostAdminRequest('/admin'+feed+'/rollback',{method:'POST',headers:diagHeaders(),body:JSON.stringify({key})});
    const data=await response.json();
    if(!data.ok)throw Error(data.error||'Rollback gagal');
    showToast('Rollback v'+data.meta.version+' berhasil dipublish.');
    await Promise.all([loadClientUpdateStatus(),loadRollbackHistory()]);
  }catch(error){showToast(error.message);}
  finally{channel.disabled=$('rollbackClientUpdate').disabled=$('publishClientUpdate').disabled=false;}
};
$('chooseClientUpdate').onclick=()=>$('clientUpdateInstaller').click();
let publishArmedUntil=0;
function showChosenInstaller(){
  const appVersion=rustInstallerVersion(publishedInstallerFile?.name);
  const migration=appVersion&&clientUpdateFeed()==='/client-update';
  $('clientUpdateFileName').textContent=publishedInstallerFile
    ? `Dipilih: ${publishedInstallerFile.name} · ${fmtSize(publishedInstallerFile.size)}${migration?' · Feed Electron v'+migrationFeedVersion(appVersion):''}`
    : 'Pilih installer CLIENT sesuai jalur yang dipilih.';
}
function resetUpdateConfirmation(){
  publishArmedUntil=0;
  $('publishClientUpdate').textContent='Publish Update';
}
$('clientUpdateInstaller').onchange=event=>{
  publishedInstallerFile=event.target.files?.[0]||null;
  const appVersion=rustInstallerVersion(publishedInstallerFile?.name);
  if(appVersion)$('updateVersion').value=appVersion;
  resetUpdateConfirmation();
  showChosenInstaller();
};
$('updateVersion').addEventListener('input',resetUpdateConfirmation);
$('updateClientChannel').onchange=()=>{
  publishedInstallerFile=null;
  $('clientUpdateInstaller').value='';
  $('updateVersion').value='';
  $('updateVersion').placeholder='Versi yang terpasang di CLIENT';
  $('rollbackVersion').replaceChildren(new Option('Memuat riwayat update…',''));
  $('publishedUpdateStatus').textContent='Memeriksa status…';
  resetUpdateConfirmation();
  showChosenInstaller();
  void Promise.all([loadClientUpdateStatus(),loadRollbackHistory()]).catch(error=>showToast(error.message));
};
$('updateClientChannelField').classList.toggle('hidden',!window.kc.supportsRustClientUpdates);
$('publishClientUpdate').onclick=async()=>{
  if(!cfg.isHost)return;
  const feed=clientUpdateFeed(),installer=publishedInstallerFile,version=$('updateVersion').value.trim();
  if(!installer)return showToast('Pilih installer CLIENT .exe dulu.');
  if(!/^\d+\.\d+\.\d+$/.test(version))return showToast('Isi versi seperti 1.1.20');
  const rustVersion=rustInstallerVersion(installer.name);
  if(rustVersion&&version!==rustVersion)return showToast('Versi aplikasi harus sesuai nama installer.');
  const feedVersion=rustVersion&&feed==='/client-update'?migrationFeedVersion(rustVersion):version;
  const button=$('publishClientUpdate'),channel=$('updateClientChannel');
  if(Date.now()>publishArmedUntil){
    publishArmedUntil=Date.now()+6000;
    button.textContent=`Klik lagi: Publish v${version}`;
    setTimeout(()=>{if(!button.disabled&&Date.now()>publishArmedUntil)resetUpdateConfirmation();},6200);
    return;
  }
  resetUpdateConfirmation();
  const form=new FormData();
  form.append('installer',installer);
  form.append('version',feedVersion);
  const release = window.KCReleaseNotes?.get(version);
  const andromeda = release?.title === 'v1 Andromeda';
  form.append('notes',$('updateNotes').value.trim() || (andromeda ? release.title + ' · ' + release.highlights.join(' · ') : ''));
  form.append('required',String($('updateRequired').checked));
  form.append('changelog',$('updateFullChangelog').value || (andromeda ? release.changelog : ''));
  button.disabled=channel.disabled=$('rollbackClientUpdate').disabled=$('chooseClientUpdate').disabled=true;
  try{
    $('preUpdateBackupStatus').textContent='Membuat backup sebelum update...';
    const backup=await window.kc.preUpdateBackup();
    if(!backup?.ok){
      $('preUpdateBackupStatus').textContent='Backup gagal';
      throw Error(backup?.error||'Backup sebelum update gagal');
    }
    $('preUpdateBackupStatus').textContent='Backup aman: '+(backup.target||'selesai');
    button.textContent='Mengupload...';
    const response=await hostAdminRequest('/admin'+feed,{method:'POST',headers:{Authorization:'Bearer '+(cfg.adminToken||'')},body:form});
    const data=await response.json();
    if(!response.ok||!data.ok)throw Error(data.error||'Gagal publish update');
    updateStatusRequest++;
    updatePublishedStatus({available:true,version:feedVersion,originalName:installer.name,size:installer.size});
    showToast(`Update CLIENT v${version} sudah dipublish.`);
    publishedInstallerFile=null;
    $('clientUpdateInstaller').value='';
    $('clientUpdateFileName').textContent='Installer tersimpan di HOST.';
    await loadRollbackHistory();
  }catch(error){
    reportBug('publish-client-update-failed',error,'error',{installerSize:installer.size});
    showToast('Publish update gagal: '+error.message,4200);
  }finally{
    button.disabled=channel.disabled=$('rollbackClientUpdate').disabled=$('chooseClientUpdate').disabled=false;
    resetUpdateConfirmation();
  }
};
window.kc.onRequiredUpdateInstall?.(async state=>{if(!serverBase){window.kc.requiredUpdateStatus?.('HOST belum terhubung. Periksa jaringan, lalu coba lagi.');return;}if(!pendingClientUpdate)pendingClientUpdate={...state,downloadUrl:'/client-update/download'};try{await $('installUpdateNow').onclick();window.kc.requiredUpdateStatus?.($('updateBannerText').textContent);}catch(e){window.kc.requiredUpdateStatus?.('Update gagal: '+e.message);}});
$('installUpdateNow').onclick=async()=>{if(!pendingClientUpdate)return;const yes=pendingClientUpdate.manifest?.required===true||await appConfirm({title:`Download update v${pendingClientUpdate.version}?`,text:'Installer .exe akan didownload dari HOST lalu langsung dibuka. Kamu tinggal klik Install seperti instal manual. Aplikasi ini tidak akan ditutup otomatis.',ok:'Download & Buka'});if(!yes)return;$('installUpdateNow').disabled=true;$('dismissUpdate').disabled=true;$('updateBannerText').textContent='Mengunduh installer dari HOST... 0%';const r=await window.kc.installClientUpdate({url:fullUrl(pendingClientUpdate.downloadUrl||'/client-update/download'),manifest:pendingClientUpdate.manifest});if(!r?.ok){$('installUpdateNow').disabled=false;$('dismissUpdate').disabled=false;$('updateBannerText').textContent='Update gagal: '+(r?.error||'Tidak diketahui');return;}$('updateBannerText').textContent='Installer sudah dibuka. Silakan lanjutkan instalasi secara manual.';$('installUpdateNow').disabled=false;$('dismissUpdate').disabled=false;};
if(window.kc?.onClientUpdateProgress)window.kc.onClientUpdateProgress(d=>{if(!pendingClientUpdate)return;const pct=Math.max(0,Math.min(100,Number(d?.pct)||0));$('updateBannerText').textContent=`Mengunduh installer dari HOST... ${pct}%${d?.total?` · ${fmtSize(d.received||0)} / ${fmtSize(d.total)}`:''}`;});
$('dismissUpdate').onclick=()=>$('updateBanner').classList.add('hidden');
setInterval(()=>{if(cfg&&!cfg.isHost&&serverBase)checkClientUpdate();},60000);


let searchTimer=null;
function openSearch(initial=''){
  $('searchModal').classList.remove('hidden');
  $('searchQuery').value=initial||'';
  setTimeout(()=>$('searchQuery').focus(),0);
  if(initial)runSearch(initial);
  else {$('searchResults').innerHTML='';$('searchStatus').textContent='Ketik nama user, isi chat, mention, atau nama file.';}
}
async function runSearch(raw){
  const q=String(raw||'').trim();
  if(!q){$('searchResults').innerHTML='';$('searchStatus').textContent='Ketik minimal 1 karakter.';return;}
  $('searchStatus').textContent='Mencari…';
  try{
    const params=new URLSearchParams({q,type:$('searchType')?.value||'all',sender:$('searchSender')?.value||'',tag:$('searchTag')?.value||''});if($('searchFrom')?.value)params.set('from',String(new Date($('searchFrom').value+'T00:00:00').getTime()));if($('searchTo')?.value)params.set('to',String(new Date($('searchTo').value+'T23:59:59').getTime()));const r=await kcFetch(serverBase+'/search?conversationId='+encodeURIComponent(activeConversationId||'general')+'&'+params.toString(),{cache:'no-store'}),j=await r.json();
    if(!r.ok||!j.ok)throw new Error(j.error||'Pencarian gagal');
    const rows=j.results||[];
    $('searchStatus').textContent=rows.length?`${rows.length} hasil ditemukan`:'Tidak ada hasil.';
    $('searchResults').innerHTML=rows.map(m=>`<button class="search-result" data-message="${esc(m.id)}"><div class="search-result-head"><b>${esc(profileDisplayName(m.sender))}</b><span>${new Date(m.ts).toLocaleString('id-ID')}</span></div><div class="search-result-body">${m.file?'📎 '+esc(m.file.name)+' · ':''}${esc(messageDisplayText(m)||'(pesan file)')}</div></button>`).join('');
    $('searchResults').querySelectorAll('[data-message]').forEach(b=>b.onclick=()=>{$('searchModal').classList.add('hidden');jumpToMessage(b.dataset.message);});
  }catch(e){$('searchStatus').textContent=e.message;$('searchResults').innerHTML='';}
}
$('headerSearchButton').onclick=()=>openSearch('');
$('closeSearch').onclick=()=>$('searchModal').classList.add('hidden');
$('searchQuery').addEventListener('input',e=>{clearTimeout(searchTimer);searchTimer=setTimeout(()=>runSearch(e.target.value),180);});
$('searchQuery').addEventListener('keydown',e=>{if(e.key==='Escape')$('searchModal').classList.add('hidden');});
;['searchType','searchSender','searchTag','searchFrom','searchTo'].forEach(id=>$(id)?.addEventListener('change',()=>runSearch($('searchQuery').value)));
$('sidebarSearchInput').addEventListener('keydown',e=>{if(e.key==='Enter'){openSearch(e.target.value);}});
$('sidebarSearchInput').addEventListener('input',e=>{if(e.target.value.trim().length>=2){clearTimeout(searchTimer);searchTimer=setTimeout(()=>openSearch(e.target.value),350);}});


// v0.4.1 — Emoji picker + custom sticker editor
const EMOJI_GROUPS={
'😀': ['😀','😃','😄','😁','😆','😅','😂','🤣','😊','😇','🙂','🙃','😉','😌','😍','🥰','😘','😗','😙','😚','😋','😛','😝','😜','🤪','🤨','🧐','🤓','😎','🥸','🤩','🥳','🙂‍↕️','😏','😒','🙂‍↔️','😞','😔','😟','😕','🙁','☹️','😣','😖','😫','😩','🥺','🥹','😢','😭','😤','😠','😡','🤬','🤯','😳','🥵','🥶','😱','😨','😰','😥','😓','🫣','🤗','🫡','🤔','🫢','🤭','🤫','🤥','😶','😶‍🌫️','😐','😑','😬','🫨','🙄','😯','😦','😧','😮','😲','🥱','😴','🤤','😪','😵','😵‍💫','🤐','🥴','🤢','🤮','🤧','😷','🤒','🤕','🤑','🤠','😈','👿','👹','👺','🤡','💩','👻','💀','☠️','👽','👾','🤖'],
'👋': ['👋','🤚','🖐️','✋','🖖','🫱','🫲','🫳','🫴','👌','🤌','🤏','✌️','🤞','🫰','🤟','🤘','🤙','👈','👉','👆','🖕','👇','☝️','🫵','👍','👎','✊','👊','🤛','🤜','👏','🙌','🫶','👐','🤲','🤝','🙏','✍️','💅','🤳','💪','🦾','🦵','🦶','👂','👃','🧠','🫀','🫁','🦷','🦴','👀','👁️','👄','🫦','💋'],
'❤️': ['❤️','🩷','🧡','💛','💚','💙','🩵','💜','🤎','🖤','🩶','🤍','💔','❤️‍🔥','❤️‍🩹','❣️','💕','💞','💓','💗','💖','💘','💝','💟','💌','💯','💢','💥','💫','💦','💨','🕳️','💬','👁️‍🗨️','🗨️','🗯️','💭','💤'],
'🐶': ['🐶','🐱','🐭','🐹','🐰','🦊','🐻','🐼','🐻‍❄️','🐨','🐯','🦁','🐮','🐷','🐽','🐸','🐵','🙈','🙉','🙊','🐒','🐔','🐧','🐦','🐤','🐣','🐥','🦆','🦅','🦉','🦇','🐺','🐗','🐴','🦄','🐝','🪱','🐛','🦋','🐌','🐞','🐜','🪰','🪲','🪳','🦟','🦗','🕷️','🦂','🐢','🐍','🦎','🐙','🦑','🦐','🦞','🦀','🐠','🐟','🐡','🐬','🐳','🐋','🦈','🐊','🐅','🐆','🦓','🦍','🦧','🦣','🐘','🦛','🦏','🐪','🐫','🦒','🦘','🦬','🐃','🐂','🐄','🐎','🐖','🐏','🐑','🦙','🐐','🦌','🐕','🐩','🦮','🐕‍🦺','🐈','🐈‍⬛','🪿','🦃','🦚','🦜','🪽','🐇','🦝','🦨','🦡','🦫','🦦','🦥','🐁','🐀','🐿️','🦔'],
'🍔': ['🍏','🍎','🍐','🍊','🍋','🍋‍🟩','🍌','🍉','🍇','🍓','🫐','🍈','🍒','🍑','🥭','🍍','🥥','🥝','🍅','🍆','🥑','🥦','🫛','🥬','🥒','🌶️','🫑','🌽','🥕','🫒','🧄','🧅','🥔','🍠','🫘','🥐','🥯','🍞','🥖','🥨','🧀','🥚','🍳','🧈','🥞','🧇','🥓','🥩','🍗','🍖','🌭','🍔','🍟','🍕','🫓','🥪','🥙','🧆','🌮','🌯','🫔','🥗','🥘','🫕','🥫','🍝','🍜','🍲','🍛','🍣','🍱','🥟','🦪','🍤','🍙','🍚','🍘','🍥','🥠','🥮','🍢','🍡','🍧','🍨','🍦','🥧','🧁','🍰','🎂','🍮','🍭','🍬','🍫','🍿','🍩','🍪','🌰','🥜','🍯','🥛','☕','🫖','🍵','🧃','🥤','🧋','🍺','🍻','🥂'],
'⚽': ['⚽','🏀','🏈','⚾','🥎','🎾','🏐','🏉','🥏','🎱','🪀','🏓','🏸','🏒','🏑','🥍','🏏','🪃','🥅','⛳','🪁','🏹','🎣','🤿','🥊','🥋','🎽','🛹','🛼','🛷','⛸️','🥌','🎿','⛷️','🏂','🪂','🏋️','🤼','🤸','⛹️','🤺','🤾','🏌️','🏇','🧘','🏄','🏊','🤽','🚣','🧗','🚵','🚴','🏆','🥇','🥈','🥉','🏅','🎖️','🏵️','🎗️','🎫','🎟️','🎪','🤹','🎭','🩰','🎨','🎬','🎤','🎧','🎼','🎹','🥁','🪘','🎷','🎺','🪗','🎸','🪕','🎻','🪈','🎲','♟️','🎯','🎳','🎮','🎰','🧩'],
'🚗': ['🚗','🚕','🚙','🚌','🚎','🏎️','🚓','🚑','🚒','🚐','🛻','🚚','🚛','🚜','🏍️','🛵','🚲','🛴','🛹','🛼','🚨','🚔','🚍','🚘','🚖','🚡','🚠','🚟','🚃','🚋','🚞','🚝','🚄','🚅','🚈','🚂','🚆','🚇','🚊','🚉','✈️','🛫','🛬','🛩️','💺','🛰️','🚀','🛸','🚁','🛶','⛵','🚤','🛥️','🛳️','⛴️','🚢','⚓','🛟','⛽','🚧','🚦','🚥','🗺️','🗿','🗽','🗼','🏰','🏯','🏟️','🎡','🎢','🎠','⛲','⛱️','🏖️','🏝️','🏜️','🌋','⛰️','🏕️','🏠','🏢','🏥','🏦','🏨','🏪','🏫','🏭','🏯'],
'💡': ['⌚','📱','💻','⌨️','🖥️','🖨️','🖱️','🖲️','🕹️','🗜️','💽','💾','💿','📀','📼','📷','📸','📹','🎥','📽️','🎞️','📞','☎️','📟','📠','📺','📻','🎙️','🎚️','🎛️','🧭','⏱️','⏲️','⏰','🕰️','⌛','⏳','📡','🔋','🪫','🔌','💡','🔦','🕯️','🧯','🛢️','💸','💵','💴','💶','💷','🪙','💳','🧾','💎','⚖️','🪜','🧰','🪛','🔧','🔨','⚒️','🛠️','⛏️','🪚','🔩','⚙️','🪤','🧱','⛓️','🧲','🔫','💣','🧨','🪓','🔪','🗡️','🛡️','🚬','⚰️','🪦','⚱️','🏺','🔮','📿','🧿','🪬','💈','⚗️','🔭','🔬','🕳️','🩹','🩺','💊','💉','🩸','🧬','🦠','🧫','🧪'],
'✅': ['✅','❌','⭕','🛑','⛔','📛','🚫','💯','💢','♨️','🚷','🚯','🚳','🚱','🔞','📵','❗','❕','❓','❔','‼️','⁉️','⚠️','🚸','🔱','⚜️','🔰','♻️','✳️','❇️','❎','🌐','💠','Ⓜ️','🌀','💤','🏧','🚾','♿','🅿️','🛗','🚹','🚺','🚻','🚼','🚮','🎦','📶','🈁','🆗','🆙','🆒','🆕','🆓','0️⃣','1️⃣','2️⃣','3️⃣','4️⃣','5️⃣','6️⃣','7️⃣','8️⃣','9️⃣','🔟','#️⃣','*️⃣','⏏️','▶️','⏸️','⏯️','⏹️','⏺️','⏭️','⏮️','⏩','⏪','🔀','🔁','🔂','◀️','🔼','🔽','➡️','⬅️','⬆️','⬇️','↗️','↘️','↙️','↖️','↕️','↔️','🔄','↪️','↩️','⤴️','⤵️','🔃','🔛','🔝','🔜','☑️','🔘','🔴','🟠','🟡','🟢','🔵','🟣','⚫','⚪','🟤','🔺','🔻','🔸','🔹','🔶','🔷','🔳','🔲','▪️','▫️','◾','◽','◼️','◻️','🟥','🟧','🟨','🟩','🟦','🟪','⬛','⬜','🟫']};
let emojiGroup=Object.keys(EMOJI_GROUPS)[0];
function allEmojis(){return Object.values(EMOJI_GROUPS).flat();}
function renderEmojiPicker(){const q=String($('emojiSearch')?.value||'').trim();const groups=Object.keys(EMOJI_GROUPS);$('emojiTabs').innerHTML=groups.map(g=>`<button type="button" data-eg="${esc(g)}" class="${g===emojiGroup?'active':''}">${g}</button>`).join('');$('emojiTabs').querySelectorAll('[data-eg]').forEach(b=>b.onclick=()=>{emojiGroup=b.dataset.eg;renderEmojiPicker();});const list=q?allEmojis().filter(e=>e.includes(q)):EMOJI_GROUPS[emojiGroup];$('emojiGrid').innerHTML=list.map((e,i)=>`<button type="button" data-emoji="${i}" title="${esc(e)}">${e}</button>`).join('');$('emojiGrid').querySelectorAll('[data-emoji]').forEach((b,i)=>b.onclick=()=>{insertAtCursor(list[i]);});}
function openEmoji(){const pop=$('emojiPopover'),btn=$('emojiButton');renderEmojiPicker();const r=btn.getBoundingClientRect(),gap=10,maxH=Math.max(220,Math.min(520,r.top-gap-8));pop.style.left=Math.max(8,Math.min(innerWidth-390,r.left))+'px';pop.style.top='auto';pop.style.bottom=Math.max(8,innerHeight-r.top+gap)+'px';pop.style.maxHeight=maxH+'px';pop.classList.toggle('hidden');if(!pop.classList.contains('hidden'))setTimeout(()=>$('emojiSearch')?.focus(),20);}
$('emojiButton').onclick=openEmoji;
$('emojiSearch').oninput=renderEmojiPicker;
function installSearchClearButtons(){const ids=['searchQuery','searchSender','searchFrom','searchTo','sidebarSearchInput','newDmSearch','gallerySearch','storageFileSearch','emojiSearch'];for(const id of ids){const input=$(id);if(!input||input.dataset.clearControl)return;input.dataset.clearControl='1';const wrap=document.createElement('span');wrap.className='search-clear-wrap';input.parentNode.insertBefore(wrap,input);wrap.appendChild(input);const btn=document.createElement('button');btn.type='button';btn.className='search-clear-btn hidden';btn.textContent='×';btn.title='Hapus pencarian';btn.setAttribute('aria-label','Hapus pencarian');wrap.appendChild(btn);const sync=()=>btn.classList.toggle('hidden',!String(input.value||''));input.addEventListener('input',sync);input.addEventListener('change',sync);btn.addEventListener('click',()=>{input.value='';sync();input.dispatchEvent(new Event('input',{bubbles:true}));input.dispatchEvent(new Event('change',{bubbles:true}));if(id==='sidebarSearchInput'){$('searchModal')?.classList.add('hidden');renderConversationList()}if(['searchSender','searchFrom','searchTo'].includes(id))runSearch($('searchQuery')?.value||'');input.focus();});sync();}}
installSearchClearButtons();
document.addEventListener('mousedown',e=>{const pop=$('emojiPopover');if(!pop.classList.contains('hidden')&&!pop.contains(e.target)&&e.target!==$('emojiButton'))pop.classList.add('hidden');});

const stickerKey=()=>`kc-stickers:${String(cfg?.name||'user').toLowerCase()}`;
let stickerDatabase;
async function updateStickerCollection(key, change = items => items) {
  stickerDatabase ??= new Promise((resolve, reject) => {
    const request = indexedDB.open('chatx-sticker-library', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('collections');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  }).catch(error => { stickerDatabase = null; throw error; });
  const database = await stickerDatabase;
  let migrated = false;
  const items = await new Promise((resolve, reject) => {
    const transaction = database.transaction('collections', 'readwrite');
    const store = transaction.objectStore('collections');
    const request = store.get(key);
    let next;
    request.onsuccess = () => {
      try {
        const previous = request.result ?? JSON.parse(localStorage.getItem(key) || '[]').filter(Boolean);
        next = change(previous);
        migrated = request.result === undefined;
        if (migrated || next !== previous) store.put(next, key);
      } catch (error) {
        reject(error);
        transaction.abort();
      }
    };
    transaction.oncomplete = () => resolve(next);
    transaction.onabort = () => reject(transaction.error || Error('Penyimpanan stiker dibatalkan.'));
  });
  if (migrated) {
    try { localStorage.removeItem(key); } catch {}
  }
  return items;
}
async function saveStickerToCollection(data) {
  const key = stickerKey();
  try {
    let duplicate = false;
    await updateStickerCollection(key, items => {
      duplicate = items.includes(data);
      return duplicate ? items : [data, ...items];
    });
    if (duplicate) return showToast('Stiker ini sudah ada di koleksi.');
    if (key === stickerKey()) void renderStickers();
    showToast('Stiker disimpan ke koleksi.');
  } catch (error) {
    reportBug('sticker-storage-failed', error);
    showToast('Stiker gagal disimpan: ' + error.message);
  }
}
async function saveReceivedSticker(message) {
  try {
    const url = fullUrl(message?.file?.url);
    if (!url) throw Error('Gambar stiker tidak tersedia.');
    const png = await chatImagePng(url);
    saveStickerToCollection('data:image/png;base64,' + await pngBlobBase64(png));
  } catch (error) {
    showToast(error.message || 'Gagal menyimpan stiker.');
  }
}
async function renderStickers() {
  const key = stickerKey();
  let items;
  try { items = await updateStickerCollection(key); }
  catch (error) { showToast('Koleksi stiker gagal dibuka: ' + error.message); return; }
  if (key !== stickerKey()) return;
  const library = $('stickerLibrary');
  library.replaceChildren();
  if (!items.length) {
    const empty = document.createElement('div');
    empty.className = 'sticker-empty';
    empty.textContent = 'Belum ada stiker. Tambah stiker atau simpan dari chat.';
    library.append(empty);
    return;
  }
  for (const [index, src] of items.entries()) {
    const card = document.createElement('div');
    card.className = 'sticker-item';
    const send = document.createElement('button');
    send.type = 'button';
    send.className = 'send-saved-sticker';
    send.setAttribute('aria-label', 'Kirim stiker ' + (index + 1));
    send.onclick = () => sendStickerData(src);
    const image = document.createElement('img');
    image.src = src;
    image.alt = '';
    send.append(image);
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'remove-sticker';
    remove.title = 'Hapus dari koleksi';
    remove.setAttribute('aria-label', 'Hapus stiker ' + (index + 1));
    remove.textContent = '×';
    remove.onclick = async () => {
      try {
        const current = await updateStickerCollection(key, items => items.filter(item => item !== src));
        await renderStickers();
        (library.querySelectorAll('.remove-sticker')[Math.min(index, current.length - 1)]
          || $('newSticker')).focus();
        showToast('Stiker dihapus dari koleksi.');
      } catch (error) { showToast('Stiker gagal dihapus: ' + error.message); }
    };
    card.append(send, remove);
    library.append(card);
  }
}
$('stickerButton').onclick=()=>{$('stickerModal').classList.remove('hidden');renderStickers();};$('closeSticker').onclick=()=>$('stickerModal').classList.add('hidden');
function stickerData(){return window.KCStickerEditor.exportPng();}
$('saveSticker').onclick=()=>saveStickerToCollection(stickerData());

async function prepareLocalResultForChat(path,name){try{if(!path)return showToast('Belum ada hasil.');const rf=await window.kc.readFile(path);if(!rf?.base64)throw new Error(rf?.error||'Tidak bisa membaca hasil');const ext=(String(path).split('.').pop()||'png').toLowerCase(),mime=ext==='svg'?'image/svg+xml':ext==='jpg'||ext==='jpeg'?'image/jpeg':ext==='webp'?'image/webp':'image/png';const bin=atob(rf.base64);const bytes=new Uint8Array(bin.length);for(let i=0;i<bin.length;i++)bytes[i]=bin.charCodeAt(i);selectedFile=new File([bytes],name||String(path).split(/[\/]/).pop(),{type:mime});sendingSticker=false;showFilePreview();showToast('Hasil siap dikirim ke chat.');$('text')?.focus();}catch(e){showToast('Gagal menyiapkan hasil: '+e.message)}}
function prepareCanvasForChat(canvas,name){try{const b64=canvas.toDataURL('image/png').split(',')[1],bin=atob(b64),bytes=new Uint8Array(bin.length);for(let i=0;i<bin.length;i++)bytes[i]=bin.charCodeAt(i);selectedFile=new File([bytes],name||'hasil.png',{type:'image/png'});sendingSticker=false;showFilePreview();showToast('Hasil PNG siap dikirim ke chat.');$('text')?.focus();}catch(e){showToast('Gagal menyiapkan hasil: '+e.message)}}

async function sendStickerData(source){
  const conversationId=activeConversationId;
  if(!conversationId)return showToast('Pilih chat dulu.');
  if(attachmentSending)return showToast('File sebelumnya masih dikirim.');
  try{
    $('stickerModal').classList.add('hidden');
    $('stickerEditorModal').classList.add('hidden');
    await yieldChatUi();
    const blob=typeof source==='function'?await source():await (await kcFetch(source)).blob();
    if(conversationId!==activeConversationId)return showToast('Chat berubah. Buka chat tujuan untuk mengirim stiker.');
    await send(null,new File([blob],`sticker-${Date.now()}.png`,{type:'image/png'}));
  }catch(error){showToast('Gagal mengirim stiker: '+error.message);}
}
$('sendSticker').onclick=()=>sendStickerData(window.KCStickerEditor.exportBlob);



// v1.0.2 — attachment menu + animated lightweight 3D games
let currentGame=null,incomingGame=null,chessSelected=null,lastGameFxId='',gameRenderSignature='',powerRaf=0,ludoPower=52,ludoPowerDir=1,powerManualUntil=0;
let gameAudioCtx=null;
function audioCtx(){try{if(!gameAudioCtx)gameAudioCtx=new (window.AudioContext||window.webkitAudioContext)();if(gameAudioCtx.state==='suspended')gameAudioCtx.resume();return gameAudioCtx}catch{return null}}
function gameTone(freq,dur=.08,type='sine',vol=.05,delay=0){const c=audioCtx();if(!c)return;const o=c.createOscillator(),g=c.createGain(),t=c.currentTime+delay;o.type=type;o.frequency.setValueAtTime(freq,t);g.gain.setValueAtTime(0.0001,t);g.gain.exponentialRampToValueAtTime(Math.max(.001,vol),t+.01);g.gain.exponentialRampToValueAtTime(.0001,t+dur);o.connect(g);g.connect(c.destination);o.start(t);o.stop(t+dur+.02)}
function gameSound(kind,power=50){const k=String(kind);if(k==='select')gameTone(520,.045,'sine',.025);else if(k==='chess-move'){gameTone(240,.07,'triangle',.04);gameTone(330,.08,'sine',.03,.055)}else if(k==='capture'){gameTone(150,.11,'square',.055);gameTone(95,.15,'triangle',.045,.07)}else if(k==='dice'){const v=.025+Math.max(0,Math.min(100,power))/3000;[330,470,610,760].forEach((f,i)=>gameTone(f,.045,'square',v,i*.045))}else if(k==='pawn'){gameTone(390,.055,'triangle',.035);gameTone(520,.055,'triangle',.03,.055)}else if(k==='win'){[523,659,784,1046].forEach((f,i)=>gameTone(f,.13,'sine',.045,i*.09))}}
function workTagMeta(tag){return ({'print-test':{label:'TES CETAK',mention:'@tescetak'},'design-revision':{label:'REVISI DESAIN',mention:'@revisidesain'},'design-request':{label:'MINTA DESAIN',mention:'@mintadesain'}})[tag]||null;}
function roleName(){return myTeamRole();}
function workTagAllowed(){return true;}
function workTagLabel(mode){return mode==='design-request'?'🎨 MINTA DESAIN':mode==='print-test'?'🧪 TES CETAK':'✏️ REVISI DESAIN';}
function userAcceptsWorkTag(u,mode){return normalizeWorkTagPrefs(u?.workTagPrefs)[mode]!==false;}
function openWorkTag(mode){if(!activeConversationId)return showToast('Pilih chat dulu.');workTagConversationId=String(activeConversationId);workTagMode=mode;$('attachMenu')?.classList.add('hidden');const conv=activeConversation(),isPrivate=conv?.type==='dm';$('workTagTitle').textContent=workTagLabel(mode)+' · '+(isPrivate?'Tag Pribadi':'Tag Grup');if(isPrivate){const peer=displayConversationName(conv),u=[...(onlineUsers||[]),...(editorDirectory||[])].find(x=>profileKeyLocal(x.name)===profileKeyLocal(peer));$('workTagSubtitle').textContent='Tag ini hanya dikirim ke '+peer;$('workTagUsers').innerHTML=`<button class="worktag-target default-target" data-target="@${esc(peer)}"><b>👤 ${esc(peer)}</b><span>Tag pribadi · ${esc(workTagMeta(mode).label)}</span></button>`;$('workTagUsers').querySelector('[data-target]')?.addEventListener('click',()=>sendWorkTagTarget('@'+peer,false));}else{$('workTagSubtitle').textContent=mode==='design-request'?'Tag grup · default kirim ke semua penerima Minta Desain':'Tag grup · pilih semua penerima atau orang tertentu';const users=(onlineUsers||[]).filter(u=>profileKeyLocal(u.name)!==profileKeyLocal(cfg.name)&&userAcceptsWorkTag(u,mode));const allLabel=mode==='design-request'?'Semua · default Minta Desain':'Semua penerima '+workTagMeta(mode).label;$('workTagUsers').innerHTML=`<button class="worktag-target default-target" data-target="@semua" data-all="1"><b>👥 ${esc(allLabel)}</b><span>${users.length} orang online menerima tag ini</span></button>`+users.map(u=>`<button class="worktag-target" data-target="@${esc(u.name)}"><b>${esc(profileDisplayName(u.name))}</b><span>${esc(jobTitle(u))} · menerima ${esc(workTagMeta(mode).label)}</span></button>`).join('');$('workTagUsers').querySelectorAll('[data-target]').forEach(b=>b.onclick=()=>sendWorkTagTarget(b.dataset.target,b.dataset.all==='1'));}$('workTagModal').classList.remove('hidden');}
async function sendWorkTagTarget(target,isAll=false){const conversationId=String(workTagConversationId||'');if(!conversationId||String(activeConversationId)!==conversationId){$('workTagModal')?.classList.add('hidden');return showToast('Chat berubah. Buka menu tag dari chat tujuan, lalu coba lagi.');}const note=$('workTagNote').value.trim(),meta=workTagMeta(workTagMode);if(!meta)return;const targetLabel=isAll?'@semua':target,text=`${targetLabel} ${meta.mention}${note?' · '+note:''}`,workTagTarget=isAll?{type:'all'}:{type:'user',name:String(target||'').replace(/^@/,'')};try{await socketSend({text,workTag:workTagMode,workTagTarget,conversationId,replyToId:currentReply?.id||null});$('workTagModal').classList.add('hidden');$('workTagNote').value='';clearReply();showToast(meta.label+' dikirim.');setTimeout(()=>$('text')?.focus(),0);}catch(e){showToast(e.message||'Tag gagal dikirim.');}}
function openAttachMenu(){const m=$('attachMenu'),b=$('attach');if(!m||!b)return;const opening=m.classList.contains('hidden');m.classList.toggle('hidden');if(!opening)return;requestAnimationFrame(()=>{const r=b.getBoundingClientRect(),mw=m.offsetWidth||240,mh=m.offsetHeight||320,pad=12;let left=Math.min(Math.max(pad,r.left),window.innerWidth-mw-pad);let top=r.top-mh-10;if(top<pad)top=Math.min(window.innerHeight-mh-pad,r.bottom+10);m.style.left=Math.round(left)+'px';m.style.top=Math.round(Math.max(pad,top))+'px';});}
$('attach').onclick=openAttachMenu;
$('attachFileAction').onclick=()=>{$('attachMenu').classList.add('hidden');$('fileInput').click();};
$('wallpaperAction').onclick=()=>{$('attachMenu').classList.add('hidden');$('wallpaperModal').classList.remove('hidden');};
$('printTestTagAction')?.addEventListener('click',()=>openWorkTag('print-test'));
$('revisionTagAction')?.addEventListener('click',()=>openWorkTag('design-revision'));
$('designRequestTagAction')?.addEventListener('click',()=>openWorkTag('design-request'));
$('closeWorkTag')?.addEventListener('click',()=>{$('workTagModal').classList.add('hidden');setTimeout(()=>$('text')?.focus(),0);});
const KC_WALL_KEY='kc-chat-wallpaper';
function applyChatWallpaper(v){
  const el=document.querySelector('.chat-panel');if(!el)return;
  el.classList.remove('wall-default','wall-doodle','wall-soft','wall-paper','wall-custom');
  el.style.removeProperty('--kc-custom-wallpaper');
  const type=v?.type||'default';
  if(type==='custom'&&v?.data){
    const safe=String(v.data).replace(/["\\\r\n]/g,m=>m==='"'?'%22':m==='\\'?'%5C':'');
    el.classList.add('wall-custom');
    el.style.setProperty('--kc-custom-wallpaper',`linear-gradient(var(--kc-wall-tint),var(--kc-wall-tint)),url("${safe}")`);
  }else el.classList.add('wall-'+type);
}
function saveChatWallpaperMode(v){try{localStorage.setItem(KC_WALL_KEY,JSON.stringify({type:v?.type||'default'}));}catch{}applyChatWallpaper(v);}
async function restoreChatWallpaper(){
  // v1.10.6 one-time wallpaper migration: legacy/custom state from older broken builds
  // could keep overriding the new requested default forever. Reset only once on upgrade.
  const migrationKey='kc-wallpaper-default-v1107';
  let migrated=false;try{migrated=localStorage.getItem(migrationKey)==='1';}catch{}
  if(!migrated){
    try{await window.kc?.clearChatWallpaper?.();}catch{}
    try{localStorage.setItem(KC_WALL_KEY,JSON.stringify({type:'default'}));localStorage.setItem(migrationKey,'1');}catch{}
    applyChatWallpaper({type:'default'});return;
  }
  let pref={type:'default'};try{pref=JSON.parse(localStorage.getItem(KC_WALL_KEY)||'{"type":"default"}')||pref;}catch{}
  if(pref.type==='custom'&&window.kc?.loadChatWallpaper){try{const r=await window.kc.loadChatWallpaper();if(r?.ok&&r.exists&&r.dataUrl)return applyChatWallpaper({type:'custom',data:r.dataUrl});}catch{}}
  applyChatWallpaper(pref);
}
restoreChatWallpaper();
$('wallpaperModal')?.querySelectorAll('[data-wall]').forEach(b=>b.onclick=()=>{saveChatWallpaperMode({type:b.dataset.wall});$('wallpaperModal').classList.add('hidden');showToast('Wallpaper diterapkan.');});
if($('customWallpaper'))$('customWallpaper').onclick=()=>$('wallpaperInput').click();
if($('wallpaperInput'))$('wallpaperInput').onchange=e=>{
  const f=e.target.files?.[0];if(!f)return;
  const rd=new FileReader();rd.onload=()=>{const im=new Image();im.onload=async()=>{
    const max=1920,scale=Math.min(1,max/Math.max(im.width,im.height)),cv=document.createElement('canvas');
    cv.width=Math.max(1,Math.round(im.width*scale));cv.height=Math.max(1,Math.round(im.height*scale));cv.getContext('2d').drawImage(im,0,0,cv.width,cv.height);
    const data=cv.toDataURL('image/jpeg',.82);
    try{
      const r=await window.kc?.saveChatWallpaper?.({dataUrl:data});
      if(!r?.ok)throw new Error(r?.error||'Gagal menyimpan wallpaper');
      try{localStorage.setItem(KC_WALL_KEY,JSON.stringify({type:'custom'}));}catch{}
      applyChatWallpaper({type:'custom',data:r.dataUrl||data});$('wallpaperModal').classList.add('hidden');showToast('Wallpaper custom diterapkan.');
    }catch(err){showToast('Wallpaper custom gagal: '+(err?.message||'coba gambar lain'));reportBug('custom-wallpaper-failed',err,'warning');}
  };im.onerror=()=>showToast('File gambar wallpaper tidak bisa dibaca.');im.src=rd.result;};rd.readAsDataURL(f);e.target.value='';
};
if($('resetWallpaper'))$('resetWallpaper').onclick=async()=>{try{await window.kc?.clearChatWallpaper?.();}catch{}saveChatWallpaperMode({type:'default'});$('wallpaperModal').classList.add('hidden');showToast('Wallpaper dikembalikan ke default.');};
if($('closeWallpaper'))$('closeWallpaper').onclick=()=>$('wallpaperModal').classList.add('hidden');

document.addEventListener('mousedown',e=>{const m=$('attachMenu');if(!m.classList.contains('hidden')&&!m.contains(e.target)&&e.target!==$('attach'))m.classList.add('hidden');});
let gameInviteSelected=new Set();
function gameInviteCandidates(){const by=new Map();for(const u of (onlineUsers||[])){const k=profileKeyLocal(u?.name);if(!k||k===profileKeyLocal(cfg?.name))continue;if(!by.has(k))by.set(k,u);}return [...by.values()].sort((a,b)=>String(a.name||'').localeCompare(String(b.name||''),'id',{sensitivity:'base'}));}
function updateGameInviteSelection(){const names=[...gameInviteSelected],el=$('gameInviteSelection');if(el)el.textContent=names.length?`Dipilih: ${names.join(', ')}`:'Belum ada lawan dipilih.';document.querySelectorAll('#gameInviteUsers [data-game-user]').forEach(row=>row.classList.toggle('selected',gameInviteSelected.has(row.dataset.gameUser)));}
function renderGameInviteUsers(){const box=$('gameInviteUsers');if(!box)return;const users=gameInviteCandidates();gameInviteSelected=new Set([...gameInviteSelected].filter(n=>users.some(u=>profileKeyLocal(u.name)===profileKeyLocal(n))));box.innerHTML=users.length?users.map(u=>`<button type="button" class="game-invite-user" data-game-user="${esc(u.name)}">${avatarHtml(u.avatarUrl,u.name,'avatar')}<span><b>${esc(profileDisplayName(u.name))}</b><small>${esc(jobTitle(u))} · online</small></span><i>✓</i></button>`).join(''):'<div class="game-empty-users">Tidak ada editor lain yang online.</div>';box.querySelectorAll('[data-game-user]').forEach(row=>row.onclick=()=>{const n=row.dataset.gameUser;if(gameInviteSelected.has(n))gameInviteSelected.delete(n);else gameInviteSelected.add(n);updateGameInviteSelection();});updateGameInviteSelection();}
function renderOpenSlitherRooms(rooms){const box=$('gameOpenSlitherList');if(!box)return;box.innerHTML=rooms?.length?rooms.map(r=>`<div class="slither-room-row"><span><b>🟢 ${esc(r.host)}</b><small>${r.humanPlayers} editor · ${r.players}/${r.maxPlayers} pemain</small></span><button data-slither-join="${esc(r.id)}" ${r.players>=r.maxPlayers?'disabled':''}>Gabung</button></div>`).join(''):'<small>Belum ada room aktif. Buat room baru untuk langsung bermain melawan bot.</small>';box.querySelectorAll('[data-slither-join]').forEach(b=>b.onclick=()=>{socket.emit('game-join',{id:b.dataset.slitherJoin,slitherColor:localStorage.getItem('kc.slither.skin')||'#47d7ac'},r=>{if(!r?.ok)return showToast(r?.error||'Gagal bergabung');currentGame=r.state;gameRenderSignature='';$('gameInviteModal').classList.add('hidden');$('railGame')?.classList.remove('active');openGame();});});}
function refreshSlitherRooms(){if(!socket?.connected)return renderOpenSlitherRooms([]);socket.emit('game-open-slither-list',{},r=>renderOpenSlitherRooms(r?.rooms||[]));}
$('refreshSlitherRooms')?.addEventListener('click',refreshSlitherRooms);
function openGameHub(){if(currentGame&&currentGame.status!=='finished')return openGame();gameInviteSelected.clear();renderGameInviteUsers();refreshSlitherRooms();$('gameInviteModal').classList.remove('hidden');$('railGame')?.classList.add('active');}
$('closeGameInvite').onclick=()=>{$('gameInviteModal').classList.add('hidden');$('railGame')?.classList.remove('active');};
$('gameInviteModal').querySelectorAll('[data-game]').forEach(b=>b.onclick=()=>{if(!socket?.connected)return showToast('HOST belum tersambung.');const type=b.dataset.game,names=type==='slither'?[]:[...gameInviteSelected];if((type==='chess'||type==='snakes')&&names.length>1)return showToast(`${gameName(type)} hanya bisa mengundang 1 orang.`);if(type==='ludo'&&names.length>3)return showToast('Ludo maksimal mengundang 3 orang.');socket.emit('game-create',{type,invitees:names,slitherColor:localStorage.getItem('kc.slither.skin')||'#47d7ac',color:type==='chess'?($('chessColorPref')?.value||'random'):($('ludoColorPref')?.value||'random'),botLevel:type==='chess'?($('chessBotLevel')?.value||'medium'):'medium'},r=>{if(!r?.ok)return alert(r?.error||'Gagal membuat game');currentGame=r.state;gameRenderSignature='';$('gameInviteModal').classList.add('hidden');$('railGame')?.classList.remove('active');openGame();});});
function gameName(t){return t==='ludo'?'Ludo':t==='snakes'?'Ular Tangga':t==='slither'?'Slither.io':'Catur'}
function detachedGameState(g){return g?{...g,viewerName:cfg?.name||''}:g}
function syncDetachedGame(g){if(g&&window.kc?.syncGameState)window.kc.syncGameState(detachedGameState(g));}
function gameFeedback(ok,text){window.kc?.syncGameFeedback?.({ok:!!ok,text:String(text||'')});}
function gameActionId(){try{return crypto.randomUUID()}catch{return 'ga-'+Date.now().toString(36)+'-'+Math.random().toString(36).slice(2)}}
function bindGameSocket(){if(!socket||socket._kcGameBound)return;socket._kcGameBound=true;socket.on('game-invite',inv=>{if(inv?.type==='slither')return;if(profileKeyLocal(inv.host)===profileKeyLocal(cfg.name))return;incomingGame=inv;$('incomingGameTitle').textContent=`🎮 Invite ${gameName(inv.type)} dari ${inv.host}`;$('incomingGameText').textContent=inv.type==='ludo'?`${inv.host} khusus mengundang kamu ke lobby Ludo. Terima untuk masuk sebagai pemain.`:inv.type==='snakes'?`${inv.host} mengundang kamu bermain Ular Tangga. Terima untuk masuk ke papan.`:`${inv.host} khusus mengundang kamu untuk Catur 1 lawan 1.`;$('incomingGameModal').classList.remove('hidden');window.kc?.notifyGameInvite?.({title:`🎮 Invite ${gameName(inv.type)} dari ${inv.host}`,body:`Kamu diundang bermain ${gameName(inv.type)}. Klik untuk kembali ke undangan.`});});socket.on('game-open-slither-list',renderOpenSlitherRooms);socket.on('game-state',g=>{if(g?.type==='slither')return;if(currentGame?.id===g.id){releaseLudoClickPending();currentGame=g;syncDetachedGame(g);if(!$('gameModal').classList.contains('hidden'))renderGame();}});socket.on('game-reconnect',items=>{items=(items||[]).filter(g=>g?.type!=='slither');if(!currentGame&&items?.length){currentGame=items[0];gameRenderSignature='';showToast('Game sebelumnya ditemukan. Buka menu Game untuk reconnect.');}});socket.on('game-closed',x=>{if(currentGame?.id===x.id){stopPowerMeter();currentGame=null;gameRenderSignature='';$('gameModal').classList.add('hidden');const msg=x?.reason==='player-left'?`${x.by||'Pemain'} keluar · game dibatalkan.`:x?.reason==='idle-3x'?`${x.by||'Pemain'} tidak jalan 3 giliran berturut-turut · game dibatalkan.`:'Game sudah ditutup.';window.kc?.syncGameFeedback?.({ok:false,text:msg,closed:true});showToast(msg,4800);}});socket.on('game-restart-consent',async x=>{if(!currentGame||String(x?.id)!==String(currentGame.id))return;const yes=await appConfirm({title:'Restart game?',text:`${x.from||'Pemain lain'} meminta pertandingan diulang dari awal.`,ok:'Setuju Restart'});if(yes)socket.emit('game-restart-request',{id:currentGame.id});});}
function profileKeyLocal(x){return String(x||'').trim().toLowerCase()}
function startClientPrimaryMonitor(){
  clearInterval(clientPrimaryMonitor);clientPrimaryMonitor=null;
  if(cfg?.isHost||!sessionReady)return;
  clientPrimaryMonitor=setInterval(async()=>{
    if(cfg?.isHost||!sessionReady||!socket?.connected||!serverBase||clientPrimaryBusy)return;
    clientPrimaryBusy=true;
    try{const primary=await window.kc.discoverPrimaryServer?.();if(primary&&primary!==serverBase&&lastPrimaryNotice!==primary){lastPrimaryNotice=primary;techLog('primary-host-found-while-on-temporary',{primary,current:serverBase});setConn('HOST utama aktif · menunggu sinkronisasi dan handover aman…',true);showToast('HOST utama sudah aktif. Client tetap terhubung sampai pemindahan data dari server non-utama terkonfirmasi.',4500);}}
    catch(error){techLog('primary-host-watch-failed',{message:error?.message||String(error)});}finally{clientPrimaryBusy=false;}
  },15000);clientPrimaryMonitor.unref?.();
}
async function scanHostPeerOnce(){
    if(!cfg?.isHost||!sessionReady||!socket?.connected||hostHandoverBusy||hostHandoverActive)return;
    hostHandoverBusy=true;
    try{const peer=await window.kc.discoverPeerServer?.();if(typeof peer==='string'&&isLanServerAddress(peer)){techLog('automatic-host-handover-peer-found',{peer});await beginHostHandover(peer,true);}else if(peer)techLog('automatic-host-handover-invalid-result',{type:typeof peer});}
    catch(error){techLog('automatic-host-handover-scan-failed',{message:error?.message||String(error)});}finally{hostHandoverBusy=false;}
}
function isLanServerAddress(value){try{const u=new URL(String(value||''));return u.protocol==='http:'&&!!u.hostname&&u.port==='45820'&&!u.username&&!u.password;}catch{return false}}
function startHostPeerMonitor(){
  clearInterval(hostPeerMonitor);hostPeerMonitor=null;
  if(!cfg?.isHost||!sessionReady||!socket?.connected)return;
  clearTimeout(hostHandoverReleaseTimer);hostHandoverReleaseTimer=null;hostHandoverActive=false;
  void scanHostPeerOnce();
  hostPeerMonitor=setInterval(()=>void scanHostPeerOnce(),45000);hostPeerMonitor.unref?.();
}
async function beginHostHandover(peer,automatic=false){
  if(!cfg?.isHost)return showToast('Sinkronisasi server hanya tersedia di aplikasi HOST.');
  if(hostHandoverActive||(hostHandoverBusy&&!automatic))return showToast('Pemeriksaan handover sedang berjalan.');
  hostHandoverBusy=true;hostHandoverActive=true;clearTimeout(hostHandoverReleaseTimer);hostHandoverReleaseTimer=setTimeout(()=>{hostHandoverActive=false;hostHandoverReleaseTimer=null;},90000);const button=$('manualHostHandover');if(button)button.disabled=true;
  setConn(automatic?'Server sementara ditemukan · sinkronisasi aman di latar belakang…':'Mencari server non-utama untuk sinkronisasi…',true);
  try{
    peer=peer||await window.kc.discoverPeerServer?.();
    if(!isLanServerAddress(peer)){hostHandoverActive=false;clearTimeout(hostHandoverReleaseTimer);hostHandoverReleaseTimer=null;setConn('HOST utama aktif · tidak ada server non-utama yang valid',true);if(!automatic)showToast('Server sementara tidak ditemukan atau alamatnya tidak valid.');return false;}
    techLog(automatic?'automatic-host-peer-sync-start':'manual-host-peer-sync-start',{peer,primary:serverBase});
    const result=await window.KCHostPeerHandover.handoverTemporaryPeer({
      io:(...args)=>io(...args),peer,
      hostUser:{name:cfg.name,pc:cfg.pc,deviceId:cfg.deviceId,deviceKey:cfg.deviceKey,version:cfg.version||'',adminToken:cfg.adminToken||null,teamRole:myTeamRole(),workTagPrefs:myWorkTagPrefs(),activitySharing:activityShareEnabled(),activity:lastLocalActivity||{kind:'chat',app:'ChatX',title:'Online di ChatX',state:'active'}},
      startHostPrimary:()=>window.kc.startHostPrimary(),
      syncPeerToHost:payload=>window.kc.syncPeerToHost(payload)
    });
    techLog('host-peer-sync-complete',{peer,hostAddress:result.hostAddress,...(result.synced||{})});
    setConn('HOST utama stabil · server non-utama sudah disinkronkan',true);
    if(!automatic)showToast('Data server non-utama sudah disinkronkan. HOST tetap aktif sebagai server utama.');
    return true;
  }catch(e){
    techLog('host-peer-sync-failed',{peer,message:e?.message||String(e)});
    hostHandoverActive=false;clearTimeout(hostHandoverReleaseTimer);hostHandoverReleaseTimer=null;
    setConn('HOST utama tetap aktif · sinkronisasi server non-utama akan dicoba lagi',true);
    if(!automatic)showToast(e?.message||'Sinkronisasi server non-utama gagal. HOST tetap aktif.');
    return false;
  }finally{hostHandoverBusy=false;if(button)setTimeout(()=>{button.disabled=false},2500);}
}
async function manualHostHandover(){return beginHostHandover(null,false);}
if($('manualHostHandover'))$('manualHostHandover').onclick=manualHostHandover;
if(window.kc?.onFocusGameInvite)window.kc.onFocusGameInvite(()=>{if(incomingGame)$('incomingGameModal')?.classList.remove('hidden');});
$('declineGame').onclick=()=>{if(incomingGame&&socket?.connected)socket.emit('game-decline',{id:incomingGame.id});$('incomingGameModal').classList.add('hidden');incomingGame=null;};$('acceptGame').onclick=()=>{if(!incomingGame||!socket?.connected)return;socket.emit('game-join',{id:incomingGame.id,color:'random'},r=>{if(!r?.ok)return alert(r?.error||'Gagal join game');currentGame=r.state;gameRenderSignature='';incomingGame=null;$('incomingGameModal').classList.add('hidden');openGame();});};
$('closeGame').onclick=()=>{stopPowerMeter();$('gameModal').classList.add('hidden');showToast('Game disembunyikan. Kamu bisa buka lagi dari menu Game.');};$('minimizeGame')?.addEventListener('click',()=>{$('gameModal').classList.toggle('game-minimized');});$('leaveGame').onclick=()=>{if(currentGame&&socket?.connected)socket.emit('game-leave',{id:currentGame.id});stopPowerMeter();currentGame=null;gameRenderSignature='';$('gameModal').classList.add('hidden');};
$('gameBot').onclick=()=>currentGame&&socket?.emit('game-add-bot',{id:currentGame.id},r=>{if(!r?.ok)showToast(r?.error||'Tidak bisa tambah bot')});$('gameCancel').onclick=async()=>{if(!currentGame)return;if(!await appConfirm({title:'Batalkan game?',text:'Lobby/game ini akan ditutup untuk semua pemain.',ok:'Batalkan Game'}))return;socket.emit('game-cancel',{id:currentGame.id},r=>{if(!r?.ok)showToast(r?.error||'Gagal membatalkan')});};
async function openGame(){if(!currentGame)return;if(window.kc?.openGameWindow){const ok=await window.kc.openGameWindow();if(ok){syncDetachedGame(currentGame);setTimeout(()=>syncDetachedGame(currentGame),300);$('gameModal').classList.add('hidden');}else{showToast('Window game gagal dibuka.');$('gameModal').classList.remove('hidden');renderGame();}}else{$('gameModal').classList.remove('hidden');renderGame();}}

window.kc?.onGameAction?.(async a=>{
  if(a?.type==='request-current'){
    if(currentGame){syncDetachedGame(currentGame);return;}
    if(!socket?.connected){gameFeedback(false,'HOST belum tersambung.');return;}
    socket.emit('game-active-request',{},r=>{if(r?.ok&&r.state){currentGame=r.state;gameRenderSignature='';syncDetachedGame(currentGame);gameFeedback(true,'Pertandingan tersambung kembali.');}else gameFeedback(false,'Tidak ada pertandingan aktif. Buat game baru dari menu Game di kiri.');});return;
  }
  if(a?.type==='reconnect'){
    if(!socket?.connected){gameFeedback(false,'HOST belum tersambung.');return;}
    if(currentGame){socket.emit('game-state-request',{id:currentGame.id},r=>{if(r?.ok&&r.state){currentGame=r.state;syncDetachedGame(currentGame);gameFeedback(true,'Reconnect berhasil.');}else{currentGame=null;socket.emit('game-active-request',{},x=>{if(x?.ok&&x.state){currentGame=x.state;syncDetachedGame(currentGame);gameFeedback(true,'Room aktif ditemukan.');}else gameFeedback(false,'Pertandingan lama sudah selesai. Buat game baru.');});}});gameFeedback(true,'Meminta state terbaru…');}
    else socket.emit('game-active-request',{},r=>{if(r?.ok&&r.state){currentGame=r.state;syncDetachedGame(currentGame);gameFeedback(true,'Reconnect berhasil.');}else gameFeedback(false,'Tidak ada pertandingan aktif.');});return;
  }
  if(!currentGame||String(a?.id||'')!==String(currentGame.id)){gameFeedback(false,'Game ini sudah tidak aktif. Klik ↻ untuk reconnect.');return;}
  if(!socket?.connected){gameFeedback(false,'Koneksi ke HOST terputus.');return;}
  const ack=(label)=>r=>{if(!r?.ok){gameFeedback(false,r?.error||label+' gagal');showToast(r?.error||label+' gagal');}else gameFeedback(true,r?.pending?'Menunggu persetujuan pemain lain…':label+' berhasil.');};
  if(a.type==='chess-move')socket.emit('chess-move',{id:currentGame.id,from:a.from,to:a.to},ack('Langkah'));
  else if(a.type==='ludo-roll')sendDetachedLudoAction('ludo-roll',{id:currentGame.id,power:a.power||50,clientActionId:a.clientActionId||gameActionId()});
  else if(a.type==='ludo-move')sendDetachedLudoAction('ludo-move',{id:currentGame.id,pawn:a.pawn,clientActionId:a.clientActionId||gameActionId()});
  else if(a.type==='snakes-roll')socket.emit('snakes-roll',{id:currentGame.id,clientActionId:a.clientActionId||gameActionId()},ack('Lempar dadu'));
  else if(a.type==='slither-input')socket.emit('slither-input',{id:currentGame.id,angle:a.angle,boost:a.boost===true});
  else if(a.type==='slither-respawn')socket.emit('slither-respawn',{id:currentGame.id},r=>{if(r?.ok&&r.state){currentGame=r.state;syncDetachedGame(currentGame)}else gameFeedback(false,r?.error||'Respawn gagal')});
  else if(a.type==='slither-skin')socket.emit('slither-skin',{id:currentGame.id,color:a.color},r=>{if(r?.ok&&r.state){currentGame=r.state;syncDetachedGame(currentGame)}else gameFeedback(false,r?.error||'Skin gagal disimpan')});
  else if(a.type==='add-bot')socket.emit('game-add-bot',{id:currentGame.id},ack('Tambah bot'));
  else if(a.type==='leave'){if(!await appConfirm({title:'Keluar dari game?',text:'Kamu akan keluar dari pertandingan ini.',ok:'Keluar'}))return;socket.emit('game-leave',{id:currentGame.id});currentGame=null;gameFeedback(true,'Keluar dari game.');}
  else if(a.type==='restart'){if(!await appConfirm({title:'Restart game?',text:'Pertandingan akan diulang dari awal. Pemain lain perlu menyetujui.',ok:'Minta Restart'}))return;socket.emit('game-restart-request',{id:currentGame.id},ack('Restart'));}
});
function gameMeIndex(g){return g.players.findIndex(p=>profileKeyLocal(p.name)===profileKeyLocal(cfg.name))}
function gameSig(g){return [g.id,g.status,g.turn,g.countdown,g.soloCountdown,g.dice,g.winner,JSON.stringify(g.board),JSON.stringify(g.ludo),JSON.stringify(g.snakesLadders),g.lastAction?.id||''].join('|')}
function renderSlitherFallback(g){$('gamePrimary').classList.add('hidden');const area=$('gameArea');if(!area.querySelector('#gameSlitherCanvas')){area.innerHTML='<div class="slither-shell"><div class="slither-hud"><b id="gameSlitherLength"></b><span>Gerakkan pointer untuk mengarahkan · tahan klik untuk boost</span></div><div class="slither-stage"><canvas id="gameSlitherCanvas" width="960" height="600"></canvas><div id="gameSlitherRank" class="slither-board"></div></div><button id="gameSlitherRespawn" class="primary wide">Main lagi</button></div>';const cv=$('gameSlitherCanvas'),send=(ev,boost=false)=>{const r=cv.getBoundingClientRect(),self=(currentGame?.slither?.entities||[]).find(e=>e.playerIndex===gameMeIndex(currentGame));if(!self)return;socket.emit('slither-input',{id:currentGame.id,angle:Math.atan2(ev.clientY-r.top-r.height/2,ev.clientX-r.left-r.width/2),boost});};cv.onpointermove=e=>send(e,false);cv.onpointerdown=e=>{cv.setPointerCapture(e.pointerId);send(e,true)};cv.onpointerup=e=>send(e,false);$('gameSlitherRespawn').onclick=()=>socket.emit('slither-respawn',{id:currentGame.id},r=>{if(r?.ok)currentGame=r.state;else showToast(r?.error||'Respawn gagal')});}const cv=$('gameSlitherCanvas');if(!cv)return;const world=g.slither||{},self=(world.entities||[]).find(e=>e.playerIndex===gameMeIndex(g)),ctx=cv.getContext('2d'),W=cv.width,H=cv.height,cx=self?.x||1200,cy=self?.y||850,sx=x=>(x-cx)*.8+W/2,sy=y=>(y-cy)*.8+H/2;ctx.fillStyle='#092019';ctx.fillRect(0,0,W,H);ctx.strokeStyle='#235441';for(let x=0;x<W;x+=52){ctx.beginPath();ctx.moveTo(x,0);ctx.lineTo(x,H);ctx.stroke()}for(let y=0;y<H;y+=52){ctx.beginPath();ctx.moveTo(0,y);ctx.lineTo(W,y);ctx.stroke()}for(const f of world.food||[]){ctx.fillStyle=f.color;ctx.shadowColor=f.color;ctx.shadowBlur=9;ctx.beginPath();ctx.arc(sx(f.x),sy(f.y),4,0,Math.PI*2);ctx.fill()}ctx.shadowBlur=0;for(const e of world.entities||[]){if(!e.alive)continue;ctx.strokeStyle=e.color;ctx.lineWidth=22;ctx.lineCap='round';ctx.beginPath();(e.body||[]).forEach((p,i)=>i?ctx.lineTo(sx(p.x),sy(p.y)):ctx.moveTo(sx(p.x),sy(p.y)));ctx.stroke();ctx.fillStyle=e.color;ctx.beginPath();ctx.arc(sx(e.x),sy(e.y),13,0,Math.PI*2);ctx.fill()}$('gameSlitherLength').textContent='Panjang '+Math.floor(self?.length||0);$('gameSlitherRank').innerHTML='<b>🏆 Peringkat</b>'+[...(world.entities||[])].sort((a,b)=>b.length-a.length).slice(0,6).map((e,i)=>`<span>${i+1} · ${esc(e.name)} <small>${Math.floor(e.length)}</small></span>`).join('');$('gameSlitherRespawn').classList.toggle('hidden',!!self?.alive);}
function renderSnakesLadders(g){$('gamePrimary').classList.add('hidden');const cell=48,ctxHolder=$('gameArea');ctxHolder.innerHTML='<div class="snakes-shell"><div class="snakes-board-wrap"><canvas id="appSnakesCanvas" width="480" height="480"></canvas></div><div class="game-info strong" id="appSnakesHelp"></div><button id="appSnakesRoll" class="primary">🎲 Lempar dadu</button></div>';const cv=$('appSnakesCanvas'),ctx=cv.getContext('2d'),xy=n=>{const v=n-1,row=Math.floor(v/10),col=v%10;return{x:(row%2?9-col:col)*cell+cell/2,y:(9-row)*cell+cell/2}};ctx.fillStyle='#eadbbd';ctx.fillRect(0,0,480,480);for(let n=1;n<=100;n++){const q=xy(n),x=q.x-cell/2,y=q.y-cell/2;ctx.fillStyle=(Math.floor((n-1)/10)+((n-1)%10))%2?'#d2bd96':'#f8ebd2';ctx.fillRect(x,y,cell,cell);ctx.strokeStyle='#947553';ctx.strokeRect(x,y,cell,cell);ctx.fillStyle='#604b38';ctx.font='11px Arial';ctx.fillText(String(n),x+4,y+12)}const map=g.snakesLadders?.board||{};for(const [a,b]of Object.entries(map.ladders||{})){const p=xy(+a),q=xy(+b);ctx.strokeStyle='#3eaf69';ctx.lineWidth=5;ctx.beginPath();ctx.moveTo(p.x-4,p.y);ctx.lineTo(q.x-4,q.y);ctx.moveTo(p.x+4,p.y);ctx.lineTo(q.x+4,q.y);ctx.stroke()}for(const[a,b]of Object.entries(map.snakes||{})){const p=xy(+a),q=xy(+b);ctx.strokeStyle='#df6975';ctx.lineWidth=8;ctx.beginPath();ctx.moveTo(p.x,p.y);ctx.quadraticCurveTo(p.x+35,q.y-15,q.x,q.y);ctx.stroke()}(g.snakesLadders?.positions||[]).forEach((pos,i)=>{if(!pos)return;const q=xy(pos);ctx.fillStyle=i?'#e15d6b':'#2cbd7a';ctx.beginPath();ctx.arc(q.x+(i?8:-8),q.y,9,0,Math.PI*2);ctx.fill()});const me=gameMeIndex(g),myTurn=g.status==='playing'&&g.turn===me;$('appSnakesHelp').textContent=g.status==='lobby'?'Menunggu lawan atau bot…':g.status==='finished'?`🏆 ${g.winner} menang`:myTurn?'Giliran kamu · harus tepat di kotak 100':`Giliran ${g.players[g.turn]?.name||'lawan'}`;$('appSnakesRoll').disabled=!myTurn;$('appSnakesRoll').onclick=()=>socket.emit('snakes-roll',{id:g.id,clientActionId:gameActionId()},r=>{if(!r?.ok)showToast(r?.error||'Lempar dadu gagal')});}
function renderGame(){const g=currentGame;if(!g)return;if(g.type==='slither'&&$('gameArea').querySelector('#gameSlitherCanvas')){$('gameTitle').textContent='🟢 Slither.io';$('gameStatus').textContent='Sedang berlangsung · room publik';$('gamePlayers').innerHTML=(g.players||[]).map(p=>`<div class="game-player joined">${avatarHtml(p.avatarUrl,p.name,'game-avatar')}<div><b>${esc(p.name)}</b><span>${p.isBot?'BOT':'PEMAIN'}</span></div></div>`).join('');renderSlitherFallback(g);return;}const sig=gameSig(g),actionChanged=g.lastAction?.id&&g.lastAction.id!==lastGameFxId;if(sig===gameRenderSignature&&!actionChanged)return;gameRenderSignature=sig;$('gameTitle').textContent=`${g.type==='chess'?'♟️':g.type==='slither'?'🟢':g.type==='snakes'?'🐍':'🎲'} ${gameName(g.type)}`;const isHost=profileKeyLocal(g.host)===profileKeyLocal(cfg.name),max=(g.type==='chess'||g.type==='snakes')?2:g.type==='slither'?12:4,joined=new Set((g.players||[]).map(p=>profileKeyLocal(p.name))),declined=new Set((g.declinedNames||[]).map(profileKeyLocal)),pending=(g.invitedNames||[]).filter(n=>!joined.has(profileKeyLocal(n)));const playersHtml=g.players.map((p,i)=>`<div class="game-player joined ${i===g.turn&&g.status==='playing'?'turn':''}">${avatarHtml(p.avatarUrl,p.name,'game-avatar')}<div><b>${esc(p.name)}</b><span>${p.isBot?`BOT · ${String(g.botLevel||'medium').toUpperCase()}`:profileKeyLocal(p.name)===profileKeyLocal(g.host)?'HOST · SIAP':'PEMAIN · SIAP'}</span></div><em>✓</em></div>`).join('');const pendingHtml=g.status==='lobby'?pending.map(name=>{const u=[...(onlineUsers||[]),...(editorDirectory||[])].find(x=>profileKeyLocal(x.name)===profileKeyLocal(name));const no=declined.has(profileKeyLocal(name));return`<div class="game-player invite-status ${no?'declined':'pending'}">${avatarHtml(u?.avatarUrl,name,'game-avatar')}<div><b>${esc(name)}</b><span>${no?'MENOLAK INVITE':'MENUNGGU JAWABAN'}</span></div><em>${no?'×':'…'}</em></div>`}).join(''):'';$('gamePlayers').innerHTML=playersHtml+pendingHtml;let status=g.status==='lobby'?(pending.length?`Lobby · menunggu ${pending.filter(n=>!declined.has(profileKeyLocal(n))).join(', ')||'pemain'}`:`Lobby · ${g.players.length}/${max} pemain`):g.status==='finished'?`Selesai · pemenang ${g.winner||'-'}`:`Sedang bermain · giliran ${g.players[g.turn]?.name||'-'}`;if(g.status==='lobby'&&g.soloCountdown>0)status=`Lobby bot · mulai otomatis dalam ${g.soloCountdown} detik`;if(g.type==='ludo'&&g.status==='lobby'&&g.countdown>0)status=`Mulai dalam ${g.countdown} detik · ${g.players.length}/4 pemain`;$('gameStatus').textContent=status;$('gameBot').classList.toggle('hidden',!(isHost&&g.status==='lobby'&&g.players.length<max));$('gameCancel').classList.toggle('hidden',!(isHost&&g.status==='lobby'));if(g.type==='chess')renderChess(g);else if(g.type==='snakes')renderSnakesLadders(g);else if(g.type==='slither')renderSlitherFallback(g);else renderLudo(g);if(actionChanged){const a=g.lastAction;lastGameFxId=a.id;setTimeout(()=>runGameFx(g,a),25)}if(g.status==='finished'&&g.winner)gameSound('win');}
function chessOwnerLocal(p){return p?(p===p.toUpperCase()?0:1):-1}
function chessPseudoMoves(board,from,player){const out=[];const p=board?.[from];if(!p||chessOwnerLocal(p)!==player)return out;const fr=Math.floor(from/8),fc=from%8,low=p.toLowerCase();for(let to=0;to<64;to++){if(to===from)continue;const tr=Math.floor(to/8),tc=to%8,dr=tr-fr,dc=tc-fc,ar=Math.abs(dr),ac=Math.abs(dc);if(board[to]&&chessOwnerLocal(board[to])===player)continue;let ok=false;const clear=(sr,sc)=>{let r=fr+sr,c=fc+sc;while(r!==tr||c!==tc){if(board[r*8+c])return false;r+=sr;c+=sc}return true};if(low==='p'){const dir=player===0?-1:1,start=player===0?6:1;ok=(dc===0&&!board[to]&&(dr===dir||(fr===start&&dr===2*dir&&!board[(fr+dir)*8+fc])))||(ac===1&&dr===dir&&!!board[to]);}else if(low==='n')ok=(ar===2&&ac===1)||(ar===1&&ac===2);else if(low==='k')ok=ar<=1&&ac<=1;else if(low==='r')ok=(dr===0||dc===0)&&clear(Math.sign(dr),Math.sign(dc));else if(low==='b')ok=ar===ac&&clear(Math.sign(dr),Math.sign(dc));else if(low==='q')ok=((dr===0||dc===0)||ar===ac)&&clear(Math.sign(dr),Math.sign(dc));if(ok)out.push(to)}return out}
const PIECES={K:'♔',Q:'♕',R:'♖',B:'♗',N:'♘',P:'♙',k:'♚',q:'♛',r:'♜',b:'♝',n:'♞',p:'♟'};
function gameSecondsLeft(g){return g?.turnDeadline?Math.max(0,Math.ceil((Number(g.turnDeadline)-Date.now())/1000)):0}function renderChess(g){stopPowerMeter();const me=gameMeIndex(g),order=me===1?[...Array(64).keys()].reverse():[...Array(64).keys()],legal=chessSelected!==null?chessPseudoMoves(g.board,chessSelected,me):[];$('gamePrimary').classList.add('hidden');$('gameArea').innerHTML=`<div class="chess-shell"><div class="game-info strong">${g.status==='lobby'?(g.soloCountdown>0?`Menunggu lawan ${g.soloCountdown} detik · atau pilih Lawan Bot`:'Menunggu lawan…'):g.status==='finished'?`🏆 Game selesai · ${esc(g.winner||'')}`:(g.turn===me?'Giliran kamu · pilih bidak':'Menunggu '+esc(g.players[g.turn]?.name||''))} ${g.status==='playing'?`<span class=\"turn-clock\">⏱ ${gameSecondsLeft(g)}s</span>`:''}</div><div class="chess-table-3d"><div class="chess-board-wrap"><div id="chessBoard" class="chess-board premium chess-board-3d"></div></div></div><div class="chess-help">Klik bidak → kotak tujuan. Titik = langkah tersedia, lingkaran merah = makan.</div></div>`;const b=$('chessBoard');order.forEach(i=>{const rr=Math.floor(i/8),cc=i%8,bt=document.createElement('button'),piece=g.board?.[i];bt.dataset.index=i;bt.className=`chess-cell ${(rr+cc)%2?'dark':'light'} ${chessSelected===i?'selected':''} ${legal.includes(i)?'legal':''} ${legal.includes(i)&&piece?'capture':''}`;bt.innerHTML=`<span class="piece ${piece&&piece===piece.toUpperCase()?'white-piece':'black-piece'}">${PIECES[piece]||''}</span>${legal.includes(i)?'<i></i>':''}<small class="coord">${cc===0?8-rr:''}${rr===7?'abcdefgh'[cc]:''}</small>`;bt.disabled=g.status!=='playing'||g.turn!==me;bt.onclick=()=>{if(chessSelected===null){if(piece&&chessOwnerLocal(piece)===me){gameSound('select');chessSelected=i;gameRenderSignature='';renderChess(g)}}else if(chessSelected===i){chessSelected=null;gameRenderSignature='';renderChess(g)}else if(legal.includes(i)){const from=chessSelected;chessSelected=null;socket.emit('chess-move',{id:g.id,from,to:i},res=>{if(!res?.ok){showToast(res?.error||'Langkah tidak valid');gameRenderSignature='';renderChess(g)}})}else if(piece&&chessOwnerLocal(piece)===me){gameSound('select');chessSelected=i;gameRenderSignature='';renderChess(g)}else{chessSelected=null;gameRenderSignature='';renderChess(g)}};b.appendChild(bt);});}
const LUDO_PATH=[[6,0],[6,1],[6,2],[6,3],[6,4],[6,5],[5,6],[4,6],[3,6],[2,6],[1,6],[0,6],[0,7],[0,8],[1,8],[2,8],[3,8],[4,8],[5,8],[6,9],[6,10],[6,11],[6,12],[6,13],[6,14],[7,14],[8,14],[8,13],[8,12],[8,11],[8,10],[8,9],[9,8],[10,8],[11,8],[12,8],[13,8],[14,8],[14,7],[14,6],[13,6],[12,6],[11,6],[10,6],[9,6],[8,5],[8,4],[8,3],[8,2],[8,1],[8,0],[7,0]];
const LUDO_START_BY_COLOR={red:0,green:13,yellow:26,blue:39};
function ludoColor(g,pi){return String(g?.players?.[pi]?.color||['red','green','yellow','blue'][pi]||'red').toLowerCase()}
function ludoGlobal(pos,pi,g=currentGame){if(pos<0||pos>=52)return -1;return(pos+(LUDO_START_BY_COLOR[ludoColor(g,pi)]??pi*13))%52}
function ludoLegalLocal(g,pi,d){const a=[];for(let k=0;k<4;k++){const p=g.ludo?.pawns?.[pi]?.[k]??-1;if(p<0){if(d===6)a.push(k)}else if(p<57&&p+d<=57)a.push(k)}return a}
let ludoClickPending=false,ludoClickPendingTimer=0;
function releaseLudoClickPending(){ludoClickPending=false;if(ludoClickPendingTimer){clearTimeout(ludoClickPendingTimer);ludoClickPendingTimer=0}}
function guardLudoClick(){if(ludoClickPending)return false;ludoClickPending=true;ludoClickPendingTimer=setTimeout(()=>{ludoClickPending=false;ludoClickPendingTimer=0;if(currentGame?.type==='ludo'&&!$('gameModal')?.classList.contains('hidden'))renderGame();},850);return true}
function stopPowerMeter(){if(powerRaf){cancelAnimationFrame(powerRaf);powerRaf=0}}
function startPowerMeter(){stopPowerMeter();const slider=$('ludoPowerSlider'),value=$('ludoPowerValue');if(!slider)return;let last=performance.now();const tick=now=>{if(!$('gameModal')||$('gameModal').classList.contains('hidden')||!currentGame||currentGame.type!=='ludo'){powerRaf=0;return}const dt=Math.min(40,now-last);last=now;if(now>powerManualUntil){ludoPower+=ludoPowerDir*dt*.075;if(ludoPower>=100){ludoPower=100;ludoPowerDir=-1}else if(ludoPower<=0){ludoPower=0;ludoPowerDir=1}slider.value=String(Math.round(ludoPower));if(value)value.textContent=Math.round(ludoPower)+'%'}powerRaf=requestAnimationFrame(tick)};slider.oninput=()=>{ludoPower=Number(slider.value)||0;powerManualUntil=performance.now()+1400;if(value)value.textContent=Math.round(ludoPower)+'%'};powerRaf=requestAnimationFrame(tick)}
function renderLudo(g){const me=gameMeIndex(g);if(g.status==='lobby'){stopPowerMeter();$('gamePrimary').classList.add('hidden');$('gameArea').innerHTML=`<div class="ludo-lobby"><div class="countdown-ring"><b>${g.countdown||g.soloCountdown||'…'}</b><span>${g.players.length>=2?'mulai':'tunggu'}</span></div><div class="game-info strong">${g.players.length<2?'Menunggu pemain lain. Bot otomatis masuk jika waktu habis.':'Pemain lain masih bisa join sampai countdown selesai.'}</div></div>`;return;}const legal=(g.turn===me&&g.dice)?ludoLegalLocal(g,me,g.dice):[];let grid='';for(let row=0;row<15;row++)for(let col=0;col<15;col++){let cls='ludo-board-cell';if(row<6&&col<6)cls+=' home-red';else if(row<6&&col>8)cls+=' home-green';else if(row>8&&col>8)cls+=' home-yellow';else if(row>8&&col<6)cls+=' home-blue';const pathIndex=LUDO_PATH.findIndex(([r,c])=>r===row&&c===col);if(pathIndex>=0)cls+=' path-cell';if(row>=6&&row<=8&&col>=6&&col<=8)cls+=' center-cell';grid+=`<div class="${cls}" data-r="${row}" data-c="${col}" ${pathIndex>=0?`data-track="${pathIndex}"`:''}></div>`;}const myTurn=g.status==='playing'&&g.turn===me&&!g.dice;$('gameArea').innerHTML=`<div class="ludo-shell"><div class="ludo-top"><div class="dice-stage"><div id="diceCube" class="dice-cube show-${g.dice||1}"><i class="face f1">⚀</i><i class="face f2">⚁</i><i class="face f3">⚂</i><i class="face f4">⚃</i><i class="face f5">⚄</i><i class="face f6">⚅</i></div><small>${g.dice?'HASIL '+g.dice:'DADU'}</small></div><div class="game-info strong">${g.status==='finished'?`🏆 ${esc(g.winner||'')}`:(g.turn===me?(g.dice?(legal.length?'Pilih pion yang menyala':'Tidak ada langkah · giliran akan lewat otomatis'):'Atur power lalu lempar dadu'):'Giliran '+esc(g.players[g.turn]?.name||''))}</div></div>${myTurn?`<div class="ludo-power"><span>Pelan</span><input id="ludoPowerSlider" type="range" min="0" max="100" value="${Math.round(ludoPower)}"><b id="ludoPowerValue">${Math.round(ludoPower)}%</b><span>Kuat</span></div>`:''}<div class="ludo-table-3d"><div id="ludoBoard" class="ludo-board ludo-board-3d">${grid}</div></div><div id="ludoBench" class="ludo-bench"></div></div>`;for(let pi=0;pi<g.players.length;pi++)for(let k=0;k<4;k++){const pos=g.ludo.pawns[pi][k],pawn=document.createElement('button');pawn.dataset.player=pi;pawn.dataset.pawn=k;const pColor=ludoColor(g,pi);pawn.className=`ludo-piece p${pi} ludo-color-${pColor} ${pi===me&&legal.includes(k)?'movable':''}`;pawn.textContent=String(k+1);pawn.title=`${g.players[pi].name} · pion ${k+1}`;if(pi===me&&legal.includes(k))pawn.onclick=()=>{if(!guardLudoClick())return;socket.emit('ludo-move',{id:g.id,pawn:k,clientActionId:gameActionId()},res=>{if(!res?.ok){releaseLudoClickPending();showToast(res?.error||'Tidak bisa jalan');renderGame();}})};const glob=ludoGlobal(pos,pi,g);if(glob>=0){const coord=LUDO_PATH[glob],cell=$('ludoBoard').querySelector(`[data-r="${coord[0]}"][data-c="${coord[1]}"]`);cell?.appendChild(pawn)}else{let bench=$(`bench${pi}`);if(!bench){bench=document.createElement('div');bench.id=`bench${pi}`;bench.className=`bench-player p${pi}`;bench.innerHTML=`<b>${esc(g.players[pi].name)}</b><div></div>`;$('ludoBench').appendChild(bench)}bench.querySelector('div').appendChild(pawn);}}$('gamePrimary').classList.toggle('hidden',!myTurn);$('gamePrimary').textContent='🎲 Lempar Dadu';$('gamePrimary').onclick=()=>{if(!guardLudoClick())return;const p=Math.round(ludoPower);stopPowerMeter();socket.emit('ludo-roll',{id:g.id,power:p,clientActionId:gameActionId()},res=>{if(!res?.ok){releaseLudoClickPending();showToast(res?.error||'Gagal lempar dadu');startPowerMeter();renderGame()}})};if(myTurn)startPowerMeter();else stopPowerMeter();}
function runGameFx(g,a){if(!a)return;if(a.type==='chess-move'){const board=$('chessBoard'),from=board?.querySelector(`[data-index="${a.from}"]`),to=board?.querySelector(`[data-index="${a.to}"]`);if(from&&to){const br=board.getBoundingClientRect(),fr=from.getBoundingClientRect(),tr=to.getBoundingClientRect(),ghost=document.createElement('span');ghost.className=`chess-fx-piece ${a.piece===String(a.piece).toUpperCase()?'white-piece':'black-piece'}`;ghost.textContent=PIECES[a.piece]||'';ghost.style.left=(fr.left-br.left)+'px';ghost.style.top=(fr.top-br.top)+'px';ghost.style.width=fr.width+'px';ghost.style.height=fr.height+'px';board.appendChild(ghost);to.classList.add('anim-target');requestAnimationFrame(()=>requestAnimationFrame(()=>{ghost.style.transform=`translate3d(${tr.left-fr.left}px,${tr.top-fr.top}px,32px) scale(1.08)`}));setTimeout(()=>{ghost.remove();to.classList.remove('anim-target');to.classList.add('piece-land');setTimeout(()=>to.classList.remove('piece-land'),260)},330);if(a.captured){to.classList.add('capture-hit');setTimeout(()=>to.classList.remove('capture-hit'),420);gameSound('capture')}else gameSound('chess-move')}}else if(a.type==='ludo-roll'){const cube=$('diceCube');if(cube){cube.classList.add('rolling');setTimeout(()=>cube.classList.remove('rolling'),620)}gameSound('dice',a.power)}else if(a.type==='ludo-move'){const target=$('ludoBoard')?.querySelector(`[data-player="${a.player}"][data-pawn="${a.pawn}"]`)||document.querySelector(`[data-player="${a.player}"][data-pawn="${a.pawn}"]`);const board=$('ludoBoard');if(target&&board){let source=null;if(a.from>=0&&a.from<52){const glob=ludoGlobal(a.from,a.player,g),c=LUDO_PATH[glob];source=board.querySelector(`[data-r="${c[0]}"][data-c="${c[1]}"]`)}const tr=target.getBoundingClientRect();if(source){const sr=source.getBoundingClientRect();target.style.transition='none';target.style.transform=`translate3d(${sr.left-tr.left}px,${sr.top-tr.top}px,34px) scale(1.18)`;requestAnimationFrame(()=>requestAnimationFrame(()=>{target.style.transition='transform .42s cubic-bezier(.2,.85,.3,1.2)';target.style.transform='translate3d(0,0,18px) scale(1)'}));setTimeout(()=>{target.style.transition='';target.style.transform=''},500)}else{target.classList.add('pawn-enter');setTimeout(()=>target.classList.remove('pawn-enter'),480)}const glob=ludoGlobal(a.to,a.player,g);if(glob>=0){const c=LUDO_PATH[glob],cell=board.querySelector(`[data-r="${c[0]}"][data-c="${c[1]}"]`);if(a.captured?.length){cell?.classList.add('capture-hit');setTimeout(()=>cell?.classList.remove('capture-hit'),500);gameSound('capture')}else gameSound('pawn')}}}}


// v1.3.1 — Crop Kepala
let headCropImg=null,headCropDataUrl='',headFace=null,headFaces=[],headFaceIndex=0,headCropFiles=[],headCropFileIndex=0,headCropOutputDir='';
let headCropPreviewToken=0;
function headVals(){return{x:Number($('headCropX').value)/100,y:Number($('headCropY').value)/100,zoom:Number($('headCropZoom').value)/100,pad:Number($('headCropPad')?.value||175)/100};}
function getHeadCropRect(){if(!headCropImg)return null;const iw=headCropImg.naturalWidth,ih=headCropImg.naturalHeight,v=headVals(),headOnly=$('headCropHeadOnly')?.checked!==false;let base=Math.min(iw,ih)/v.zoom,cx=v.x*iw,cy=v.y*ih;if(headFaces[headFaceIndex]){const b=headFaces[headFaceIndex].boundingBox||headFaces[headFaceIndex];cx=b.x+b.width/2;if(headOnly){base=Math.max(b.width*1.62,b.height*1.48);cy=b.y+b.height*0.27;}else{base=Math.max(b.width*1.55,b.height*v.pad);cy=b.y+b.height*0.36;}}base=Math.min(base,iw,ih);const left=Math.max(0,Math.min(iw-base,cx-base/2)),top=Math.max(0,Math.min(ih-base,cy-base/2));return{left,top,base,headOnly};}
function applyHeadOnlyBottomMask(canvas){if(!$('headCropHeadOnly')?.checked)return;const ctx=canvas.getContext('2d'),w=canvas.width,h=canvas.height;ctx.save();ctx.globalCompositeOperation='destination-out';const g=ctx.createLinearGradient(0,h*.82,0,h);g.addColorStop(0,'rgba(0,0,0,0)');g.addColorStop(.55,'rgba(0,0,0,.18)');g.addColorStop(.82,'rgba(0,0,0,.78)');g.addColorStop(1,'rgba(0,0,0,1)');ctx.fillStyle=g;ctx.fillRect(0,h*.80,w,h*.20);ctx.restore();}
async function updateHeadCropResultPreview(){if(!headCropImg)return;const token=++headCropPreviewToken;const out=$('headCropResult'),tmp=document.createElement('canvas'),rect=getHeadCropRect();if(!rect)return;tmp.width=512;tmp.height=512;const tx=tmp.getContext('2d');tx.clearRect(0,0,512,512);tx.drawImage(headCropImg,rect.left,rect.top,rect.base,rect.base,0,0,512,512);if(!$('headCropRemoveBg').checked){const o=out.getContext('2d');o.clearRect(0,0,out.width,out.height);o.drawImage(tmp,0,0,out.width,out.height);applyHeadOnlyBottomMask(out);$('saveHeadCrop').disabled=false;if($('sendHeadCrop'))$('sendHeadCrop').disabled=false;$('saveAllHeadCrop').disabled=!headFaces.length;return;}try{$('headCropStatus').textContent='Menghapus background kepala terpilih...';const base64=tmp.toDataURL('image/png').split(',')[1],res=await window.kc.aiRemoveBackground({base64,mime:'image/png',model:'large',name:'head-crop.png',computeMode:$('headCropCompute')?.value||'auto',parallel:!!$('headCropParallel')?.checked});if(token!==headCropPreviewToken)return;if(!res?.ok)throw new Error(res?.error||'Remove BG gagal');const img=new Image();img.src='data:image/png;base64,'+res.base64;await img.decode();const o=out.getContext('2d');o.clearRect(0,0,out.width,out.height);o.drawImage(img,0,0,out.width,out.height);applyHeadOnlyBottomMask(out);$('headCropStatus').textContent=`Siap - ${headFaces.length||1} kepala${$('headCropRemoveBg').checked?' - transparan':''}${$('headCropHeadOnly')?.checked?' - kepala saja':''}.`;$('saveHeadCrop').disabled=false;if($('sendHeadCrop'))$('sendHeadCrop').disabled=false;$('saveAllHeadCrop').disabled=!headFaces.length;}catch(e){if(token!==headCropPreviewToken)return;const o=out.getContext('2d');o.clearRect(0,0,out.width,out.height);o.drawImage(tmp,0,0,out.width,out.height);applyHeadOnlyBottomMask(out);$('headCropStatus').textContent='Preview remove bg gagal: '+e.message;$('saveHeadCrop').disabled=false;if($('sendHeadCrop'))$('sendHeadCrop').disabled=false;$('saveAllHeadCrop').disabled=!headFaces.length;}}
function renderHeadFaceList(){const box=$('headFaceList');if(!box)return;box.innerHTML='';if(!headFaces.length){box.innerHTML='<div class="storage-note">Belum ada kepala terdeteksi.</div>';return;}headFaces.forEach((f,i)=>{const item=document.createElement('button');item.className='head-face-item'+(i===headFaceIndex?' active':'');item.innerHTML=`<canvas width="80" height="80"></canvas><div><b>Kepala ${i+1}</b><span>${Math.round((f.boundingBox||f).width)}x${Math.round((f.boundingBox||f).height)} px</span></div>`;item.onclick=()=>{headFaceIndex=i;renderHeadFaceList();drawHeadCrop();};box.appendChild(item);const cv=item.querySelector('canvas'),x=cv.getContext('2d'),b=f.boundingBox||f;x.drawImage(headCropImg,b.x,b.y,b.width,b.height,0,0,80,80);});}
function drawHeadCrop(){if(!headCropImg)return;const c=$('headCropCanvas'),ctx=c.getContext('2d'),iw=headCropImg.naturalWidth,ih=headCropImg.naturalHeight,scale=Math.min(c.width/iw,c.height/ih),dw=iw*scale,dh=ih*scale,ox=(c.width-dw)/2,oy=(c.height-dh)/2;ctx.clearRect(0,0,c.width,c.height);ctx.drawImage(headCropImg,ox,oy,dw,dh);headFaces.forEach((f,i)=>{const b=f.boundingBox||f;ctx.save();ctx.strokeStyle=i===headFaceIndex?'#2d7dff':'rgba(255,255,255,.65)';ctx.lineWidth=i===headFaceIndex?4:2;ctx.setLineDash(i===headFaceIndex?[10,7]:[6,6]);ctx.strokeRect(ox+b.x*scale,oy+b.y*scale,b.width*scale,b.height*scale);ctx.restore();});const rect=getHeadCropRect();if(rect){ctx.save();ctx.strokeStyle='#439cff';ctx.lineWidth=4;ctx.setLineDash([12,8]);ctx.strokeRect(ox+rect.left*scale,oy+rect.top*scale,rect.base*scale,rect.base*scale);ctx.restore();}renderHeadFaceList();updateHeadCropResultPreview();}
async function autoDetectHead(){if(!headCropImg)return;try{$('headCropStatus').textContent='Mendeteksi beberapa kepala...';headFaces=[];if('FaceDetector' in window){const fd=new FaceDetector({fastMode:false,maxDetectedFaces:12}),faces=await fd.detect(headCropImg);if(faces?.length){headFaces=faces.sort((a,b)=>((a.boundingBox?.y||a.y)-(b.boundingBox?.y||b.y))||((a.boundingBox?.x||a.x)-(b.boundingBox?.x||b.x)));headFaceIndex=0;const b=headFaces[0].boundingBox||headFaces[0],iw=headCropImg.naturalWidth,ih=headCropImg.naturalHeight;$('headCropX').value=Math.round(((b.x+b.width/2)/iw)*100);$('headCropY').value=Math.round(((b.y+b.height*0.36)/ih)*100);$('headCropZoom').value=150;$('headCropStatus').textContent=`Terdeteksi ${headFaces.length} kepala. Klik daftar kanan untuk memilih kepala.`;drawHeadCrop();return;}}$('headCropStatus').textContent='Deteksi kepala tidak menemukan wajah. Gunakan slider manual.';}catch(e){$('headCropStatus').textContent='Deteksi kepala gagal. Gunakan slider manual.';}drawHeadCrop();}
async function loadHeadCropImage(p){const pv=await window.kc.readLocalImagePreview(p);if(!pv?.ok){showToast(pv?.error||'Gagal membaca gambar');return false;}headCropDataUrl=pv.dataUrl;const im=new Image();im.src=headCropDataUrl;await im.decode();headCropImg=im;headFaces=[];headFaceIndex=0;$('headCropX').value=50;$('headCropY').value=30;$('headCropZoom').value=150;if($('headCropPad'))$('headCropPad').value=175;drawHeadCrop();await autoDetectHead();return true;}
async function saveHeadCropCurrent(nameBase='KC-crop-kepala'){const data=$('headCropResult').toDataURL('image/png').split(',')[1];if(headCropOutputDir){const idx=(headFaceIndex+1).toString().padStart(2,'0');const out=(headCropOutputDir.replace(/[\\/]$/,''))+'/'+nameBase+'_head'+idx+'.png';const r=await window.kc.writeBase64File({path:out,base64:data});if(!r?.ok)throw new Error(r?.error||'Gagal simpan file');return r.path;}const r=await window.kc.savePng({name:nameBase+'.png',base64:data});if(!r?.ok)throw new Error(r?.error||'Gagal simpan');return r.path||'';}
$('headCropSidebar').onclick=()=>$('headCropModal').classList.remove('hidden');$('closeHeadCrop').onclick=()=>$('headCropModal').classList.add('hidden');
if($('chooseHeadCropOutput'))$('chooseHeadCropOutput').onclick=async()=>{const p=await window.kc.chooseVectorOutput();if(p){headCropOutputDir=p;showToast('Folder output Crop Kepala dipilih.');}};
$('chooseHeadCropImage').onclick=async()=>{const paths=await window.kc.chooseImageFiles({multiple:true});if(!paths?.length)return;headCropFiles=paths;headCropFileIndex=0;if($('headCropBatch'))$('headCropBatch').checked=paths.length>1;await loadHeadCropImage(paths[0]);if(paths.length>1)$('headCropStatus').textContent=`Batch siap: ${paths.length} foto. Foto 1 dibuka.`;};
$('autoHeadCrop').onclick=autoDetectHead;['headCropX','headCropY','headCropZoom','headCropPad'].forEach(id=>$(id)&&($(id).oninput=drawHeadCrop));if($('headCropRemoveBg'))$('headCropRemoveBg').onchange=drawHeadCrop;if($('headCropHeadOnly'))$('headCropHeadOnly').onchange=drawHeadCrop;
if($('sendHeadCrop'))$('sendHeadCrop').onclick=()=>{prepareCanvasForChat($('headCropResult'),'KC-crop-kepala.png');$('headCropModal').classList.add('hidden');};
$('saveHeadCrop').onclick=async()=>{if(!headCropImg)return;try{const base='KC-'+((headCropFiles[headCropFileIndex]||'crop').split(/[\\/]/).pop().replace(/\.[^.]+$/,''));await saveHeadCropCurrent(base);showToast('Crop kepala tersimpan.');}catch(e){showToast('Gagal simpan crop kepala: '+e.message);}};
if($('saveAllHeadCrop'))$('saveAllHeadCrop').onclick=async()=>{if(!headCropFiles.length)return;const __job=jobStart('head-crop',`Crop Kepala Batch`,Math.max(1,headCropFiles.length));if(!headCropOutputDir){const p=await window.kc.chooseVectorOutput();if(!p)return;headCropOutputDir=p;}let totalSaved=0;for(let fi=0;fi<headCropFiles.length;fi++){headCropFileIndex=fi;await loadHeadCropImage(headCropFiles[fi]);if(!headFaces.length){await saveHeadCropCurrent('KC-'+headCropFiles[fi].split(/[\\/]/).pop().replace(/\.[^.]+$/,'')+'_manual');totalSaved++;continue;}for(let hi=0;hi<headFaces.length;hi++){headFaceIndex=hi;renderHeadFaceList();drawHeadCrop();await new Promise(r=>setTimeout(r,180));const base='KC-'+headCropFiles[fi].split(/[\\/]/).pop().replace(/\.[^.]+$/,'');await saveHeadCropCurrent(base);totalSaved++;$('headCropStatus').textContent=`Menyimpan batch kepala... ${totalSaved} file`;}}$('headCropStatus').textContent=`Selesai batch crop kepala - ${totalSaved} file.`;jobDone(__job,true,`${totalSaved} file`);showToast(`Crop kepala batch selesai (${totalSaved} file).`);};
let vectorInputPath='',vectorOutputDir='',vectorResultPath='',vectorInputPaths=[];
function setVectorProgress(pct,text){const n=Math.max(0,Math.min(100,Number(pct)||0));$('vectorProgress').classList.remove('hidden');$('vectorProgressBar').value=n;$('vectorProgressPct').textContent=Math.round(n)+'%';$('vectorProgressText').textContent=text||'Memproses...';}
async function openVectorizer(){$('vectorModal').classList.remove('hidden');try{const s=await window.kc.vectorizerStatus();$('vectorStatus').textContent=s?.ok?'Engine Potrace siap, proses lokal/offline.':'Warning: '+(s?.error||'Vectorizer belum siap.');}catch(e){$('vectorStatus').textContent='Warning: '+e.message;}}
$('vectorSidebar').onclick=openVectorizer;$('closeVector').onclick=()=>$('vectorModal').classList.add('hidden');
$('chooseVectorImage').onclick=async()=>{const paths=await window.kc.chooseImageFiles({multiple:true});if(!paths?.length)return;vectorInputPaths=paths;vectorInputPath=paths[0];if($('vectorBatch'))$('vectorBatch').checked=paths.length>1;const pv=await window.kc.readLocalImagePreview(vectorInputPath);$('vectorInputPreview').src=pv?.ok?pv.dataUrl:'file:///'+vectorInputPath.replace(/\\/g,'/');$('vectorStatus').textContent=paths.length>1?`Batch siap: ${paths.length} gambar.`:'Input: '+vectorInputPath;$('vectorResultPreview').removeAttribute('src');vectorResultPath='';$('openVectorResult').disabled=true;$('openVectorFolder').disabled=true;if($('sendVectorResult'))$('sendVectorResult').disabled=true;if($('runVectorBatch'))$('runVectorBatch').classList.toggle('hidden',paths.length<2);};
$('chooseVectorOutput').onclick=async()=>{const p=await window.kc.chooseVectorOutput();if(p){vectorOutputDir=p;showToast('Folder output Vectorizer dipilih.');}};
async function vectorizeOne(inputPath,index,total){const __job=jobStart('vectorizer',`Vectorize ${inputPath.split(/[\/]/).pop()}`,1);setVectorProgress(1,`Mulai vectorize ${index}/${total}...`);const r=await window.kc.vectorizeImage({input:inputPath,outputDir:vectorOutputDir,preset:$('vectorPreset').value,outputFormat:$('vectorOutputFormat').value,backgroundMode:$('vectorBackgroundMode').value,computeMode:$('vectorCompute')?.value||'auto',parallel:!!$('vectorParallel')?.checked});if(!r?.ok)throw new Error(r?.error||'Vectorizer gagal');vectorResultPath=r.output;if(String(r.outputFormat||'').toLowerCase()!=='svg'){try{const rf=await window.kc.readFile(r.output);if(rf?.base64)clipboardResultBase64=rf.base64;}catch{}}$('vectorResultPreview').src='file:///'+r.output.replace(/\\/g,'/');$('vectorStatus').textContent=`Selesai ${index}/${total} - ${r.presetName||r.preset} - ${String(r.outputFormat||$('vectorOutputFormat').value).toUpperCase()}${r.width?` - ${r.width}x${r.height}px`:''}${r.backend?` · ${r.backend}`:''}`;setVectorProgress(100,`Vector selesai ${index}/${total}`);jobDone(__job,true,'Selesai');return r;}
$('runVectorizer').onclick=async()=>{if(!vectorInputPath){showToast('Pilih gambar bitmap dulu.');return;}$('runVectorizer').disabled=true;try{await vectorizeOne(vectorInputPath,1,1);$('openVectorResult').disabled=false;$('openVectorFolder').disabled=false;if($('sendVectorResult'))$('sendVectorResult').disabled=false;}catch(e){reportBug('vectorizer-ui-failed',e,'error',{preset:$('vectorPreset').value});$('vectorStatus').textContent='Vectorizer gagal: '+e.message;setVectorProgress(0,'Gagal');}finally{$('runVectorizer').disabled=false;}};
if($('runVectorBatch'))$('runVectorBatch').onclick=async()=>{if(vectorInputPaths.length<2){showToast('Pilih minimal 2 gambar untuk batch.');return;}if(!vectorOutputDir){const p=await window.kc.chooseVectorOutput();if(!p)return;vectorOutputDir=p;}$('runVectorBatch').disabled=true;let ok=0,fail=0;const errors=[];for(let i=0;i<vectorInputPaths.length;i++){try{await vectorizeOne(vectorInputPaths[i],i+1,vectorInputPaths.length);ok++;}catch(e){fail++;errors.push(`${vectorInputPaths[i].split(/[\/]/).pop()}: ${e.message}`);$('vectorStatus').textContent=`Batch vector ${i+1}/${vectorInputPaths.length} · gagal ${fail}, lanjut...`;}}$('vectorBatchSummary').textContent=`Selesai: ${ok} sukses · ${fail} gagal`;$('vectorBatchSummary').className='batch-summary '+(fail?'warn':'ok');$('vectorStatus').textContent=`Batch vector selesai - ${ok} sukses / ${fail} gagal.`;showToast(`Batch vector selesai (${ok} sukses, ${fail} gagal).`);$('openVectorFolder').disabled=false;$('runVectorBatch').disabled=false;if(errors.length)reportBug('vector-batch-partial-failure',new Error(errors.slice(0,10).join(' | ')),'warning',{ok,fail,total:vectorInputPaths.length});};
if(window.kc?.onVectorProgress)window.kc.onVectorProgress(d=>setVectorProgress(d?.pct,d?.text));
$('openVectorResult').onclick=()=>vectorResultPath&&window.kc.openPath(vectorResultPath);if($('sendVectorResult'))$('sendVectorResult').onclick=async()=>{await prepareLocalResultForChat(vectorResultPath);$('vectorModal').classList.add('hidden');};$('openVectorFolder').onclick=()=>{const target=vectorResultPath?vectorResultPath.replace(/[\\/][^\\/]+$/,''):vectorOutputDir;if(target)window.kc.openPath(target);};
let upscaleInputPath='',upscaleOutputDir='',upscaleResultPath='',workTagMode='',workTagConversationId='',upscaleInputPaths=[];
function setUpscaleProgress(pct,text){const n=Math.max(0,Math.min(100,Number(pct)||0));$('upscaleProgress').classList.remove('hidden');$('upscaleProgressBar').value=n;$('upscaleProgressPct').textContent=Math.round(n)+'%';$('upscaleProgressText').textContent=text||'Memproses...';}
async function openUpscale(){$('upscaleModal').classList.remove('hidden');try{const s=await window.kc.upscaleStatus();$('upscaleStatus').textContent=s?.ok?'Engine Upscayl siap, proses lokal memakai GPU Vulkan.':'Warning: '+(s?.error||'Engine Upscayl belum ditemukan.');}catch{}}
$('upscaleSidebar').onclick=openUpscale;$('closeUpscale').onclick=()=>$('upscaleModal').classList.add('hidden');
$('chooseUpscaleImage').onclick=async()=>{const paths=await window.kc.chooseImageFiles({multiple:true});if(!paths?.length)return;upscaleInputPaths=paths;upscaleInputPath=paths[0];if($('upscaleBatch'))$('upscaleBatch').checked=paths.length>1;const pv=await window.kc.readLocalImagePreview(upscaleInputPath);$('upscalePreview').src=pv?.ok?pv.dataUrl:'file:///'+upscaleInputPath.replace(/\\/g,'/');$('upscaleStatus').textContent=paths.length>1?`Batch siap: ${paths.length} gambar.`:'Input: '+upscaleInputPath;$('upscaleResultPreview').removeAttribute('src');upscaleResultPath='';$('openUpscaleResult').disabled=true;$('openUpscaleFolder').disabled=true;if($('sendUpscaleResult'))$('sendUpscaleResult').disabled=true;if($('runUpscaleBatch'))$('runUpscaleBatch').classList.toggle('hidden',paths.length<2);};
$('chooseUpscaleOutput').onclick=async()=>{const p=await window.kc.chooseUpscaleOutput();if(p){upscaleOutputDir=p;showToast('Folder output dipilih.');}};
async function upscaleOne(inputPath,index,total){const __job=jobStart('upscale',`Upscale ${inputPath.split(/[\/]/).pop()}`,1);setUpscaleProgress(1,`Mulai upscale ${index}/${total}...`);const r=await window.kc.upscaleImage({input:inputPath,outputDir:upscaleOutputDir,model:$('upscaleModel').value,scale:$('upscaleScale').value,computeMode:$('upscaleCompute')?.value||'auto',parallel:!!$('upscaleParallel')?.checked});if(!r?.ok)throw new Error(r?.error||'Upscale gagal');upscaleResultPath=r.output;try{const rf=await window.kc.readFile(r.output);if(rf?.base64)clipboardResultBase64=rf.base64;}catch{}const pv=await window.kc.readLocalImagePreview(r.output);$('upscaleResultPreview').src=pv?.ok?pv.dataUrl:'file:///'+r.output.replace(/\\/g,'/');$('upscaleStatus').textContent=`Selesai ${index}/${total} - ${r.model} - ${r.scale}x${r.width?' - '+r.width+'x'+r.height+'px':''}${r.inputWidth?` · dari ${r.inputWidth}x${r.inputHeight}`:''}${r.detailPass?' · detail pass aktif':''}${r.backend?` · ${r.backend}`:''}`;$('openUpscaleResult').disabled=false;$('openUpscaleFolder').disabled=false;if($('sendUpscaleResult'))$('sendUpscaleResult').disabled=false;setUpscaleProgress(100,`Upscale selesai ${index}/${total}`);jobDone(__job,true,'Selesai');return r;}
$('runUpscale').onclick=async()=>{if(!upscaleInputPath){showToast('Pilih gambar dulu.');return;}$('runUpscale').disabled=true;try{await upscaleOne(upscaleInputPath,1,1);}catch(e){reportBug('upscale-ui-failed',e,'error',{model:$('upscaleModel').value,scale:$('upscaleScale').value,computeMode:$('upscaleCompute')?.value||'auto',parallel:!!$('upscaleParallel')?.checked});$('upscaleStatus').textContent='Upscale gagal: '+e.message;setUpscaleProgress(0,'Gagal');}finally{$('runUpscale').disabled=false;}};
if($('runUpscaleBatch'))$('runUpscaleBatch').onclick=async()=>{if(upscaleInputPaths.length<2){showToast('Pilih minimal 2 gambar untuk batch.');return;}if(!upscaleOutputDir){const p=await window.kc.chooseUpscaleOutput();if(!p)return;upscaleOutputDir=p;}$('runUpscaleBatch').disabled=true;let ok=0,fail=0;const errors=[];for(let i=0;i<upscaleInputPaths.length;i++){try{await upscaleOne(upscaleInputPaths[i],i+1,upscaleInputPaths.length);ok++;}catch(e){fail++;errors.push(`${upscaleInputPaths[i].split(/[\/]/).pop()}: ${e.message}`);$('upscaleStatus').textContent=`Batch upscale ${i+1}/${upscaleInputPaths.length} · gagal ${fail}, lanjut...`;}}$('upscaleBatchSummary').textContent=`Selesai: ${ok} sukses · ${fail} gagal`;$('upscaleBatchSummary').className='batch-summary '+(fail?'warn':'ok');$('upscaleStatus').textContent=`Batch upscale selesai - ${ok} sukses / ${fail} gagal.`;showToast(`Batch upscale selesai (${ok} sukses, ${fail} gagal).`);$('runUpscaleBatch').disabled=false;if(errors.length)reportBug('upscale-batch-partial-failure',new Error(errors.slice(0,10).join(' | ')),'warning',{ok,fail,total:upscaleInputPaths.length});};
if(window.kc?.onUpscaleProgress)window.kc.onUpscaleProgress(d=>setUpscaleProgress(d?.pct,d?.text));
$('openUpscaleResult').onclick=()=>upscaleResultPath&&window.kc.openPath(upscaleResultPath);if($('sendUpscaleResult'))$('sendUpscaleResult').onclick=async()=>{await prepareLocalResultForChat(upscaleResultPath);$('upscaleModal').classList.add('hidden');};$('openUpscaleFolder').onclick=()=>{const target=upscaleResultPath?upscaleResultPath.replace(/[\\/][^\\/]+$/,''):upscaleOutputDir;if(target)window.kc.openPath(target);};
let removeBgFile=null,removeBgOutputFile=null,removeBgObjectUrl='',removeBgFiles=[],removeBgOutputDir='';
const RB_EXTS=['png','jpg','jpeg','webp','bmp','tif','tiff','gif','ico','heic','heif','avif'];
function rbSetStatus(text){const el=$('removeBgHint');if(el)el.textContent=text;}
function rbResetResult(){removeBgOutputFile=null;$('saveRemoveBg').disabled=true;$('sendRemoveBg').disabled=true;const c=$('removeBgResult');if(c)c.getContext('2d').clearRect(0,0,c.width,c.height);}
function rbFitCanvas(canvas,w,h,maxSide=1400){const scale=Math.min(1,maxSide/Math.max(w,h));canvas.width=Math.max(1,Math.round(w*scale));canvas.height=Math.max(1,Math.round(h*scale));}
function rbProgress(pct,text){const box=$('removeBgProgress');if(!box)return;box.classList.remove('hidden');const n=Math.max(0,Math.min(100,Number(pct)||0));$('removeBgProgressBar').value=n;$('removeBgProgressPct').textContent=Math.round(n)+'%';$('removeBgProgressText').textContent=text||'Memproses AI…';}
function rbIsSupported(file){const type=String(file?.type||'').toLowerCase();const ext=((file?.name||'').split('.').pop()||'').toLowerCase();return /^image\//.test(type)||RB_EXTS.includes(ext);}
async function rbRenderPreviewFromBlob(blob){if(removeBgObjectUrl)URL.revokeObjectURL(removeBgObjectUrl);removeBgObjectUrl=URL.createObjectURL(blob);const img=new Image();img.decoding='async';img.src=removeBgObjectUrl;await img.decode();const c=$('removeBgSource');rbFitCanvas(c,img.naturalWidth,img.naturalHeight);const x=c.getContext('2d');x.clearRect(0,0,c.width,c.height);x.drawImage(img,0,0,c.width,c.height);const r=$('removeBgResult');r.width=c.width;r.height=c.height;return {w:img.naturalWidth,h:img.naturalHeight};}
async function rbLoadFile(file){if(!file||!rbIsSupported(file)){rbSetStatus('Format didukung: PNG, JPG/JPEG, WEBP, BMP, TIFF/TIF, GIF, ICO, HEIC/HEIF, dan AVIF.');return;}if(file.size>80*1024*1024){rbSetStatus('Gambar terlalu besar untuk Remove BG AI. Maksimal 80 MB.');return;}removeBgFile=file;rbResetResult();$('removeBgProgress')?.classList.add('hidden');try{const info=await rbRenderPreviewFromBlob(file);rbSetStatus(`Siap AI · ${info.w}×${info.h}px · ${(file.size/1024/1024).toFixed(1)} MB.`);}catch(_){try{const base64=await rbFileToBase64(file),preview=await window.kc.aiPreviewImage({base64,mime:file.type||'',name:file.name||''});if(!preview?.ok)throw new Error(preview?.error||'Preview gagal');const blob=await (await kcFetch('data:image/png;base64,'+preview.base64)).blob();const info=await rbRenderPreviewFromBlob(blob);rbSetStatus(`Siap AI · preview dinormalisasi · ${info.w}×${info.h}px · ${(file.size/1024/1024).toFixed(1)} MB.`);}catch(err){rbSetStatus('Gagal membuat preview: '+(err.message||err));}}}
function rbFileToBase64(file){return new Promise((resolve,reject)=>{const rd=new FileReader();rd.onerror=()=>reject(rd.error||new Error('Gagal membaca gambar'));rd.onload=()=>resolve(String(rd.result||'').split(',')[1]||'');rd.readAsDataURL(file);});}
async function rbDrawResult(base64){const blob=await(await kcFetch('data:image/png;base64,'+base64)).blob(),url=URL.createObjectURL(blob);try{const im=new Image();im.src=url;await im.decode();const out=$('removeBgResult');rbFitCanvas(out,im.naturalWidth,im.naturalHeight);const x=out.getContext('2d');x.clearRect(0,0,out.width,out.height);x.drawImage(im,0,0,out.width,out.height);}finally{URL.revokeObjectURL(url)}return blob;}
async function rbProcessFile(file,saveToDisk=false,index=1,total=1){const __job=jobStart('remove-bg',`Remove BG ${file.name||'gambar'}`,1);rbProgress(1,`Membaca gambar ${index}/${total}...`);const base64=await rbFileToBase64(file),rawModel=$('removeBgModel')?.value||'large',model=['small','medium','large'].includes(rawModel)?rawModel:'large',mime=(file.type==='image/jpg'?'image/jpeg':file.type),res=await window.kc.aiRemoveBackground({base64,mime,model,name:file.name,computeMode:$('removeBgCompute')?.value||'auto',parallel:!!$('removeBgParallel')?.checked});if(!res?.ok)throw new Error(res?.error||'AI gagal memproses gambar');const blob=saveToDisk?await (await kcFetch('data:image/png;base64,'+res.base64)).blob():await rbDrawResult(res.base64),base=(file.name||'image').replace(/\.[^.]+$/,''),pngName=`${base}_AI_no-bg.png`;if(saveToDisk){const arr=new Uint8Array(await blob.arrayBuffer());let bin='';for(let i=0;i<arr.length;i+=0x8000){bin+=String.fromCharCode(...arr.subarray(i,i+0x8000));}const out=(removeBgOutputDir.replace(/[\/]$/,''))+'/'+pngName;const wr=await window.kc.writeBase64File({path:out,base64:btoa(bin)});if(!wr?.ok)throw new Error(wr?.error||'Gagal menyimpan PNG');jobDone(__job,true,'Selesai');return{path:wr.path,res};}removeBgOutputFile=new File([blob],pngName,{type:'image/png'});jobDone(__job,true,'Selesai');return{res};}
async function rbProcess(){if(!removeBgFile){$('removeBgInput').click();return;}if(!window.kc?.aiRemoveBackground){rbSetStatus('Engine AI tidak tersedia pada build ini.');return;}$('processRemoveBg').disabled=true;rbSetStatus('AI sedang memisahkan subjek dari background...');try{const out=await rbProcessFile(removeBgFile,false,1,1);try{clipboardResultBase64=$('removeBgResult').toDataURL('image/png').split(',')[1];}catch{}$('saveRemoveBg').disabled=false;$('sendRemoveBg').disabled=false;rbProgress(100,'Selesai - background transparan');rbSetStatus(`Selesai dengan AI ${out.res.model||$('removeBgModel').value}${out.res.fallbackUsed?' (fallback stabil)':''} - ${(Number(out.res.elapsedMs||0)/1000).toFixed(1)} detik${out.res.backend?' · '+out.res.backend:''}.`);}catch(e){reportBug('ai-remove-background-failed',e,'error',{fileType:removeBgFile?.type||'',fileSize:removeBgFile?.size||0,model:$('removeBgModel')?.value||''});rbSetStatus('Remove Background AI gagal: '+(e.message||e));rbProgress(0,'Gagal');}finally{$('processRemoveBg').disabled=false;}}
function openRemoveBg(){$('attachMenu')?.classList.add('hidden');$('removeBgModal').classList.remove('hidden');setTimeout(()=>$('chooseRemoveBg')?.focus(),50);}
if($('removeBgSidebar'))$('removeBgSidebar').onclick=openRemoveBg;
if($('closeRemoveBg'))$('closeRemoveBg').onclick=()=>$('removeBgModal').classList.add('hidden');
if($('chooseRemoveBg'))$('chooseRemoveBg').onclick=()=>{$('removeBgInput').multiple=true;$('removeBgInput').click();};
if($('chooseRemoveBgOutput'))$('chooseRemoveBgOutput').onclick=async()=>{const p=await window.kc.chooseVectorOutput();if(p){removeBgOutputDir=p;showToast('Folder output Remove BG dipilih.');}};
if($('removeBgBatch'))$('removeBgBatch').onchange=()=>{$('runRemoveBgBatch')?.classList.toggle('hidden',!$('removeBgBatch').checked||removeBgFiles.length<2);};
if($('removeBgInput'))$('removeBgInput').onchange=async e=>{const files=[...(e.target.files||[])];if(!files.length)return;removeBgFiles=files;removeBgFile=files[0];if($('removeBgBatch'))$('removeBgBatch').checked=files.length>1;$('runRemoveBgBatch')?.classList.toggle('hidden',!$('removeBgBatch')?.checked||files.length<2);try{await rbLoadFile(removeBgFile);if(files.length>1)rbSetStatus(`Batch siap: ${files.length} gambar.`);}catch(err){rbSetStatus('Gagal membaca gambar: '+err.message);}};
if($('processRemoveBg'))$('processRemoveBg').onclick=rbProcess;
if(window.kc?.onAiBgProgress)window.kc.onAiBgProgress(data=>rbProgress(data?.pct,data?.text));
if($('runRemoveBgBatch'))$('runRemoveBgBatch').onclick=async()=>{if(removeBgFiles.length<2){showToast('Pilih minimal 2 gambar untuk batch.');return;}if(!removeBgOutputDir){const p=await window.kc.chooseVectorOutput();if(!p)return;removeBgOutputDir=p;}$('runRemoveBgBatch').disabled=true;let ok=0,fail=0;const errors=[];for(let i=0;i<removeBgFiles.length;i++){try{await rbProcessFile(removeBgFiles[i],true,i+1,removeBgFiles.length);ok++;}catch(e){fail++;errors.push(`${removeBgFiles[i].name}: ${e.message}`);}rbSetStatus(`Batch Remove BG ${i+1}/${removeBgFiles.length} · sukses ${ok} · gagal ${fail}`);}rbProgress(100,'Batch selesai');$('removeBgBatchSummary').textContent=`Selesai: ${ok} sukses · ${fail} gagal`;$('removeBgBatchSummary').className='batch-summary '+(fail?'warn':'ok');rbSetStatus(`Batch Remove BG selesai - ${ok} sukses / ${fail} gagal.`);showToast(`Batch Remove BG selesai (${ok} sukses, ${fail} gagal).`);$('runRemoveBgBatch').disabled=false;if(errors.length)reportBug('remove-bg-batch-partial-failure',new Error(errors.slice(0,10).join(' | ')),'warning',{ok,fail,total:removeBgFiles.length});};
if($('sendRemoveBg'))$('sendRemoveBg').onclick=()=>{if(!removeBgOutputFile)return;selectedFile=removeBgOutputFile;sendingSticker=false;showFilePreview();$('removeBgModal').classList.add('hidden');showToast('PNG AI transparan siap dikirim ke chat.');$('text')?.focus();};
if($('saveRemoveBg'))$('saveRemoveBg').onclick=async()=>{if(!removeBgOutputFile)return;try{const c=$('removeBgResult'),dataUrl=c.toDataURL('image/png').split(',')[1],r=await window.kc.savePng({name:removeBgOutputFile.name,base64:dataUrl});if(r?.ok)showToast('PNG AI transparan disimpan.');}catch(e){reportBug('remove-background-save-failed',e,'warning',{});}};



// v1.4.1 — renderer bootstrap wajib.
// Bug v1.4.0: init() terdefinisi tetapi tidak pernah dipanggil, sehingga UI mentok CHECKING ROLE dan Mencari server.
let __kcRendererBooted=false;

// v1.7.0 conversation UI bindings
$('newConversation')?.addEventListener('click',()=>{renderNewChatUsers();$('newChatModal').classList.remove('hidden');});
$('closeNewChat')?.addEventListener('click',()=>$('newChatModal').classList.add('hidden'));
$('closeChatImageWorkspace')?.addEventListener('click',closeChatImageWorkspace);
$('chatImagePrev')?.addEventListener('click',e=>{e.stopPropagation();stepChatImage(-1);});
$('chatImageNext')?.addEventListener('click',e=>{e.stopPropagation();stepChatImage(1);});
$('copyChatImage')?.addEventListener('click',async e=>{e.stopPropagation();const item=chatImageSequence[chatImageIndex];if(item)await copyChatImageToClipboard(item.url);});
bindScrollableImage($('chatImageStage'),$('chatImageFull'));
$('chatImageRetry')?.addEventListener('click',()=>{$('chatImageFull').removeAttribute('src');renderChatImageWorkspace();});
$('chatImageStage')?.addEventListener('contextmenu',e=>{const item=chatImageSequence[chatImageIndex],m=item&&getMessageById(item.messageId);if(!m)return;e.preventDefault();e.stopPropagation();showMessageContextMenu(m,e);});
$('chatImageStage')?.addEventListener('click',e=>{if(e.target===$('chatImageStage'))closeChatImageWorkspace();});
$('chatImageFull')?.addEventListener('click',e=>e.stopPropagation());
$('chatImageThumbs')?.addEventListener('click',e=>e.stopPropagation());
document.addEventListener('keydown',e=>{if(!chatImageWorkspaceOpen)return;if(e.key==='Escape'){e.preventDefault();e.stopImmediatePropagation();const menu=$('messageContextMenu');if($('messageReactionPicker')||(menu&&!menu.classList.contains('hidden'))){window.KCChatReactions?.close();menu?.classList.add('hidden');return;}closeChatImageWorkspace();}else if(e.key==='ArrowLeft'){e.preventDefault();stepChatImage(-1);}else if(e.key==='ArrowRight'){e.preventDefault();stepChatImage(1);}},true);
$('newDmTab')?.addEventListener('click',()=>{$('newDmTab').classList.add('active');$('newGroupTab').classList.remove('active');$('newDmPane').classList.remove('hidden');$('newGroupPane').classList.add('hidden');});
$('newGroupTab')?.addEventListener('click',()=>{$('newGroupTab').classList.add('active');$('newDmTab').classList.remove('active');$('newGroupPane').classList.remove('hidden');$('newDmPane').classList.add('hidden');renderNewChatUsers();});
$('newDmSearch')?.addEventListener('input',renderNewChatUsers);
$('createGroupNow')?.addEventListener('click',()=>{const name=$('newGroupName').value.trim(),description=$('newGroupDescription').value.trim(),members=[...document.querySelectorAll('#newGroupMembers input:checked')].map(x=>x.value);socket.emit('conversation-create-group',{name,description,members},r=>{if(!r?.ok)return showToast(r?.error||'Gagal membuat grup');$('newChatModal').classList.add('hidden');$('newGroupName').value='';$('newGroupDescription').value='';requestConversationList();setTimeout(()=>openConversation(r.conversation.id),60);});});
function centerActiveConversationFilter(btn,instant=false){const strip=btn?.closest?.('.conversation-filters');if(!strip||!btn)return;const left=btn.offsetLeft-(strip.clientWidth-btn.offsetWidth)/2;strip.scrollTo({left:Math.max(0,left),behavior:instant?'auto':'smooth'});}
document.querySelectorAll('.conv-filter').forEach(b=>b.addEventListener('click',()=>{document.querySelectorAll('.conv-filter').forEach(x=>x.classList.remove('active'));b.classList.add('active');conversationFilter=b.dataset.filter||'all';centerActiveConversationFilter(b,false);renderConversationList();}));
const conversationFilterStrip=document.querySelector('.conversation-filters');conversationFilterStrip?.addEventListener('wheel',e=>{if(Math.abs(e.deltaY)>Math.abs(e.deltaX)){e.preventDefault();conversationFilterStrip.scrollBy({left:e.deltaY,behavior:'smooth'});}},{passive:false});
$('sidebarSearchInput')?.addEventListener('input',renderConversationList);
$('conversationMenu')?.addEventListener('click',async e=>{const a=e.target.closest('button')?.dataset.action,c=conversationMenuTarget;if(!a||!c)return;const map={pin:'pins',favorite:'favorites',mute:'mutes',archive:'archives'};if(a==='unread'){toggleConvPref('manualUnread',c.id);if(prefHas('manualUnread',c.id)&&String(activeConversationId)===String(c.id))closeActiveConversation();}else if(map[a])toggleConvPref(map[a],c.id);else if(a==='delete-local'){if(!prefHas('deleted',c.id))conversationPrefs.deleted=[...(conversationPrefs.deleted||[]),String(c.id)];saveConversationPrefs();if(String(activeConversationId)===String(c.id))closeActiveConversation();showToast('Chat dihapus dari daftar kamu.');}else if(a==='leave'&&c.type==='group'){const yes=await appConfirm({title:'Keluar grup?',text:'Kamu tidak akan menerima pesan baru dari grup ini.',ok:'Keluar'});if(yes)socket.emit('conversation-leave',{id:c.id},r=>{if(r?.ok){closeActiveConversation();requestConversationList();}else showToast(r?.error||'Gagal keluar grup');});}else if(a==='delete-everyone'&&cfg?.isHost&&c.type==='group'){const yes=await appConfirm({title:'Hapus grup untuk semua?',text:'Grup dan seluruh riwayat pesannya akan dihapus untuk semua anggota.',ok:'Hapus Grup'});if(yes)socket.emit('conversation-delete-group',{id:c.id},r=>{if(r?.ok){closeActiveConversation();requestConversationList();showToast('Grup dihapus untuk semua.');}else showToast(r?.error||'Gagal menghapus grup');});}$('conversationMenu').classList.add('hidden');});
document.addEventListener('click',e=>{if(!e.target.closest('#conversationMenu')&&!e.target.closest('.conv-more'))$('conversationMenu')?.classList.add('hidden');});
async function showMessageDetails(m){const r=await socketAckCompat('message-details',{id:m.id},7000);if(!r?.ok)return showToast(r?.error||'Detail tidak tersedia');if(r.conversationType==='dm'){const x=r.receipt||{};return showReliabilityPanel('Detail pesan',`<article><b>${x.readAt?'✓✓ Dibaca':x.deliveredAt?'✓✓ Terkirim':'✓ Belum terkirim ke penerima'}</b><p>${x.readAt?new Date(x.readAt).toLocaleString('id-ID'):x.deliveredAt?new Date(x.deliveredAt).toLocaleString('id-ID'):'Penerima sedang offline / belum menerima'}</p></article>`)}const rows=(r.members||[]).map(x=>`<article><b>${esc(x.name)}</b><p>${x.readAt?'✓✓ Dibaca · '+new Date(x.readAt).toLocaleString('id-ID'):'✓✓ Terkirim / belum dibaca'}</p></article>`).join('');showReliabilityPanel('Detail dibaca grup',rows||'<p>Belum ada anggota lain.</p>')}
async function forwardMessage(m){const choices=(conversations||[]).filter(c=>String(c.id)!==String(m.conversationId));if(!choices.length)return showToast('Tidak ada chat tujuan.');const html=choices.map(c=>`<article><b>${esc(c.type==='dm'?((c.members||[]).find(n=>profileKeyLocal(n)!==profileKeyLocal(cfg.name))||'Chat pribadi'):(c.name||'Grup'))}</b><button data-forward-to="${esc(c.id)}">Teruskan ke sini</button></article>`).join('');showReliabilityPanel('Teruskan pesan',html);document.querySelectorAll('[data-forward-to]').forEach(b=>b.onclick=async()=>{const r=await socketAckCompat('message-forward',{id:m.id,conversationId:b.dataset.forwardTo,clientMessageId:crypto.randomUUID()},10000);if(r?.ok){document.getElementById('reliabilityPanel')?.close();showToast('Pesan diteruskan.')}else showToast(r?.error||'Forward gagal')})}
$('messageContextMenu')?.addEventListener('click',e=>{const a=e.target.closest('button')?.dataset.msgAction,m=messageContextTarget;if(!a||!m)return;$('messageContextMenu').classList.add('hidden');if(['reply','edit','select','forward','details','delete'].includes(a)&&chatImageWorkspaceOpen)closeChatImageWorkspace();if(a==='copy-image')copyChatImageToClipboard(fullUrl(m.file.url));else if(a==='copy-text')navigator.clipboard.writeText(m.text||'').then(()=>showToast('Teks disalin.')).catch(()=>showToast('Gagal menyalin teks.'));else if(a==='download-image')downloadMessageImage(m);else if(a==='select')toggleMessageSelection(m,true);else if(a==='reply')setReply(m);else if(a==='forward')forwardMessage(m);else if(a==='details')showMessageDetails(m);else if(a==='edit')startEdit(m);else if(a==='delete')deleteMessage(m);else if(a==='save-sticker')saveReceivedSticker(m);});
document.addEventListener('click',e=>{if(!e.target.closest('#messageContextMenu'))$('messageContextMenu')?.classList.add('hidden');});
document.addEventListener('keydown',e=>{if(e.key==='Escape'&&!document.querySelector('.modal:not(.hidden)')){e.preventDefault();closeActiveConversation();}});


document.addEventListener('pointerdown',e=>{
  const dock=$('toolDock');
  if(dock&&!dock.classList.contains('hidden')&&!e.target.closest('#toolDock')&&!e.target.closest('#railTools')){dock.classList.add('hidden');$('railTools')?.classList.remove('active');$('railChats')?.classList.add('active');}
  const host=$('hostQuickMenu');
  if(host&&!host.classList.contains('hidden')&&!e.target.closest('#hostQuickMenu')&&!e.target.closest('#railHostMore'))host.classList.add('hidden');
});
document.querySelectorAll('.modal').forEach(modal=>modal.addEventListener('pointerdown',e=>{if(e.target!==modal)return;if(modal.id==='confirmModal'||modal.id==='setup')return;if(modal.id==='activityModal'){closeStatusWorkspace();return;}modal.classList.add('hidden');}));

$('activeChatPin')?.addEventListener('click',()=>toggleConvPref('pins',activeConversationId));
$('activeChatMute')?.addEventListener('click',()=>toggleConvPref('mutes',activeConversationId));
$('activeChatInfo')?.addEventListener('click',openChatInfo);$('activeChatIdentity')?.addEventListener('click',openChatInfo);$('activeChatIdentity')?.addEventListener('keydown',e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();openChatInfo();}});$('closeChatInfo')?.addEventListener('click',()=>$('chatInfoModal').classList.add('hidden'));
$('railChats')?.addEventListener('click',()=>{if(chatImageWorkspaceOpen)closeChatImageWorkspace();$('toolDock')?.classList.add('hidden');document.querySelector('.conversation-sidebar')?.classList.remove('hidden');$('railChats')?.classList.add('active');$('railTools')?.classList.remove('active');});
$('railGame')?.addEventListener('click',()=>{if(statusWorkspaceIsOpen?.())closeStatusWorkspace();$('toolDock')?.classList.add('hidden');openGameHub();});
$('railModeration')?.addEventListener('click',()=>{$('adminButton')?.click();});
$('railTools')?.addEventListener('click',()=>{const dock=$('toolDock');$('hostQuickMenu')?.classList.add('hidden');dock?.classList.toggle('hidden');$('railTools')?.classList.toggle('active',!dock?.classList.contains('hidden'));$('railChats')?.classList.toggle('active',dock?.classList.contains('hidden'));});$('closeToolDock')?.addEventListener('click',()=>{$('toolDock')?.classList.add('hidden');$('railTools')?.classList.remove('active');$('railChats')?.classList.add('active');});
$('railTheme')?.addEventListener('click',()=>{$('appearanceModal')?.classList.remove('hidden');applyAppearancePrefs(true);if($('appearanceTheme'))$('appearanceTheme').value=document.documentElement.dataset.theme||'dark';});
$('railNotify')?.addEventListener('click',()=>$('notificationButton')?.click());
const openMyProfilePanel=()=>{renderProfilePreview();renderMyActivityPreview();$('profileModal')?.classList.remove('hidden');};$('railProfile')?.addEventListener('click',openMyProfilePanel);$('discordProfileBar')?.addEventListener('click',openMyProfilePanel);$('closeActivity')?.addEventListener('click',closeStatusWorkspace);
$('railStorage')?.addEventListener('click',()=>$('storageButton')?.click());$('railSystem')?.addEventListener('click',()=>$('systemButton')?.click());
$('closeAppearance')?.addEventListener('click',()=>$('appearanceModal')?.classList.add('hidden'));$('appearanceTheme')?.addEventListener('change',e=>applyTheme(e.target.value,true));
$('accentColor')?.addEventListener('input',e=>{appearancePrefs.preset='custom';appearancePrefs.accent=e.target.value;saveAppearancePrefs();applyAppearancePrefs(true);});
document.getElementById('accentPreset')?.addEventListener('change', event => { appearancePrefs.preset = event.target.value; saveAppearancePrefs(); applyAppearancePrefs(true); });
$('peerMessageAlign')?.addEventListener('change',e=>{appearancePrefs.peerAlign=e.target.value;saveAppearancePrefs();});
$('sidebarWidthRange')?.addEventListener('input',e=>{appearancePrefs.sidebarWidth=Number(e.target.value);saveAppearancePrefs();applyAppearancePrefs(true);});
$('archivedShortcut')?.addEventListener('click',()=>{conversationFilter=conversationFilter==='archived'?'all':'archived';document.querySelectorAll('.conv-filter').forEach(x=>x.classList.toggle('active',conversationFilter==='all'&&x.dataset.filter==='all'));renderConversationList();});
(()=>{const rz=$('conversationResizer');if(!rz)return;let dragging=false;const move=e=>{if(!dragging)return;const rail=56;appearancePrefs.sidebarWidth=Math.max(280,Math.min(520,e.clientX-rail));applyAppearancePrefs(true);};rz.addEventListener('pointerdown',e=>{dragging=true;rz.setPointerCapture?.(e.pointerId);document.body.classList.add('resizing-conversations');e.preventDefault();});rz.addEventListener('pointermove',move);rz.addEventListener('pointerup',e=>{if(!dragging)return;dragging=false;document.body.classList.remove('resizing-conversations');saveAppearancePrefs();rz.releasePointerCapture?.(e.pointerId);});rz.addEventListener('dblclick',()=>{appearancePrefs.sidebarWidth=360;applyAppearancePrefs(true);saveAppearancePrefs();});})();
$('railHostMore')?.addEventListener('click',e=>{const m=$('hostQuickMenu');if(!m)return;const r=e.currentTarget.getBoundingClientRect();m.style.left=Math.min(innerWidth-200,r.right+8)+'px';m.style.bottom='auto';m.style.top=Math.max(12,Math.min(innerHeight-m.offsetHeight-12,r.top-150))+'px';m.classList.toggle('hidden');});
$('hostQuickMenu')?.addEventListener('click',e=>{const a=e.target.closest('button')?.dataset.hostAction;if(!a)return;if(a==='conversations'){openHostConversationControl();}else{const map={update:'updateManagerButton',admin:'adminButton',system:'systemButton',bug:'diagnosticsButton'};$(map[a])?.click();}$('hostQuickMenu').classList.add('hidden');});
document.addEventListener('click',e=>{if(!e.target.closest('#railHostMore,#hostQuickMenu'))$('hostQuickMenu')?.classList.add('hidden');});
document.querySelector('.wa-rail')?.addEventListener('click',e=>{if(chatImageWorkspaceOpen&&e.target.closest('.rail-btn'))closeChatImageWorkspace();},true);

let groupInviteTarget=null;function openGroupInvite(c){groupInviteTarget=c;const members=new Set((c.members||[]).map(x=>String(x).toLowerCase()));const users=[...(editorDirectory||[]),...(onlineUsers||[])];const by=new Map();for(const u of users){const k=String(u.name||'').toLowerCase();if(k&&!by.has(k))by.set(k,u);}const box=$('groupInviteList');box.innerHTML=[...by.values()].map(u=>`<label class="new-chat-user"><input type="checkbox" value="${esc(u.name)}" ${members.has(String(u.name).toLowerCase())?'checked':''}>${avatarHtml(u.avatarUrl,u.name,'avatar')}<span><b>${esc(profileDisplayName(u.name))}</b></span></label>`).join('');$('groupInviteModal').classList.remove('hidden');}
$('closeGroupInvite')?.addEventListener('click',()=>$('groupInviteModal')?.classList.add('hidden'));$('saveGroupInvite')?.addEventListener('click',()=>{if(!groupInviteTarget)return;const members=[...document.querySelectorAll('#groupInviteList input:checked')].map(x=>x.value);socket.emit('conversation-group-update',{id:groupInviteTarget.id,members},r=>{if(!r?.ok)return showToast(r?.error||'Gagal memperbarui anggota');$('groupInviteModal').classList.add('hidden');$('chatInfoModal').classList.add('hidden');requestConversationList();setTimeout(()=>openConversation(groupInviteTarget.id),80);showToast('Anggota grup diperbarui.');});});
function openHostConversationControl(){if(!cfg?.isHost)return;socket.emit('host-conversation-list',{},r=>{if(!r?.ok)return showToast(r?.error||'Gagal memuat kontrol chat');const box=$('hostConversationList');box.innerHTML=(r.items||[]).map(c=>`<div class="new-chat-user host-conv-row" data-host-open="${esc(c.id)}"><div class="avatar">${c.type==='group'?'👥':'↔'}</div><div><b>${esc(c.type==='dm'?(c.members||[]).join(' ↔ '):(c.name||'Grup'))}</b><div class="conv-preview">${esc(c.type)} · ${(c.members||[]).length} anggota${c.hostListed?' · tampil di sidebar HOST':' · hidden dari sidebar HOST'}</div></div></div>`).join('')||'<div class="storage-note">Belum ada percakapan.</div>';box.querySelectorAll('[data-host-open]').forEach(el=>el.onclick=()=>{$('hostConversationModal').classList.add('hidden');openConversation(el.dataset.hostOpen);});$('hostConversationModal').classList.remove('hidden');});}
$('closeHostConversations')?.addEventListener('click',()=>$('hostConversationModal')?.classList.add('hidden'));


// ================= v1.11.0: STATUS + DISTRIBUTED FILE GALLERY + GROUP IDENTITY =================
let statusItems=[],statusPendingMedia=null,statusViewerItem=null,statusViewerSequence=[],statusViewerIndex=0,groupAvatarTarget=null;
let galleryItems=[],gallerySelectedItem=null,galleryView='large',galleryScanBusy=false,galleryRegisterCount=0;
const galleryPreviewCache=new Map(),galleryPreviewWaiters=new Map(),galleryIncomingTransfers=new Map(),galleryCancelledTransfers=new Set();
function socketAckEvent(event,payload,timeout=15000){return new Promise(resolve=>{if(!socket?.connected)return resolve({ok:false,error:'Tidak terhubung ke HOST'});let done=false;const t=setTimeout(()=>{if(!done){done=true;resolve({ok:false,error:'Timeout '+event})}},timeout);socket.emit(event,payload,r=>{if(done)return;done=true;clearTimeout(t);resolve(r||{ok:true});});});}
function statusTypeIcon(x){return x?.type==='image'?'🖼️':x?.type==='video'?'🎬':x?.type==='audio'?'🎵':'✎';}
function statusAge(ts){const m=Math.max(0,Math.floor((Date.now()-Number(ts||0))/60000));if(m<1)return'baru saja';if(m<60)return m+' menit lalu';const h=Math.floor(m/60);return h<24?h+' jam lalu':'kemarin';}
function renderStatusMeAvatar(){const el=$('statusMeAvatar');if(!el)return;const url=myProfile?.avatarUrl;if(url){el.classList.add('has-photo');el.style.backgroundImage=`url("${fullUrl(url)}")`;el.textContent='';}else{el.classList.remove('has-photo');el.style.backgroundImage='';el.textContent=(cfg?.name||'K')[0].toUpperCase();}}
function renderStatuses(){const box=$('statusList');if(!box)return;renderStatusMeAvatar();const valid=(statusItems||[]).filter(x=>Number(x.expiresAt||0)>Date.now());const by=new Map();for(const st of valid){const k=profileKeyLocal(st.user),arr=by.get(k)||[];arr.push(st);by.set(k,arr);}const groups=[...by.values()].map(a=>a.sort((x,y)=>Number(y.createdAt||0)-Number(x.createdAt||0))).sort((a,b)=>{const am=profileKeyLocal(a[0]?.user)===profileKeyLocal(cfg?.name),bm=profileKeyLocal(b[0]?.user)===profileKeyLocal(cfg?.name);if(am!==bm)return am?-1:1;return Number(b[0]?.createdAt||0)-Number(a[0]?.createdAt||0)});if(!groups.length){box.innerHTML='<div class="status-empty compact">Belum ada status.</div>';statusWorkspaceReset();return;}box.innerHTML=groups.map(arr=>{const st=arr[0],url=st.avatarUrl,me=profileKeyLocal(st.user)===profileKeyLocal(cfg?.name);return`<div class="status-card" data-status-id="${esc(st.id)}" data-status-user="${esc(profileKeyLocal(st.user))}"><div class="status-ring">${avatarHtml(url,st.user,'avatar')}</div><div class="status-card-main"><b>${me?'Status saya':esc(st.user)}</b><span>${statusTypeIcon(st)} ${esc(st.text||st.media?.name||'Status media')}</span><small>${statusAge(st.createdAt)}${arr.length>1?' · '+arr.length+' update':''}</small></div></div>`;}).join('');box.querySelectorAll('[data-status-id]').forEach(card=>card.onclick=()=>openStatusViewer(card.dataset.statusId));}
function requestStatuses(){socketAckEvent('status-list-request',{}).then(r=>{if(r?.ok){statusItems=r.items||[];renderStatuses();}});}
function renderStatusViewerItem(){const st=statusViewerSequence[statusViewerIndex];if(!st)return;statusViewerItem=st;$('statusWorkspaceEmpty')?.classList.add('hidden');$('activityWorkspaceViewer')?.classList.add('hidden');$('statusWorkspaceViewer')?.classList.remove('hidden');setWorkspaceAvatar($('statusViewerAvatar'),st.user,st.avatarUrl);$('statusViewerName').textContent=profileKeyLocal(st.user)===profileKeyLocal(cfg?.name)?'Status saya':st.user;$('statusViewerTime').textContent=statusAge(st.createdAt)+' · '+(statusViewerIndex+1)+'/'+statusViewerSequence.length+' · hilang setelah 24 jam';const progress=$('statusViewerProgress');if(progress)progress.innerHTML=statusViewerSequence.map((_,i)=>`<i class="${i<=statusViewerIndex?'active':''}"></i>`).join('');const b=$('statusViewerBody');if(st.type==='image'&&st.media?.url)b.innerHTML=`<img src="${esc(fullUrl(st.media.url))}" alt="Status">`;else if(st.type==='video'&&st.media?.url)b.innerHTML=`<video src="${esc(fullUrl(st.media.url))}" controls autoplay></video>`;else if(st.type==='audio'&&st.media?.url)b.innerHTML=`<div class="status-audio-card">🎵<b>${esc(st.media?.name||'Musik')}</b><audio src="${esc(fullUrl(st.media.url))}" controls autoplay></audio></div>`;else b.innerHTML=`<div class="status-text-only">${esc(st.text||'Status')}</div>`;$('statusViewerText').textContent=st.type==='text'?'':(st.text||'');const ownStatus=profileKeyLocal(st.user)===profileKeyLocal(cfg?.name);$('deleteStatusNow').classList.toggle('hidden',!(cfg?.isHost||ownStatus));$('statusReplyBar')?.classList.toggle('hidden',ownStatus);if(!ownStatus&&$('statusReplyInput'))$('statusReplyInput').placeholder=`Balas status ${st.user}...`;$('statusViewerPrev').disabled=statusViewerIndex<=0;$('statusViewerNext').disabled=statusViewerIndex>=statusViewerSequence.length-1;document.querySelectorAll('#statusList .status-card.active,#activityList .activity-row.active').forEach(x=>x.classList.remove('active'));document.querySelector(`#statusList .status-card[data-status-user="${CSS.escape(profileKeyLocal(st.user))}"]`)?.classList.add('active');}
function openStatusViewer(id){const st=(statusItems||[]).find(x=>String(x.id)===String(id));if(!st)return;statusViewerSequence=(statusItems||[]).filter(x=>profileKeyLocal(x.user)===profileKeyLocal(st.user)&&Number(x.expiresAt||0)>Date.now()).sort((a,b)=>Number(a.createdAt||0)-Number(b.createdAt||0));statusViewerIndex=statusViewerSequence.findIndex(x=>String(x.id)===String(id));if(statusViewerIndex<0)statusViewerIndex=Math.max(0,statusViewerSequence.length-1);renderStatusViewerItem();}
async function publishStatusNow(){const text=String($('statusTextInput')?.value||'').trim();if(!text&&!statusPendingMedia)return showToast('Isi tulisan atau pilih media dulu.');const r=await socketAckEvent('status-create',{text,media:statusPendingMedia},15000);if(!r?.ok)return showToast(r?.error||'Gagal membuat status');statusPendingMedia=null;$('statusTextInput').value='';$('statusComposer').classList.add('hidden');showToast('Status dibagikan selama 24 jam.');requestStatuses();}
async function uploadStatusMedia(file){if(!file)return;if(file.size>250*1024*1024)return showToast('Media status maksimal 250 MB.');const fd=new FormData();fd.append('media',file);showToast('Mengupload media status...');try{const r=await kcFetch(serverBase+'/status/upload',{method:'POST',body:fd}),j=await r.json();if(!r.ok||!j.ok)throw new Error(j.error||'Upload status gagal');statusPendingMedia=j.media;$('statusComposer').classList.remove('hidden');$('statusTextInput').placeholder='Caption status (opsional)';$('statusTextInput').focus();showToast('Media siap. Tambahkan caption lalu Bagikan Status.');}catch(e){showToast(e.message||'Upload status gagal');}}

function galleryFolders(){return Array.isArray(cfg?.galleryFolders)?cfg.galleryFolders.filter(Boolean):[];}
let galleryTotalMatches=0,galleryIndexedUniverse=0,galleryHasMore=false,galleryPageSize=120,galleryQuerySeq=0,galleryThumbObserver=null,galleryThumbActive=0;
const gallerySelectedIds=new Set(),galleryThumbQueue=[],galleryDownloadCompletion=new Map();
function renderGalleryFolders(){const box=$('galleryFolders');if(!box)return;const folders=galleryFolders();box.innerHTML=folders.length?folders.map((f,i)=>`<span class="gallery-folder-chip" title="${esc(f)}"><span>${esc(f)}</span><button type="button" data-gallery-remove-folder="${i}" title="Hapus folder dari index">×</button></span>`).join(''):'<span class="gallery-folder-chip">Belum ada folder dipilih di PC ini</span>';box.querySelectorAll('[data-gallery-remove-folder]').forEach(btn=>btn.onclick=async e=>{e.stopPropagation();const i=Number(btn.dataset.galleryRemoveFolder),next=galleryFolders().filter((_,n)=>n!==i);cfg.galleryFolders=next;await window.kc.saveConfig({galleryFolders:next});renderGalleryFolders();galleryPreviewCache.clear();await autoScanFileGallery(false,true);if(!next.length){await queryFileGallery();}else await queryFileGallery();});}
function galleryIcon(item){const e=String(item?.ext||'').toUpperCase();if(item.kind==='photoshop')return'PSD';if(item.kind==='illustrator')return'AI';if(item.kind==='coreldraw')return'CDR';if(item.kind==='pdf')return'PDF';if(item.kind==='image')return'IMG';return e||'FILE';}
function gallerySortedRows(rows){const mode=$('gallerySort')?.value||'az',a=[...rows];if(mode==='za')a.sort((x,y)=>String(y.name).localeCompare(String(x.name),undefined,{numeric:true,sensitivity:'base'}));else if(mode==='newest')a.sort((x,y)=>Number(y.mtimeMs||0)-Number(x.mtimeMs||0));else if(mode==='largest')a.sort((x,y)=>Number(y.size||0)-Number(x.size||0));else a.sort((x,y)=>String(x.name).localeCompare(String(y.name),undefined,{numeric:true,sensitivity:'base'}));return a;}
function galleryGroupKey(item,mode){if(mode==='letter')return (String(item.name||'#')[0]||'#').toUpperCase();if(mode==='format')return String(item.ext||'LAINNYA').toUpperCase();if(mode==='pc')return item.ownerName||item.ownerPc||'Editor tidak diketahui';return'';}
function cacheGalleryPreview(id,data){if(!id||!data)return;if(galleryPreviewCache.has(id))galleryPreviewCache.delete(id);galleryPreviewCache.set(id,data);while(galleryPreviewCache.size>120)galleryPreviewCache.delete(galleryPreviewCache.keys().next().value);}
function updateGallerySelectionUi(){const n=gallerySelectedIds.size,dl=$('galleryDownloadSelected'),clear=$('galleryClearSelection');if(dl){dl.disabled=!n;dl.textContent=`Unduh Terpilih (${n})`;}if(clear)clear.disabled=!n;document.querySelectorAll('#fileGalleryResults [data-gallery-file]').forEach(card=>{const on=gallerySelectedIds.has(card.dataset.galleryFile);card.classList.toggle('selected',on);const cb=card.querySelector('[data-gallery-select]');if(cb)cb.checked=on;});}
function galleryCardHtml(f){const cached=galleryPreviewCache.get(f.fileId),online=f.sourceOnline!==false,selected=gallerySelectedIds.has(String(f.fileId)),chat=f.sourceType==='chat',sourceEditor=f.ownerName||'Editor tidak diketahui',sourcePc=f.ownerPc||'';return`<div class="gallery-file ${selected?'selected':''}" data-gallery-file="${esc(f.fileId)}" data-previewable="${f.previewable?'1':'0'}"><label class="gallery-select-box" title="Pilih file"><input type="checkbox" data-gallery-select="${esc(f.fileId)}" ${selected?'checked':''}><span>✓</span></label><div class="gallery-thumb">${cached?`<img src="${cached}" alt="">`:`<span>${esc(galleryIcon(f))}</span>`}</div><div class="gallery-meta"><b title="${esc(f.name)}">${esc(f.name)}</b><span class="gallery-pc-source" title="${esc(sourcePc?`Editor ${sourceEditor} · PC ${sourcePc}`:`Editor ${sourceEditor}`)}"><strong>Editor:</strong> ${esc(sourceEditor)}</span><span>${fmtSize(Number(f.size||0))} · ${esc((f.ext||'file').toUpperCase())}</span><span title="${esc(f.relativePath||'')}">${esc(f.root||'')}${f.relativePath?' / '+esc(f.relativePath):''}</span></div><div class="gallery-card-actions"><button type="button" data-gallery-download="${esc(f.fileId)}" ${online?'':'disabled'}>Unduh</button></div><span class="gallery-source ${chat?'chat-source':online?'':'offline'}">${chat?'CHAT':online?'ONLINE':'OFFLINE'}</span></div>`;}
async function replyStatusNow(){const st=statusViewerItem,text=String($('statusReplyInput')?.value||'').trim();if(!st||profileKeyLocal(st.user)===profileKeyLocal(cfg?.name))return;if(!text)return showToast('Tulis balasan status dulu.');const sendBtn=$('statusReplySend');if(sendBtn)sendBtn.disabled=true;try{const dm=await socketAckEvent('conversation-create-dm',{name:st.user},8000);if(!dm?.ok||!dm.conversation?.id)throw new Error(dm?.error||'Chat pribadi tidak bisa dibuat');const r=await socketSend({text,conversationId:dm.conversation.id,statusReplyToId:st.id});if(!r?.ok)throw new Error(r?.error||'Balasan status gagal dikirim');$('statusReplyInput').value='';showToast('↩ Balasan status terkirim ke chat '+st.user);closeStatusWorkspace();requestConversationList();setTimeout(()=>openConversation(dm.conversation.id),100);}catch(e){showToast(e?.message||'Balasan status gagal dikirim',4200);}finally{if(sendBtn)sendBtn.disabled=false;}}
function renderFileGallery(){const box=$('fileGalleryResults');if(!box)return;box.className='file-gallery-results view-'+galleryView;const rows=gallerySortedRows(galleryItems),mode=$('galleryGroup')?.value||'none';$('galleryStats').innerHTML=`${galleryTotalMatches} file cocok · ${rows.length} ditampilkan · <span class="full-index-note">search mengecek seluruh ${galleryIndexedUniverse||galleryTotalMatches} file terindeks</span> · index PC ini ${galleryRegisterCount} file`;if(!rows.length){box.innerHTML='<div class="status-empty">Belum ada file yang cocok. Pilih folder pada setiap PC lalu Scan Ulang.</div>';$('galleryLoadMore')?.classList.add('hidden');updateGallerySelectionUi();return;}if(mode==='none')box.innerHTML=`<div class="gallery-grid">${rows.map(galleryCardHtml).join('')}</div>`;else{const groups=new Map();for(const f of rows){const k=galleryGroupKey(f,mode),a=groups.get(k)||[];a.push(f);groups.set(k,a);}box.innerHTML=[...groups.entries()].sort((a,b)=>String(a[0]).localeCompare(String(b[0]),undefined,{numeric:true})).map(([k,a])=>`<div class="gallery-group"><div class="gallery-group-title">${esc(k)} · ${a.length}</div><div class="gallery-grid">${a.map(galleryCardHtml).join('')}</div></div>`).join('');}box.querySelectorAll('[data-gallery-file]').forEach(card=>card.onclick=e=>{if(e.target.closest('[data-gallery-select],[data-gallery-download],.gallery-select-box'))return;openGalleryPreview(card.dataset.galleryFile);});box.querySelectorAll('[data-gallery-select]').forEach(cb=>cb.onchange=e=>{e.stopPropagation();const id=String(cb.dataset.gallerySelect||'');if(cb.checked)gallerySelectedIds.add(id);else gallerySelectedIds.delete(id);updateGallerySelectionUi();});box.querySelectorAll('[data-gallery-download]').forEach(btn=>btn.onclick=e=>{e.stopPropagation();const item=galleryItems.find(x=>String(x.fileId)===String(btn.dataset.galleryDownload));if(item)downloadGalleryFile(item);});const more=$('galleryLoadMore');if(more)more.classList.toggle('hidden',!galleryHasMore);updateGallerySelectionUi();hydrateGalleryThumbs();}
let galleryQueryInFlight=false,galleryQueuedQuery=null;
async function queryFileGallery(opts={}){
  if(galleryQueryInFlight){galleryQueuedQuery=opts;return;}
  galleryQueryInFlight=true;
  const append=opts?.append===true,seq=++galleryQuerySeq,q=String($('gallerySearch')?.value||'').trim(),kind=$('galleryKind')?.value||'all',scope=$('gallerySearchScope')?.value||'file',sort=$('gallerySort')?.value||'az',offset=append?galleryItems.length:0;
  if(!socket?.connected){if($('galleryStats'))$('galleryStats').textContent='Menunggu koneksi HOST...';galleryQueryInFlight=false;return;}
  if(!append){gallerySelectedIds.clear();galleryItems=[];galleryTotalMatches=0;galleryHasMore=false;if($('galleryStats'))$('galleryStats').textContent='Mencari di index Gallery semua PC...';const box=$('fileGalleryResults');if(box)box.innerHTML='<div class="status-empty">Mencari file…</div>';}
  try{
    const r=await socketAckEvent('file-gallery-query',{q,kind,scope,sort,offset,limit:galleryPageSize},26000);
    if(seq!==galleryQuerySeq)return;
    if(!r?.ok){const why=String(r?.error||'Gagal mencari file');if($('galleryStats'))$('galleryStats').textContent=/timeout|batas waktu/i.test(why)?'Pencarian melewati batas waktu · coba lagi setelah koneksi HOST stabil':'Pencarian Gallery gagal · '+why;const box=$('fileGalleryResults');if(box)box.innerHTML='<div class="status-empty">Pencarian belum selesai. Coba lagi atau tekan Scan Ulang bila index PC belum tersinkron.</div>';showToast(why);return;}
    const incoming=Array.isArray(r.items)?r.items:[];
    if(append){const by=new Map(galleryItems.map(x=>[String(x.fileId),x]));for(const x of incoming)by.set(String(x.fileId),x);galleryItems=[...by.values()];}else galleryItems=incoming;
    galleryTotalMatches=Number(r.total??galleryItems.length);galleryIndexedUniverse=Number(r.indexedTotal??galleryTotalMatches);galleryHasMore=!!r.hasMore;
    if($('galleryStats')&&Number.isFinite(Number(r.queryMs)))$('galleryStats').dataset.lastQueryMs=String(r.queryMs);
    renderFileGallery();
  }catch(error){
    if(seq!==galleryQuerySeq)return;
    const message=String(error?.message||error||'Gagal mencari file');if($('galleryStats'))$('galleryStats').textContent=/timeout|batas waktu/i.test(message)?'Pencarian melewati batas waktu · coba lagi setelah koneksi HOST stabil':'Pencarian Gallery gagal';const box=$('fileGalleryResults');if(box)box.innerHTML='<div class="status-empty">Pencarian belum selesai. Coba lagi atau tekan Scan Ulang bila index PC belum tersinkron.</div>';showToast(message);
  }finally{
    galleryQueryInFlight=false;
    if(galleryQueuedQuery){const next=galleryQueuedQuery;galleryQueuedQuery=null;setTimeout(()=>queryFileGallery(next),0);}
  }
}
let gallerySyncPromise=null;
function syncGalleryDatabaseV2(){if(gallerySyncPromise)return gallerySyncPromise;gallerySyncPromise=syncGalleryDatabaseRun().finally(()=>gallerySyncPromise=null);return gallerySyncPromise;}
async function syncGalleryDatabaseRun(){
 const local=await window.kc.galleryIndexLocalStatus();if(!local?.ok)return{database:false};const remote=await socketAckEvent('file-gallery-index-status',{},8000);if(!remote?.ok||!remote.database)return{database:false};
 const localMax=Number(local.maxSeq||0),generation=local.generation;let after=Number(remote.lastSeq||0);let full=remote.generation!==generation||after>localMax||(remote.count!==local.count&&after===localMax);
 let delta=full?null:await window.kc.galleryIndexLocalChanges({afterSeq:after,limit:1000});if(delta?.items?.length&&Number(delta.items[0].seq)!==after+1)full=true;
 if(full){const snapshotId=crypto.randomUUID();let offset=0,first=true;do{const page=await window.kc.galleryIndexLocalExport({offset,limit:1000});if(!page?.ok)throw Error(page?.error||'Index lokal gagal');if(Number(page.maxSeq)!==localMax)throw Error('Index berubah saat sync, ulangi Scan');const r=await socketAckEvent('file-gallery-delta',{full:true,reset:first,final:!page.hasMore,snapshotId,generation,seq:localMax,upserts:page.items||[]},30000);if(!r?.ok)throw Error(r?.error||'Snapshot gagal');first=false;offset+=(page.items||[]).length;if(!page.hasMore)break;}while(true);after=localMax;
 }else while(after<localMax){const page=delta||await window.kc.galleryIndexLocalChanges({afterSeq:after,limit:1000});delta=null;if(!page?.ok||!page.items?.length)throw Error('Delta hilang; Scan Ulang untuk sinkronisasi');const last=new Map();for(const c of page.items)last.set(c.localId,c);const changes=[...last.values()],seq=Number(page.maxSeq);const r=await socketAckEvent('file-gallery-delta',{generation,baseSeq:after,seq,upserts:changes.filter(x=>x.action==='upsert').map(x=>x.row),deletes:changes.filter(x=>x.action==='delete').map(x=>x.localId)},30000);if(!r?.ok)throw Error(r?.error||'Delta gagal');after=seq;}
 if(after>0)await window.kc.galleryIndexLocalAck({seq:after});galleryRegisterCount=Number(local.count||0);return{database:true,count:galleryRegisterCount,syncedSeq:after};
}
async function registerGalleryFiles(files,signature=''){const rows=Array.isArray(files)?files:[],sig=String(signature||'');
  if(rows.length){try{const status=await socketAckEvent('file-gallery-index-status',{},5000);if(status?.ok&&Number(status.count||0)===rows.length&&sig&&String(status.signature||'')===sig){galleryRegisterCount=rows.length;return{skipped:true,count:rows.length};}}catch{}}
  const batch=1000;if(!rows.length){await socketAckEvent('file-gallery-register',{reset:true,final:true,files:[],signature:sig},20000);galleryRegisterCount=0;return{skipped:false,count:0};}
  for(let i=0;i<rows.length;i+=batch){const final=i+batch>=rows.length,r=await socketAckEvent('file-gallery-register',{reset:i===0,final,files:rows.slice(i,i+batch),signature:sig},30000);if(!r?.ok)throw new Error(r?.error||'Gagal mendaftarkan index file');galleryRegisterCount=r.count||Math.min(rows.length,i+batch);}galleryRegisterCount=rows.length;return{skipped:false,count:rows.length};}
const GALLERY_REFRESH_INTERVAL_MS = 30 * 60 * 1000;
let galleryRefreshTimer;
function scheduleGalleryRefresh(target, epoch) {
  const current = () => target === socket && target._kcSessionEpoch === epoch && sessionReady;
  async function refresh() {
    if (!current() || !target.connected) return;
    try {
      if (galleryFolders().length) await autoScanFileGallery(false, true);
    } finally {
      if (current() && target.connected) galleryRefreshTimer = setTimeout(refresh, GALLERY_REFRESH_INTERVAL_MS);
    }
  }
  clearTimeout(galleryRefreshTimer);
  galleryRefreshTimer = setTimeout(refresh, 20000);
}
async function autoScanFileGallery(showUi=true,forceRefresh=showUi===true){if(galleryScanBusy||!window.kc?.scanGalleryFolders)return;const folders=galleryFolders();renderGalleryFolders();if(!folders.length){if(forceRefresh){galleryScanBusy=true;try{const r=await window.kc.scanGalleryFolders({folders:[],refresh:true});if(r?.database)await syncGalleryDatabaseV2();galleryRegisterCount=0;if(showUi)showToast('Semua folder Gallery dilepas dari index PC ini.');await queryFileGallery();}catch(e){if(showUi)showToast(e.message||'Gagal membersihkan index Gallery');}finally{galleryScanBusy=false;}return;}if(showUi)showToast('Pilih folder yang mau di-scan dulu.');return;}galleryScanBusy=true;const galleryOpen=!$('fileGalleryModal')?.classList.contains('hidden');if(showUi&&$('galleryStats'))$('galleryStats').textContent=forceRefresh?'Memeriksa perubahan folder PC ini...':'Memuat database Gallery lokal...';try{const r=await window.kc.scanGalleryFolders({folders,refresh:forceRefresh===true});if(!r?.ok)throw new Error(r?.error||'Scan gagal');cfg.galleryFolders=r.folders||folders;await window.kc.saveConfig({galleryFolders:cfg.galleryFolders});let reg;if(r.database)reg=await syncGalleryDatabaseV2();else reg=await registerGalleryFiles(r.files||[],r.signature||'');renderGalleryFolders();const count=Number(r.count??(r.files||[]).length);galleryRegisterCount=count;if(showUi)showToast(r.database?`${count.toLocaleString('id-ID')} file terindeks · ${Number(r.changed||0).toLocaleString('id-ID')} berubah · ${Number(r.deleted||0).toLocaleString('id-ID')} dihapus. Database tidak punya batas file tetap.`:`${count} file dari PC ini masuk index.`);if(showUi||galleryOpen)await queryFileGallery();return{cached:!!r.cached,registered:!!reg,count};}catch(e){if(showUi)showToast(e.message||'Scan File Gallery gagal');}finally{galleryScanBusy=false;}}
function requestGalleryPreview(fileId){if(galleryPreviewCache.has(fileId))return Promise.resolve(galleryPreviewCache.get(fileId));return new Promise(async resolve=>{const r=await socketAckEvent('file-gallery-preview-request',{fileId},10000);if(!r?.ok)return resolve(null);const timer=setTimeout(()=>{galleryPreviewWaiters.delete(r.requestId);resolve(null);},12000);galleryPreviewWaiters.set(r.requestId,{resolve:(data)=>{clearTimeout(timer);const src=String(data||'');resolve(src.startsWith('/')?fullUrl(src):src);}});});}
function pumpGalleryThumbQueue(){while(galleryThumbActive<8&&galleryThumbQueue.length){const card=galleryThumbQueue.shift();if(!card?.isConnected)continue;const id=card.dataset.galleryFile;if(!id||galleryPreviewCache.has(id))continue;galleryThumbActive++;requestGalleryPreview(id).then(data=>{if(data){cacheGalleryPreview(id,data);if(card.isConnected){const thumb=card.querySelector('.gallery-thumb');if(thumb)thumb.innerHTML=`<img src="${data}" alt="">`;}}}).finally(()=>{galleryThumbActive--;pumpGalleryThumbQueue();});}}
function queueGalleryThumb(card){if(!card||card.dataset.thumbQueued==='1')return;card.dataset.thumbQueued='1';galleryThumbQueue.push(card);pumpGalleryThumbQueue();}
function hydrateGalleryThumbs(){galleryThumbObserver?.disconnect?.();galleryThumbQueue.length=0;const cards=[...document.querySelectorAll('#fileGalleryResults [data-previewable="1"]')].filter(c=>!galleryPreviewCache.has(c.dataset.galleryFile));if(!cards.length)return;if('IntersectionObserver'in window){galleryThumbObserver=new IntersectionObserver(entries=>{for(const en of entries)if(en.isIntersecting){galleryThumbObserver.unobserve(en.target);queueGalleryThumb(en.target);}},{root:$('fileGalleryResults'),rootMargin:'220px'});cards.forEach(c=>galleryThumbObserver.observe(c));}else cards.slice(0,30).forEach(queueGalleryThumb);}
async function openGalleryPreview(fileId){const item=galleryItems.find(x=>String(x.fileId)===String(fileId));if(!item)return;gallerySelectedItem=item;$('galleryPreviewName').textContent=item.name;$('galleryPreviewMeta').textContent=`Editor: ${item.ownerName||'Editor tidak diketahui'}${item.ownerPc?` · PC: ${item.ownerPc}`:''} · ${fmtSize(item.size||0)} · ${(item.ext||'FILE').toUpperCase()} · ${item.sourceOnline?'online':'offline'}`;const body=$('galleryPreviewBody');body.innerHTML=`<div class="gallery-preview-fileicon">${esc(galleryIcon(item))}<b>Memuat preview...</b></div>`;$('galleryDownloadNow').disabled=!item.sourceOnline;$('galleryPreviewModal').classList.remove('hidden');if(item.previewable&&item.sourceOnline){const data=await requestGalleryPreview(item.fileId);if(gallerySelectedItem?.fileId!==item.fileId)return;if(data){cacheGalleryPreview(item.fileId,data);body.innerHTML=`<img src="${data}" alt="${esc(item.name)}">`;}else body.innerHTML=`<div class="gallery-preview-fileicon">${esc(galleryIcon(item))}<b>Preview tidak tersedia</b></div>`;}else body.innerHTML=`<div class="gallery-preview-fileicon">${esc(galleryIcon(item))}<b>${item.sourceOnline?'Format ini dibuka setelah diunduh':'PC sumber sedang offline'}</b></div>`;}
async function downloadGalleryFile(item,opts={}){const batch=opts.batch===true,directory=opts.directory||'';if(!item?.sourceOnline){if(!batch)showToast('PC sumber sedang offline.');return false;}const req=await socketAckEvent('file-gallery-download-request',{fileId:item.fileId},10000);if(!req?.ok){if(!batch)showToast(req?.error||'Gagal meminta file');return false;}const save=await window.kc.galleryDownloadStart({name:item.name,size:item.size,mtimeMs:item.mtimeMs,fileId:item.fileId,directory});if(!save?.ok){socket.emit('file-gallery-transfer-cancel',{requestId:req.requestId});return false;}let resolveDone;const done=new Promise(resolve=>resolveDone=resolve);galleryDownloadCompletion.set(String(req.requestId),resolveDone);galleryIncomingTransfers.set(req.requestId,{downloadId:save.id,item,path:save.path,batch});const ready=await socketAckEvent('file-gallery-download-ready',{requestId:req.requestId,offset:save.offset||0},10000);if(!ready?.ok){await window.kc.galleryDownloadCancel({id:save.id});galleryIncomingTransfers.delete(req.requestId);galleryDownloadCompletion.delete(String(req.requestId));resolveDone(false);if(!batch)showToast(ready?.error||'PC sumber tidak siap');return false;}if(!batch){const body=$('galleryPreviewBody');if(body)body.innerHTML=`<div style="width:min(520px,90%)"><b>Mengunduh ${esc(item.name)}</b><div class="gallery-progress"><i id="galleryProgressBar"></i></div><small id="galleryProgressText">0%</small></div>`;}return done;}
async function downloadSelectedGalleryFiles(){const items=galleryItems.filter(x=>gallerySelectedIds.has(String(x.fileId))),online=items.filter(x=>x.sourceOnline!==false);if(!items.length)return showToast('Pilih minimal satu file.');if(!online.length)return showToast('Semua file terpilih berasal dari PC yang offline.');const directory=await window.kc.chooseGalleryDownloadFolder?.();if(!directory)return;let ok=0,fail=items.length-online.length;for(let i=0;i<online.length;i++){if($('galleryStats'))$('galleryStats').textContent=`Download batch ${i+1}/${online.length}: ${online[i].name}`;const r=await downloadGalleryFile(online[i],{directory,batch:true});if(r)ok++;else fail++;}gallerySelectedIds.clear();updateGallerySelectionUi();renderFileGallery();showToast(`Download batch selesai: ${ok} berhasil${fail?' · '+fail+' gagal':''}.`,5000);}
async function streamGalleryFileToPeer(req){const id=String(req?.requestId||''),localId=String(req?.localId||'');if(!id||!localId)return;galleryCancelledTransfers.delete(id);let offset=Number(req.offset||0),total=Number(req?.file?.size||0);try{const info=await window.kc.galleryFileInfo(localId);if(!info?.ok)throw Error(info?.error||'Checksum gagal');if(info.size!==total)throw Error('File sumber berubah; Scan Ulang');while(!galleryCancelledTransfers.has(id)){const part=await window.kc.galleryReadChunk({localId,offset,length:512*1024});if(!part?.ok)throw new Error(part?.error||'Gagal membaca file');const result=await socketAckEvent('file-gallery-download-chunk',{requestId:id,offset:Number(part.offset||offset),base64:part.base64||'',eof:!!part.eof,total:Number(part.total||total),sha256:info.sha256},20000);if(!result?.ok)throw new Error(result?.error||'Transfer dihentikan');offset=Number(part.nextOffset??(offset+Math.floor((part.base64||'').length*3/4)));if(part.eof)break;}}catch(e){socket.emit('file-gallery-transfer-cancel',{requestId:id});techLog('gallery-source-transfer-failed',{message:e?.message||String(e)});}finally{galleryCancelledTransfers.delete(id);}}
function bindV1110Socket(){if(!socket||socket._kcV1110Bound)return;socket._kcV1110Bound=true;
  socket.on('status-list',items=>{statusItems=Array.isArray(items)?items:[];renderStatuses();});socket.on('status-updated',p=>{statusItems=Array.isArray(p?.items)?p.items:[];renderStatuses();});
  socket.on('file-gallery-index-updated',()=>{if(!$('fileGalleryModal')?.classList.contains('hidden'))queryFileGallery();});
  socket.on('file-gallery-preview-source',async req=>{const r=await window.kc.galleryPreview(req.localId);socket.emit('file-gallery-preview-response',{requestId:req.requestId,ok:!!r?.ok,dataUrl:r?.dataUrl||'',error:r?.error||''});});
  socket.on('file-gallery-preview-result',p=>{const w=galleryPreviewWaiters.get(String(p?.requestId||''));if(!w)return;galleryPreviewWaiters.delete(String(p.requestId));const data=p?.ok&&p.dataUrl?p.dataUrl:null;w.resolve(data);});
  socket.on('file-gallery-download-source',req=>streamGalleryFileToPeer(req));
  socket.on('file-gallery-download-chunk',async(p,ack=()=>{})=>{const id=String(p?.requestId||''),d=galleryIncomingTransfers.get(id);if(!d)return ack({ok:false,error:'Session download tujuan tidak ada'});try{const w=await window.kc.galleryDownloadWrite({id:d.downloadId,offset:Number(p.offset||0),base64:p.base64||''});if(!w?.ok)throw new Error(w?.error||'Gagal menulis file');const pct=p.total?Math.min(100,Math.round((Number(w.written||0)/Number(p.total))*100)):0;if(d.batch){if($('galleryStats'))$('galleryStats').textContent=`Mengunduh ${d.item?.name||'file'} · ${pct}%`;}else{if($('galleryProgressBar'))$('galleryProgressBar').style.width=pct+'%';if($('galleryProgressText'))$('galleryProgressText').textContent=pct+'% · '+fmtSize(Number(w.written||0))+' / '+fmtSize(Number(p.total||0));}if(p.eof){const fin=await window.kc.galleryDownloadFinish({id:d.downloadId,sha256:p.sha256});galleryIncomingTransfers.delete(id);if(!fin?.ok)throw new Error(fin?.error||'Gagal menyelesaikan file');const done=galleryDownloadCompletion.get(id);galleryDownloadCompletion.delete(id);done?.({ok:true,path:fin.path});if(!d.batch){showToast('File selesai diunduh: '+fin.path,4200);if($('galleryPreviewBody'))$('galleryPreviewBody').innerHTML=`<div class="gallery-preview-fileicon">✓<b>Download selesai</b><small>${esc(fin.path||'')}</small></div>`;}}ack({ok:true,written:w.written});}catch(e){await window.kc.galleryDownloadCancel({id:d.downloadId});galleryIncomingTransfers.delete(id);const done=galleryDownloadCompletion.get(id);galleryDownloadCompletion.delete(id);done?.(false);ack({ok:false,error:e.message||String(e)});if(!d.batch)showToast(e.message||'Download gagal');}});
  socket.on('file-gallery-transfer-cancelled',async p=>{const id=String(p?.requestId||'');galleryCancelledTransfers.add(id);const d=galleryIncomingTransfers.get(id);if(d){if(p.reason!=='disconnect')await window.kc.galleryDownloadCancel({id:d.downloadId});galleryIncomingTransfers.delete(id);const done=galleryDownloadCompletion.get(id);galleryDownloadCompletion.delete(id);done?.(false);if(!d.batch)showToast(p.reason==='disconnect'?'Transfer dijeda. Klik Unduh lagi setelah tersambung untuk melanjutkan.':'Transfer file dibatalkan.');}});
}

$('addTextStatus')?.addEventListener('click',()=>{statusPendingMedia=null;$('statusComposer')?.classList.remove('hidden');$('statusTextInput').placeholder='Tulis status...';$('statusTextInput')?.focus();});
$('addMediaStatus')?.addEventListener('click',()=>$('statusMediaInput')?.click());$('statusMediaInput')?.addEventListener('change',e=>{const f=e.target.files?.[0];if(f)uploadStatusMedia(f);e.target.value='';});$('cancelStatus')?.addEventListener('click',()=>{statusPendingMedia=null;$('statusTextInput').value='';$('statusComposer')?.classList.add('hidden');});$('publishStatus')?.addEventListener('click',publishStatusNow);$('closeStatusViewer')?.addEventListener('click',statusWorkspaceReset);$('closeActivityViewer')?.addEventListener('click',statusWorkspaceReset);$('statusViewerPrev')?.addEventListener('click',()=>{if(statusViewerIndex>0){statusViewerIndex--;renderStatusViewerItem();}});$('statusViewerNext')?.addEventListener('click',()=>{if(statusViewerIndex<statusViewerSequence.length-1){statusViewerIndex++;renderStatusViewerItem();}});$('deleteStatusNow')?.addEventListener('click',async()=>{if(!statusViewerItem)return;const r=await socketAckEvent('status-delete',{id:statusViewerItem.id});if(!r?.ok)return showToast(r?.error||'Gagal hapus status');statusWorkspaceReset();requestStatuses();});$('statusReplySend')?.addEventListener('click',replyStatusNow);$('statusReplyInput')?.addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();replyStatusNow();}});
// v1.11.9: Status is an integrated workspace replacing Chats, never a floating overlay.
$('railActivity')?.addEventListener('click',openStatusWorkspace);
document.querySelector('.wa-rail')?.addEventListener('click',e=>{const b=e.target.closest('button');if(b&&b.id!=='railActivity'&&statusWorkspaceIsOpen())closeStatusWorkspace();},true);
document.addEventListener('keydown',e=>{if(e.key==='Escape'&&statusWorkspaceIsOpen()){e.preventDefault();e.stopImmediatePropagation();closeStatusWorkspace();}},true);
$('railGallery')?.addEventListener('click',()=>{renderGalleryFolders();queryFileGallery();$('fileGalleryModal')?.classList.remove('hidden');});$('closeFileGallery')?.addEventListener('click',()=>$('fileGalleryModal')?.classList.add('hidden'));$('galleryChooseFolders')?.addEventListener('click',async()=>{const picked=await window.kc.chooseGalleryFolders();if(!picked?.length)return;const before=galleryFolders(),seen=new Map(before.map(x=>[String(x).toLowerCase(),x]));for(const f of picked)seen.set(String(f).toLowerCase(),f);cfg.galleryFolders=[...seen.values()];await window.kc.saveConfig({galleryFolders:cfg.galleryFolders});renderGalleryFolders();showToast(`${picked.length} folder dipilih · total ${cfg.galleryFolders.length} folder di PC ini.`);autoScanFileGallery(true,true);});$('galleryRescan')?.addEventListener('click',()=>autoScanFileGallery(true,true));
let gallerySearchTimer=null;if($('gallerySearchScope'))$('gallerySearchScope').value=localStorage.getItem('kc.gallery.searchScope')||'file';$('gallerySearch')?.addEventListener('input',()=>{clearTimeout(gallerySearchTimer);gallerySearchTimer=setTimeout(()=>queryFileGallery(),400);});$('gallerySearchScope')?.addEventListener('change',e=>{localStorage.setItem('kc.gallery.searchScope',e.target.value||'file');queryFileGallery();});$('galleryKind')?.addEventListener('change',()=>queryFileGallery());$('gallerySort')?.addEventListener('change',()=>queryFileGallery());$('galleryGroup')?.addEventListener('change',renderFileGallery);$('galleryLoadMore')?.addEventListener('click',()=>queryFileGallery({append:true}));$('gallerySelectVisible')?.addEventListener('click',()=>{galleryItems.forEach(x=>gallerySelectedIds.add(String(x.fileId)));updateGallerySelectionUi();});$('galleryClearSelection')?.addEventListener('click',()=>{gallerySelectedIds.clear();updateGallerySelectionUi();});$('galleryDownloadSelected')?.addEventListener('click',downloadSelectedGalleryFiles);document.querySelectorAll('[data-gallery-view]').forEach(b=>b.addEventListener('click',()=>{galleryView=b.dataset.galleryView||'large';document.querySelectorAll('[data-gallery-view]').forEach(x=>x.classList.toggle('active',x===b));renderFileGallery();}));$('closeGalleryPreview')?.addEventListener('click',()=>$('galleryPreviewModal')?.classList.add('hidden'));$('galleryDownloadNow')?.addEventListener('click',()=>downloadGalleryFile(gallerySelectedItem));


$('groupAvatarInput')?.addEventListener('change',async e=>{const f=e.target.files?.[0],c=groupAvatarTarget;e.target.value='';if(!f||!c)return;if(f.size>10*1024*1024)return showToast('Foto grup maksimal 10 MB');try{const fd=new FormData();fd.append('avatar',f);const r=await kcFetch(serverBase+'/group-avatar/upload',{method:'POST',body:fd}),j=await r.json();if(!r.ok||!j.ok)throw new Error(j.error||'Upload foto grup gagal');const evt=c.type==='general'?'conversation-general-update':'conversation-group-update';const u=await socketAckEvent(evt,{id:c.id,avatarUrl:j.avatarUrl});if(!u?.ok)throw new Error(u?.error||'Gagal mengubah foto grup');showToast('Foto grup diperbarui.');$('chatInfoModal')?.classList.add('hidden');requestConversationList();setTimeout(()=>openConversation(c.id),100);}catch(err){showToast(err.message||'Gagal mengubah foto grup');}finally{groupAvatarTarget=null;}});
// ================= end v1.11.0 =================

window.kc?.onNetworkRecovery?.(()=>{try{requestFreshDiscovery('electron-network-service-recovery',450);}catch(e){techLog('network-service-recovery-failed',{message:e?.message||String(e)});}});
window.kc?.onNetworkInterfaceChange?.(info=>{try{requestFreshDiscovery(info?.initial?'startup-network-ready':'network-interface-change',350);}catch(e){techLog('network-interface-recovery-failed',{message:e?.message||String(e)});}});
window.kc?.onSystemResume?.(()=>{try{requestFreshDiscovery('system-resume',650);}catch(e){techLog('system-resume-recovery-failed',{message:e?.message||String(e)});}});
window.addEventListener('online',()=>requestFreshDiscovery('browser-online',350));
$('repairConnection')?.addEventListener('click',()=>requestFreshDiscovery('manual-repair',0));
const connectionWatchdogTimer=setInterval(()=>{
  if(!cfg||!cfg.name||(socket?.connected&&sessionReady))return;
  const offlineFor=Date.now()-(disconnectedSince||Date.now());
  if(offlineFor<20000||Date.now()-lastWatchdogRecoveryAt<20000)return;
  lastWatchdogRecoveryAt=Date.now();
  techLog('connection-watchdog-forced-discovery',{offlineFor,hasSocket:!!socket,serverBase:serverBase||null});
  requestFreshDiscovery('connection-watchdog',0);
},5000);
window.addEventListener('beforeunload',()=>clearInterval(connectionWatchdogTimer),{once:true});
async function bootstrapRenderer(){
  void refreshComputeHints();
  if(__kcRendererBooted)return;
  __kcRendererBooted=true;
  try{
    techLog('renderer-bootstrap-before-init');
    await init();
    techLog('renderer-bootstrap-success',{isHost:cfg?.isHost,role:cfg?.role,serverBase});
  }catch(e){
    __kcRendererBooted=false;
    try{reportBug('renderer-bootstrap-failed',e,'fatal',{phase:'init'});}catch(_){ }
    const badge=$('runtimeRoleBadge');
    if(badge){badge.textContent='STARTUP ERROR · klik untuk detail';badge.classList.add('client');badge.title=String(e?.stack||e?.message||e||'Startup error');}
    const conn=$('connectionText');if(conn)conn.textContent='Startup aplikasi gagal';
    const status=$('status');if(status){status.textContent='● Startup error: '+String(e?.message||e||'Unknown');status.style.color='#ff6577';}
    console.error('[KC] renderer bootstrap failed',e);
  }
}
if(document.readyState==='loading') document.addEventListener('DOMContentLoaded',bootstrapRenderer,{once:true});
else bootstrapRenderer();


// v1.8.4 — notification quick reply without stealing the chat window focus.
window.kc?.onNotificationReply?.(payload=>{
  const text=String(payload?.text||'').trim(),conversationId=String(payload?.conversationId||'general');
  if(!text||!socket?.connected)return;
  socketSend({text,conversationId}).catch(e=>showToast(e.message));
});

// v1.8.4 — ESC behaves as a desktop Back key: close the topmost surface first.
document.addEventListener('keydown',e=>{
  if(e.key!=='Escape')return;
  if(typeof window!=='undefined'&&window.KCShippingSelects?.closeOpen(true)){e.preventDefault();e.stopImmediatePropagation();return;}
  if($('adminModal')?.open){
    e.preventDefault();
    e.stopImmediatePropagation();
    closeModeration();
    return;
  }
  if($('messageReactionPicker')){e.preventDefault();e.stopImmediatePropagation();window.KCChatReactions?.close();return;}
  const ids=['mentionPicker','messageContextMenu','conversationMenu','hostQuickMenu','toolDock'];
  for(const id of ids){const x=$(id);if(x&&!x.classList.contains('hidden')){e.preventDefault();e.stopImmediatePropagation();x.classList.add('hidden');return;}}
  const visible=[...document.querySelectorAll('.modal:not(.hidden)')].reverse()[0];
  if(visible){e.preventDefault();e.stopImmediatePropagation();visible.classList.add('hidden');return;}
  if(currentReply||editingMessage){e.preventDefault();e.stopImmediatePropagation();clearReply();return;}
  if(selectedMessageIds.size){e.preventDefault();e.stopImmediatePropagation();clearMessageSelection();return;}
  if(activeConversationId){e.preventDefault();e.stopImmediatePropagation();closeActiveConversation();}
},true);

// v1.9.1 turn countdown refresh
setInterval(()=>{if(currentGame?.status==='playing'&&!$('gameModal')?.classList.contains('hidden')){const c=document.querySelector('.turn-clock');if(c)c.textContent='⏱ '+gameSecondsLeft(currentGame)+'s';}},500);

// v1.9.1 local font selector
$('appFontFamily')?.addEventListener('change',e=>{appearancePrefs.fontFamily=e.target.value||'SF Pro Display';saveAppearancePrefs();});$('scanFonts')?.addEventListener('click',async()=>{const sel=$('appFontFamily');if(!sel)return;try{if(!window.queryLocalFonts)throw new Error('Browser Windows ini belum mendukung daftar font otomatis');const fonts=await window.queryLocalFonts();const fam=[...new Set(fonts.map(f=>f.family).filter(Boolean))].sort((a,b)=>a.localeCompare(b));sel.innerHTML=fam.map(x=>`<option>${esc(x)}</option>`).join('');sel.value=fam.includes(appearancePrefs.fontFamily)?appearancePrefs.fontFamily:(fam.includes('SF Pro Display')?'SF Pro Display':fam.includes('Segoe UI')?'Segoe UI':fam[0]);appearancePrefs.fontFamily=sel.value;saveAppearancePrefs();showToast(fam.length+' font PC ditemukan.');}catch(e){showToast(e.message||'Daftar font tidak tersedia');}});

function sendDetachedLudoAction(event,payload){socket.timeout(4000).emit(event,payload,(error,r)=>{if(error){gameFeedback(false,'Konfirmasi terlambat; mengambil posisi terbaru…');socket.timeout(4000).emit('game-state-request',{id:payload.id},(e,fresh)=>{if(!e&&fresh?.ok){currentGame=fresh.state;syncDetachedGame(currentGame)}else gameFeedback(false,'Koneksi belum pulih. Klik reconnect.');});return;}gameFeedback(!!r?.ok,r?.ok?'Langkah dikonfirmasi':r?.error||'Langkah ditolak');if(r?.state){currentGame=r.state;syncDetachedGame(currentGame)}});}



$('diagVersion')?.addEventListener('change',()=>refreshDiagnostics());
$('chatEmptyState')?.addEventListener('click', event => {
  const shortcut = event.target.closest('[data-chat-shortcut]');
  if (!shortcut) return;
  const target = $(shortcut.dataset.chatShortcut);
  if (target instanceof HTMLInputElement) target.focus();
  else target?.click();
});
