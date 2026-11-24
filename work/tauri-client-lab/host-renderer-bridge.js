(() => {
  const invoke = (...args) => window.__TAURI__.core.invoke(...args);
  const {base64Blob} = kcDesktopDesign;
  const galleryDownload = (action,input) => invoke('gallery_download',{action,input})
    .catch(error => ({ok:false,error:String(error)}));
  let galleryScanFixtureGate;
  let reportedStartupError;
  const galleryLocal = async (action,input={}) => {
    if (galleryScanFixtureGate) await galleryScanFixtureGate(action);
    return invoke('gallery_local',{action,input}).catch(error => ({ok:false,error:String(error)}));
  };
  const read = (key, fallback) => JSON.parse(localStorage.getItem(key) ?? 'null') ?? fallback;
  const save = (key, value) => localStorage.setItem(key, JSON.stringify(value));
  const {electronHostProfileId, ...legacyProfile} = window.__kcLegacyDesktopProfile || {};
  const profileScope = electronHostProfileId ? ':' + encodeURIComponent(electronHostProfileId) : '';
  const profileKey = 'kc-rust-host-profile' + profileScope;
  const outboxKey = 'kc-rust-host-outbox' + profileScope;
  const profile = read(profileKey, {});
  if (window.__kcLegacyDesktopProfile && !profile.legacyDesktopProfileImported) {
    for (const [key, value] of Object.entries(legacyProfile)) {
      if (profile[key] === undefined) profile[key] = value;
    }
    profile.legacyDesktopProfileImported = true;
  }
  profile.deviceId ??= crypto.randomUUID();
  profile.deviceKey ??= Array.from(crypto.getRandomValues(new Uint8Array(32)), byte => byte.toString(16).padStart(2, '0')).join('');
  save(profileKey, profile);
  const rows = scope => read(outboxKey, []).filter(row => row.scope === scope);
  const update = change => save(outboxKey, change(read(outboxKey, [])));
  window.kc = Object.freeze({
    saveChatWallpaper: ({dataUrl}) => invoke('chat_wallpaper',{action:'save',dataUrl}),
    downloadChatFile: ({url,name}) => invoke('download_host_attachment',{url,name}),
    getClipboardImage: () => invoke('read_clipboard_image'),
    getLocalActivity: () => invoke('get_local_activity'),
    onSystemResume: callback => window.__TAURI__.event.listen('kc-system-resume', () => callback()).catch(() => {}),
    onNetworkInterfaceChange: callback => window.__TAURI__.event.listen('kc-network-interface-change', () => callback({initial:false})).catch(() => {}),
    loadChatWallpaper: () => invoke('chat_wallpaper',{action:'load'}),
    clearChatWallpaper: () => invoke('chat_wallpaper',{action:'clear'}),
    supportsRustClientUpdates: true,
    getConfig: async () => ({
      ...read(profileKey, {}), ...(window.__kcHostSmoke ? { name: 'HOST Lab' } : {}),
      isHost: true, role: 'server', version: '1.1.43',
      adminToken: (await invoke('host_status')).adminToken || null,
      buildRoleSource: window.__kcOfficeHost ? 'Andromeda · Rust' : 'Andromeda · Rust · Uji lokal',
      productName: 'ChatX',
    }),
    saveConfig: async input => {
      if ('backupKeep' in input || 'autoBackup' in input) {
        await invoke('backup_config',{action:'settings',input:{backupKeep:input.backupKeep,autoBackup:input.autoBackup}});
      }
      const update = Object.fromEntries(['name', 'teamRole', 'workTagPrefs', 'notificationSettings', 'galleryFolders']
        .filter(key => input[key] !== undefined).map(key => [key, input[key]]));
      save(profileKey, { ...read(profileKey, {}), ...update });
    },
    sendNotification: payload => invoke('send_notification', { payload, settings: read(profileKey, {}).notificationSettings || {} }),
    logEvent: input => invoke('support_event', {input}),
    backupSpace: input => invoke('backup_space',{input}).catch(error=>({ok:false,error:String(error)})),
    hostStorageInventory: deleteBackup => invoke('host_storage_inventory', {deleteBackup}),
    getSystemStats: () => invoke('backup_config',{action:'stats',input:{}}),
    chooseStorageFolder: () => invoke('choose_storage_folder').catch(error=>{
      showToast(String(error));
      return null;
    }),
    chooseBackupFolder: () => invoke('backup_config',{action:'choose',input:{}}).catch(error=>{
      showToast(String(error));
      return null;
    }),
    runBackupNow: () => invoke('backup_create',{preUpdate:false}).catch(error=>({ok:false,error:String(error)})),
    preUpdateBackup: () => invoke('backup_create',{preUpdate:true}).catch(error=>({ok:false,error:String(error)})),
    verifyLatestBackup: () => invoke('backup_recovery',{action:'verify'}).catch(error=>({ok:false,error:String(error)})),
    restoreBackup: () => invoke('backup_recovery',{action:'restore'}).catch(error=>({ok:false,error:String(error)})),
    chooseGalleryDownloadFolder: () => invoke('gallery_choose_download_folder').catch(() => null),
    galleryDownloadStart: input => galleryDownload('start',input),
    galleryDownloadWrite: input => galleryDownload('write',input),
    galleryDownloadFinish: input => galleryDownload('finish',input),
    galleryDownloadCancel: input => galleryDownload('cancel',input),
    chooseGalleryFolders: async () => {
      const folders = await galleryLocal('choose');
      return Array.isArray(folders) ? folders : [];
    },
    scanGalleryFolders: input => galleryLocal('scan',input),
    galleryIndexLocalStatus: () => galleryLocal('status'),
    galleryIndexLocalChanges: input => galleryLocal('changes',input),
    galleryIndexLocalExport: input => galleryLocal('export',input),
    galleryIndexLocalAck: input => galleryLocal('ack',input),
    galleryPreview: localId => galleryLocal('preview',{localId}),
    galleryFileInfo: localId => galleryLocal('info',{localId}),
    galleryReadChunk: input => galleryLocal('chunk',input),
    openLogFolder: () => invoke('open_support_folder').catch(error => ({ok:false,error:String(error)})),
    exportSupportLog: () => {
      const {name,deviceId,notificationSettings} = read(profileKey,{});
      return invoke('export_support_log', {config:{name,deviceId,notificationSettings,role:'server'},exportedAt:new Date().toISOString()})
        .catch(error => ({ok:false,error:String(error)}));
    },
    reportBug: report => invoke('queue_bug_report', {
      report: {...report,role:'host',version:'1.1.43',user:read(profileKey,{}).name,deviceId:profile.deviceId},
    }).catch(error => {
      window.kc.logEvent({type:'bug-queue-write-failed',detail:{error:String(error)}}).catch(() => {});
      return {ok:false,error:String(error)};
    }),
    onNotificationReply: callback => window.addEventListener('kc-notification-reply', event => callback(event.detail)),
    onFocusMessage: callback => window.addEventListener('kc-notification-focus', event => callback(event.detail)),
    restart: () => location.reload(),
    openGameWindow: () => invoke('open_game').catch(error => { console.error(error); return false; }),
    syncGameState: payload => invoke('game_window_sync', {kind:'state',payload}).catch(console.error),
    syncGameFeedback: payload => invoke('game_window_sync', {kind:'feedback',payload}).catch(console.error),
    onGameAction: callback => window.addEventListener('kc-game-action', event => callback(event.detail)),
    notifyGameInvite: ({title,body}) => window.kc.sendNotification({
      kind:'game-invite',title,body,theme:document.documentElement.dataset.theme || 'dark',
    }),
    onFocusGameInvite: callback => window.addEventListener('kc-focus-game-invite', callback),
    copyChatText: text => invoke('copy_local_text', { text }),
    setClipboardPng: png => invoke('copy_chat_image', { input: { png } }),
    ...kcDesktopDesign.methods,
    openServerData: () => invoke('open_server_data').catch(error => ({ok:false,error:String(error)})),
    runHealthCheck: () => kcDesktopDesign.methods.runHealthCheck('HOST', async () => {
      const status = await invoke('host_status');
      if (status.state !== 'running') return {ok:false, detail:status.error || 'Server belum aktif'};
      const response = await fetch(new URL('/health', status.address), {signal:AbortSignal.timeout(2500)});
      return {ok:response.ok, detail:response.ok ? status.address : 'HTTP '+response.status};
    }),
    loadHostModeration: () => invoke('host_moderation_snapshot'),
    startHostPrimary: async () => {
      for (let attempt = 0; attempt < 80; attempt++) {
        const state = await invoke('host_status');
        if (state.state === 'running') return { ok: true, address: state.address };
        if (state.state === 'failed') {
          if (reportedStartupError !== state.error && typeof window.showToast === 'function') {
            reportedStartupError = state.error;
            window.showToast('HOST gagal menyala: ' + state.error, 10000);
          }
          return { ok: false, error: state.error };
        }
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      return { ok: false, error: 'HOST belum siap' };
    },
    outboxList: async scope => rows(scope),
    outboxPut: async ({ scope, payload }) => update(all => [
      ...all.filter(row => row.scope !== scope || row.payload.clientMessageId !== payload.clientMessageId),
      { scope, payload, state: 'waiting', attempts: 0 },
    ]),
    outboxUpdate: async ({ scope, id, patch }) => update(all => all.map(row => (
      row.scope === scope && row.payload.clientMessageId === id ? { ...row, ...patch } : row
    ))),
    outboxRemove: async ({ scope, id }) => update(all => all.filter(row => (
      row.scope !== scope || row.payload.clientMessageId !== id
    ))),
  });
})();
