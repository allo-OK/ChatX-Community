'use strict';

(() => {
    const {invoke} = window.__TAURI__.core;
    const dialog = document.getElementById('requiredUpdate');
    if (!dialog || (window.kc && !window.kc.checkRustClientUpdate)) return;
    if (location.pathname.endsWith('/game-window.html') && !window.__kcClientGameWindow) return;
    const detached = window.__kcClientGameWindow;
    const title = document.getElementById('requiredUpdateTitle');
    const notes = document.getElementById('requiredUpdateNotes');
    const details = document.getElementById('requiredUpdateDetails');
    const changelog = document.getElementById('requiredUpdateChangelog');
    const progress = document.getElementById('requiredUpdateProgress');
    const status = document.getElementById('requiredUpdateStatus');
    const download = document.getElementById('requiredUpdateDownload');
    const open = document.getElementById('requiredUpdateOpen');
    dialog.addEventListener('cancel',event=>event.preventDefault());
    dialog.addEventListener('keydown',event=>{
        if (event.key==='Escape') {
            event.preventDefault();
            event.stopPropagation();
        }
    });

    let version;
    function apply(policy) {
        if (!policy.locked) {
            version=undefined;
            if (dialog.open) dialog.close();
            return policy;
        }
        if (version!==policy.version) {
            progress.hidden=true;
            open.hidden=true;
            status.textContent='Unduh versi terbaru untuk melanjutkan.';
        }
        version=policy.version;
        title.textContent='Update wajib · v'+version;
        notes.textContent=policy.notes || 'HOST mewajibkan versi terbaru.';
        changelog.textContent=policy.changelog || '';
        details.hidden=!changelog.textContent;
        if (!dialog.open) dialog.showModal();
        return policy;
    }

    const restore=()=>invoke('client_required_update').then(apply);
    const refresh=()=>invoke('refresh_client_update').then(apply);
    window.kcClientUpdate=Object.freeze({apply,restore,refresh});
    const ready = detached
        ? window.__TAURI__.event.listen('client-required-update',({payload})=>apply(payload))
        : Promise.resolve();
    ready.catch(error=>{console.error('Sinkronisasi update wajib tidak tersedia',error);})
        .then(restore).catch(error=>{status.textContent=String(error);});
    window.__TAURI__.event.listen('client-update-progress',event=>{
        if (!dialog.open) return;
        progress.value=event.payload.pct;
        status.textContent=event.payload.pct===100 ? 'Installer terverifikasi dan siap dibuka.' :
            'Mengunduh update · '+event.payload.pct+'%';
    }).catch(error=>{status.textContent=String(error);});

    download.onclick=async()=>{
        download.disabled=true;
        open.hidden=true;
        progress.hidden=false;
        progress.value=0;
        status.textContent='Memeriksa update dari HOST…';
        try {
            const result=await invoke('prepare_client_update');
            status.textContent='Installer terverifikasi dan siap dibuka.';
            open.hidden=false;
            return result;
        } catch(error) {
            status.textContent=String(error);
            return null;
        } finally {
            download.disabled=false;
        }
    };
    open.onclick=async()=>{
        open.disabled=true;
        try {
            await invoke('open_client_update');
            status.textContent='Installer dibuka. Jalankan update untuk melanjutkan.';
        } catch(error) {
            status.textContent=String(error);
        } finally {
            open.disabled=false;
        }
    };
})();
