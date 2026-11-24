(() => {
  'use strict';
  const invoke = (...args) => window.__TAURI__.core.invoke(...args);
  const byId = id => document.getElementById(id);
  const showError = error => {
    byId('error').textContent = String(error?.message || error);
    byId('error').hidden = false;
  };
  let working = false;
  async function action(name, text) {
    if (working) return;
    working = true;
    try { await invoke('notification_action', { action: name, text }); }
    catch (error) { showError(error); }
    finally { working = false; }
  }
  function attachment(raw, trustedOrigin, prefix) {
    try {
      const url = new URL(raw);
      return url.origin === trustedOrigin && !url.username && !url.password && (!prefix || url.pathname.startsWith(prefix)) ? url.href : '';
    } catch { return ''; }
  }
  function render({ payload, settings }) {
    document.documentElement.dataset.theme = payload.theme === 'light' ? 'light' : 'dark';
    if (/^#[a-f\d]{6}$/i.test(payload.accent)) document.documentElement.style.setProperty('--accent', payload.accent);
    const bledar = payload.mention && payload.tagVideoEnabled !== false && (payload.tagVideoEnabled === true || payload.mentionLevel === 'fullscreen');
    const rows = Array.isArray(payload.messages) && payload.messages.length ? payload.messages.slice(-20)
      : [{ text: payload.body || 'Pesan baru', imageUrl: payload.imageUrl }];
    const kind = byId('kind');
    kind.classList.toggle('mention', Boolean(payload.mention));
    const tag = payload.tagLabel || (payload.tagScope === 'private' ? 'TAG PRIBADI' : 'TAG GRUP');
    const gameInvite = payload.kind === 'game-invite';
    kind.textContent = (gameInvite ? 'UNDANGAN GAME' : payload.mention ? (bledar ? 'BLEDAR · ' : 'TAG · ') + tag : 'PESAN BARU')
      + (rows.length > 1 ? ' · ' + rows.length + ' PESAN' : '');
    byId('sender').textContent = payload.title || 'ChatX';
    const group = payload.conversationType === 'group' || payload.conversationType === 'general';
    byId('source').hidden = !group;
    byId('source').textContent = group ? 'Grup · ' + (payload.conversationName || 'Grup kantor') : '';
    if (gameInvite) {
      byId('quickReply').style.display = 'none';
      byId('openTarget').setAttribute('aria-label', 'Buka undangan');
    }
    const messages = byId('messages');
    const visibleRows = payload.mention || gameInvite ? rows : rows.slice(-1);
    if (visibleRows.length < rows.length) {
      const summary = document.createElement('div');
      summary.className = 'bubble';
      summary.textContent = rows.slice(0,-1).map(row => {
        const snippet = String(row.text || row.fileName || 'Pesan').replace(/\s+/g,' ').slice(0,72);
        return (row.sender ? row.sender+': ' : '')+snippet;
      }).join(' · ');
      messages.append(summary);
    }
    for (const row of visibleRows) {
      const bubble = document.createElement('div');
      bubble.className = 'bubble';
      bubble.textContent = row.text || row.fileName || 'Pesan';
      const url = attachment(row.imageUrl, payload._mediaOrigin);
      if (url) {
        const image = document.createElement('img');
        image.alt = row.fileName || 'Lampiran pesan';
        image.onerror = () => image.remove();
        image.src = url;
        bubble.append(image);
      }
      messages.append(bubble);
    }
    messages.scrollTop = messages.scrollHeight;
    let playVideo;
    if (bledar) {
      const video = byId('video'), status = byId('videoStatus'), retry = byId('retryVideo');
      video.muted = payload.tagSoundEnabled === false;
      byId('media').hidden = false;
      const remote = attachment(payload.bledarVideo?.url, payload._mediaOrigin, '/bledar-videos/media/');
      video.src = remote || 'notification-media/default.mp4';
      video.onplaying = () => { status.hidden = retry.hidden = true; };
      video.onerror = () => {
        status.textContent = 'Video belum bisa diputar.';
        status.hidden = retry.hidden = false;
      };
      playVideo = () => video.play().catch(error => {
        if (error.name !== 'NotAllowedError') { video.onerror(); return; }
        video.muted = true;
        return video.play().catch(video.onerror);
      });
      retry.onclick = () => { video.muted = payload.tagSoundEnabled === false; void playVideo(); };
    }
    const sound = payload._quiet || (payload.mention && payload.tagSoundEnabled === false) ? null : payload.mention
      ? ({ standard:'classic', notice:'bell' }[payload.mentionLevel] || 'mention')
      : settings.normalSound;
    const audio = ({ classic:'classic.wav', iphone:'iphone.mp3', bell:'bell.wav', pop:'pop.wav', mention:'mention.wav' })[sound];
    return () => {
      playVideo?.();
      if (audio) new Audio('notification-media/' + audio).play().catch(() => {});
    };
  }
  byId('close').onclick = () => action('close');
  document.querySelector('main').onclick = event => {
    if (event.target.closest('form, button')) return;
    if (window.getSelection()?.toString()) return;
    void action('focus');
  };
  byId('openTarget').onkeydown = event => {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    if (!event.repeat) void action('focus');
  };
  byId('quickReply').onsubmit = event => {
    event.preventDefault();
    const text = byId('replyText').value.trim();
    if (text) void action('reply', text);
  };
  document.addEventListener('keydown', event => { if (event.key === 'Escape') void action('close'); });
  void invoke('notification_snapshot').then(async data => {
    const startMedia = render(data);
    if (window.__kcNotificationSmoke) {
      const card = getComputedStyle(document.querySelector('main'));
      const body = getComputedStyle(document.body);
      if (parseFloat(card.borderTopLeftRadius) < 18 || card.overflow !== 'hidden'
          || body.backgroundColor !== 'rgba(0, 0, 0, 0)' || getComputedStyle(byId('video')).objectFit !== 'cover') {
        throw Error('Sudut popup notifikasi belum rounded atau latarnya belum transparan.');
      }
      if (data.payload.conversationType === 'group' && (byId('source').hidden
          || !byId('source').textContent.includes(data.payload.conversationName)
          || byId('sender').textContent !== data.payload.title)) throw Error('Asal grup atau pengirim tidak muncul.');
      if (data.payload.tagSoundEnabled === false && !byId('video').muted) throw Error('Video tanpa suara belum dimute.');
    }
    await invoke('notification_ready');
    startMedia();
    if (window.__kcNotificationSmoke && (data.payload.messageId === 'lab-notification-reply'
      || data.payload.body === 'Pesan langsung untuk uji notifikasi')) {
      byId('replyText').value = 'Balasan popup uji';
      byId('quickReply').requestSubmit();
    } else if (window.__kcNotificationSmoke && data.payload.kind === 'game-invite') {
      byId('messages').click();
    } else if (window.__kcNotificationSmoke && (data.payload.messageId === 'lab-notification-focus'
      || data.payload.body === '@HOST Lab uji video tag langsung')
      && byId('kind').textContent.includes('BLEDAR') && !byId('media').hidden) {
      const video = byId('video');
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(Error('Video BLEDAR tidak mulai diputar.')), 8000);
        const playing = () => {
          if (video.currentTime > 0 && video.videoWidth > 0 && !video.paused) {
            clearTimeout(timer);
            video.removeEventListener('timeupdate', playing);
            resolve();
          }
        };
        video.addEventListener('timeupdate', playing);
        playing();
      });
      video.click();
    }
  }).catch(showError);
})();
