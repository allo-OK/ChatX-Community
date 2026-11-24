(() => {
  const { invoke } = window.__TAURI__.core;
  const read = key => { try { return JSON.parse(localStorage.getItem(key)) ?? {}; } catch { return {}; } };
  const profileKey = 'kc-rust-client-prefs';
  const outboxKey = 'kc-rust-client-outbox';
  const discover = () => invoke('discover_primary_server');
  const outbox = () => { const rows = read(outboxKey); return Array.isArray(rows) ? rows : []; };
  const saveOutbox = rows => localStorage.setItem(outboxKey, JSON.stringify(rows));
  const galleryDownload = (action,input) => invoke('gallery_download',{action,input})
    .catch(error => ({ok:false,error:String(error)}));
  const galleryLocal = (action,input={}) => invoke('gallery_local',{action,input})
    .catch(error => ({ok:false,error:String(error)}));
  window.kc = Object.freeze({
    saveChatWallpaper: ({dataUrl}) => invoke('chat_wallpaper',{action:'save',dataUrl}),
    getClipboardImage: () => invoke('read_clipboard_image'),
    getLocalActivity: () => invoke('get_local_activity'),
    onSystemResume: callback => window.__TAURI__.event.listen('kc-system-resume', () => callback()).catch(() => {}),
    onNetworkInterfaceChange: callback => window.__TAURI__.event.listen('kc-network-interface-change', () => callback({initial:false})).catch(() => {}),
    loadChatWallpaper: () => invoke('chat_wallpaper',{action:'load'}),
    clearChatWallpaper: () => invoke('chat_wallpaper',{action:'clear'}),
    ...kcDesktopDesign.methods,
    openServerData: async () => ({ok:false,error:'Folder data hanya tersedia di PC server'}),
    runHealthCheck: () => kcDesktopDesign.methods.runHealthCheck('CLIENT'),
    getConfig: async () => {
      const settings = read(profileKey);
      const identity = await invoke('client_device_identity');
      return {...settings, ...identity, galleryFolders:settings.galleryFolders ?? identity?.galleryFolders ?? [],
        role:'client', isHost:false, pc:await invoke('client_computer_name'),
        version:'1.1.43', productName:'ChatX', buildRoleSource:'Andromeda · Rust'};
    },
    saveConfig: async values => {
      if (values.role === 'client' && values.name) await invoke('create_client_identity',{name:values.name});
      const settings = Object.fromEntries(['displayName','teamRole','workTagPrefs','notificationSettings',
        'performanceProfile','manualServerAddress','manualServerPcName','galleryFolders']
        .filter(key=>values[key] !== undefined).map(key=>[key,values[key]]));
      localStorage.setItem(profileKey,JSON.stringify({...read(profileKey),...settings}));
    },
    restart: () => location.reload(),
    checkRustClientUpdate: async () => {
      const result = await invoke('refresh_client_update');
      window.kcClientUpdate?.apply(result);
      return result.locked ? null : result.status;
    },
    onClientUpdateProgress: callback => {
      window.__TAURI__.event.listen('client-update-progress', ({payload}) => callback(payload))
        .catch(error => console.error('Progres update CLIENT tidak tersedia', error));
    },
    installClientUpdate: async ({manifest}) => {
      try {
        await invoke('prepare_client_update', {expectedManifest:manifest});
        return await invoke('open_client_update');
      } catch (error) {
        return {ok:false,error:String(error)};
      }
    },
    refreshFeaturePolicy: () => invoke('refresh_client_feature_policy').catch(() => null),
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
    setClipboardPng: png => invoke('client_copy_png',{png}),
    sendNotification: payload => invoke('client_notification',{payload,settings:read(profileKey).notificationSettings || {}}),
    onNotificationReply: callback => window.addEventListener('kc-notification-reply',event=>callback(event.detail)),
    onFocusMessage: callback => window.addEventListener('kc-notification-focus',event=>callback(event.detail)),
    openGameWindow: () => invoke('open_game').catch(error => { console.error(error); return false; }),
    syncGameState: payload => invoke('game_window_sync', {kind:'state',payload}).catch(console.error),
    syncGameFeedback: payload => invoke('game_window_sync', {kind:'feedback',payload}).catch(console.error),
    onGameAction: callback => window.addEventListener('kc-game-action', event => callback(event.detail)),
    notifyGameInvite: ({title,body}) => window.kc.sendNotification({
      kind:'game-invite',title,body,theme:document.documentElement.dataset.theme || 'dark',
    }),
    onFocusGameInvite: callback => window.addEventListener('kc-focus-game-invite', callback),
    downloadChatFile: ({url,name}) => invoke('download_client_attachment',{url,name})
      .catch(error=>({ok:false,error:String(error)})),
    discoverPrimaryServer: discover,
    discover,
    probePrimaryServer: address => invoke('probe_client_host',{address}),
    discoverServers: async () => {
      const address = await discover();
      if (!address) return [];
      const response = await fetch(address+'/health');
      if (!response.ok) return [];
      const health = await response.json();
      return [{...health,address}];
    },
    trustHostKey: ({origin,publicKey,fingerprint,token}) => invoke('remember_client_session',
      {origin,key:publicKey,fingerprint,token}).catch(error=>({ok:false,error:String(error)})),
    outboxRebind: async ({publicKey}) => {
      const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(publicKey));
      const scope = 'host-key:' + Array.from(new Uint8Array(hash), b=>b.toString(16).padStart(2,'0')).join('');
      return {ok:true,scope};
    },
    outboxPut: async ({scope,payload}) => {
      const rows = outbox();
      const id = payload.clientMessageId;
      if (!rows.some(row=>row.scope===scope && row.payload.clientMessageId===id)) rows.push({scope,payload,state:'waiting',attempts:0});
      saveOutbox(rows);
      return {ok:true};
    },
    outboxList: async scope => outbox().filter(row=>row.scope===scope),
    outboxUpdate: async ({scope,id,patch}) => saveOutbox(outbox().map(row =>
      row.scope===scope && row.payload.clientMessageId===id ? {...row,...patch} : row)),
    outboxRemove: async ({scope,id}) => {
      saveOutbox(outbox().filter(row=>row.scope!==scope || row.payload.clientMessageId!==id));
      return Promise.resolve({ok:true});
    },
  });
})();
