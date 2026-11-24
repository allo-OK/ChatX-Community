document.addEventListener('DOMContentLoaded', async () => {
  const invoke = (...args) => window.__TAURI__.core.invoke(...args);
  const square = index => document.querySelector('#b [data-index="' + index + '"]');
  async function until(predicate, message) {
    for (let attempt = 0; attempt < 80; attempt++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    throw Error(message);
  }
  try {
    if (await invoke('game_window_action', {kind:'smoke-client-mode',payload:null})) {
      const bot = document.getElementById('botBtn');
      await until(() => !bot.hidden && !bot.classList.contains('hidden') && !bot.disabled && document.getElementById('title').textContent.includes('Catur'),
        'Tombol bot catur CLIENT tidak tersedia.');
      bot.click();
    }
    await until(() => document.querySelectorAll('#b .sq').length === 64 && square(52)?.disabled === false,
      'Papan catur tidak siap menerima langkah pemain putih.');
    square(52).click();
    if (!square(36)?.classList.contains('legal')) throw Error('Tujuan e4 tidak ditandai legal.');
    square(36).click();
    await until(() => square(36)?.querySelector('.whitepiece') && !square(52)?.querySelector('.piece'),
      'Langkah e2–e4 tidak kembali dari server ke papan catur.');
    await invoke('game_window_action', {kind:'smoke-complete',payload:{ok:true}});
    document.getElementById('closeWindowBtn').click();
  } catch (error) {
    await invoke('game_window_action', {kind:'smoke-complete',payload:{error:error.message}});
  }
}, {once:true});
