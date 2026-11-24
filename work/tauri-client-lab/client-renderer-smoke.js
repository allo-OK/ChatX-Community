(() => {
  const { invoke } = window.__TAURI__.core;
  const sample = 'CLIENT Rust renderer LAN fixture';
  const queued = 'CLIENT Rust renderer reconnect fixture';
  const started = performance.now();
  let phase = 'initial';
  let working = false;
  const finish = error => {
    clearInterval(probe);
    return invoke('finish_chat_test', {error: error ?? null});
  };
  const sendFromComposer = text => {
    const input = document.getElementById('text');
    input.value = text;
    input.dispatchEvent(new Event('input', {bubbles:true}));
    document.getElementById('send').click();
  };
  const contains = text => document.getElementById('messages')?.textContent.includes(text);
  const probe = setInterval(async () => {
    if (working) return;
    working = true;
    try {
      if (performance.now() - started > 26000) {
        await finish(`CLIENT ${phase}: ${document.getElementById('connectionText')?.textContent}; ${document.getElementById('runtimeRoleBadge')?.title}`);
        return;
      }
      if (window.__kcFreshClientSmoke && phase === 'initial' && document.getElementById('setup')?.classList.contains('hidden') === false) {
        document.getElementById('setupName').value = 'Client UI Fresh Fixture';
        document.getElementById('setupTeamRole').value = 'QC';
        phase = 'setup';
        document.getElementById('clientRole').click();
        return;
      }
      if (phase === 'offline' && !socket?.connected && !sessionReady) {
        sendFromComposer(queued);
        const rows = await window.kc.outboxList(outboxScope());
        if (!rows.some(row=>row.payload.text === queued)) throw Error('Pesan offline tidak tersimpan dalam antrean.');
        phase = 'queued';
        return;
      }
      if (!document.getElementById('statusHost')?.classList.contains('ok')) return;
      const general = document.querySelector('#conversationList [data-id="general"]');
      if (!general) return;
      if (!general.classList.contains('active')) { general.click(); return; }
      if (phase === 'initial') {
        if (!window.__kcFreshClientSmoke &&
            (!document.querySelector('#conversationList [data-id="client-import-dm"]') ||
             !document.querySelector('#conversationList [data-id="client-import-group"]'))) {
          throw Error('Grup atau DM lama hilang dari profil CLIENT hasil migrasi.');
        }
        if (!window.__kcFreshClientSmoke && (document.documentElement.dataset.theme !== 'light'
            || !conversationPrefs.pins.includes('general'))) {
          throw Error('Tema atau pin chat CLIENT lama tidak dimuat.');
        }
        phase = 'sent';
        sendFromComposer(sample);
      } else if (phase === 'sent' && contains(sample)) {
        phase = 'offline';
        socket.io.engine.close();
      } else if (phase === 'queued' && contains(queued) && (await window.kc.outboxList(outboxScope())).length === 0) {
        if (!window.__kcFreshClientSmoke && document.documentElement.dataset.theme !== 'light') {
          throw Error('Tema CLIENT berubah setelah reconnect.');
        }
        const image = document.querySelector('[data-message-id="client-chat-image"]');
        const copy = image?.querySelector('[data-copy-image]');
        if (!copy) throw Error('Tombol Salin gambar chat CLIENT tidak muncul.');
        copy.click();
        const deadline = performance.now()+2500;
        while (performance.now()<deadline && document.getElementById('kcToast')?.textContent !== 'Gambar disalin ke clipboard.') {
          await new Promise(resolve=>setTimeout(resolve,50));
        }
        if (document.getElementById('kcToast')?.textContent !== 'Gambar disalin ke clipboard.') {
          throw Error('Salin gambar chat gagal: '+document.getElementById('kcToast')?.textContent);
        }
        const download = image.querySelector('[data-chat-download]');
        if (!download) throw Error('Tombol Unduh lampiran tidak muncul.');
        await download.onclick(new MouseEvent('click',{bubbles:true,cancelable:true}));
        if (!document.getElementById('kcToast')?.textContent.startsWith('File disimpan:')) {
          throw Error('Unduh lampiran CLIENT gagal: '+document.getElementById('kcToast')?.textContent);
        }
        const waitEvent = name => new Promise((resolve,reject) => {
          const timer = setTimeout(()=>{window.removeEventListener(name,done);reject(Error('Jendela CLIENT tidak merespons: '+name));},10000);
          const done = event => {clearTimeout(timer);resolve(event.detail);};
          window.addEventListener(name,done,{once:true});
        });
        const reply = waitEvent('kc-notification-reply');
        const payload = {title:'Fixture',body:'Pesan langsung untuk uji notifikasi',messageId:'client-popup-reply',conversationId:'general'};
        const first = await window.kc.sendNotification(payload);
        const duplicate = await window.kc.sendNotification(payload);
        if (!first.accepted || !duplicate.duplicate) throw Error('Deduplikasi popup CLIENT gagal.');
        if ((await reply).text !== 'Balasan popup uji') throw Error('Balasan popup CLIENT tidak cocok.');
        const response = await kcFetch(serverBase+'/bledar-videos/pick');
        const selected = await response.json();
        if (!selected.video?.id?.startsWith('custom-')) throw Error('Video kustom untuk popup CLIENT belum tersedia.');
        const focused = waitEvent('kc-notification-focus');
        await window.kc.sendNotification({...payload,body:'Uji video CLIENT',messageId:'lab-notification-focus',mention:true,
          mentionLevel:'fullscreen',tagVideoEnabled:true,bledarVideo:{...selected.video,url:fullUrl(selected.video.url)}});
        await focused;
        const played = waitEvent('kc-game-smoke-complete');
        document.getElementById('railGame').click();
        document.getElementById('chessColorPref').value = 'white';
        document.querySelector('#gameInviteModal [data-game="chess"]').click();
        const gameResult = await played;
        if (!gameResult?.ok) throw Error(gameResult?.error || 'Game CLIENT gagal.');
        await finish();
      }
    } catch (error) {
      await finish(String(error?.message || error));
    } finally {
      working = false;
    }
  }, 80);
})();
