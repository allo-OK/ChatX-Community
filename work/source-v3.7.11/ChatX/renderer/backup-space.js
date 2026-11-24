(()=>{const modal=document.getElementById('storageModal');if(!modal)return;const box=document.createElement('section');box.className='backup-space-panel';box.hidden=true;box.innerHTML='<h3>Hemat ruang backup</h3><p>Periksa file identik di backup update dan checkpoint. Optimasi berbagi penyimpanan hanya antar salinan backup yang cocok checksum. Chat, lampiran aktif, dan jumlah backup tetap dipertahankan.</p><button id="backupSpaceAnalyze">Analisis ruang</button><button id="backupSpaceOptimize" hidden>Optimalkan salinan identik</button><p id="backupSpaceResult" role="status">Klik Analisis ruang untuk menghitung potensi penghematan. Proses membaca backup besar dapat memerlukan beberapa menit.</p>';modal.querySelector('.modal-card').append(box);let last=null,busy=false;const $=id=>document.getElementById(id),size=n=>(n/1073741824).toLocaleString('id-ID',{maximumFractionDigits:2})+' GB';
 async function run(dryRun){if(busy)return;busy=true;$('backupSpaceAnalyze').disabled=$('backupSpaceOptimize').disabled=true;$('backupSpaceResult').textContent=dryRun?'Memeriksa checksum salinan backup…':'Mengoptimalkan salinan identik…';try{const r=await window.kc.backupSpace({dryRun,confirmed:!dryRun});if(!r?.ok)throw Error(r?.error||'Pemeriksaan backup gagal.');last=r;$('backupSpaceResult').textContent=(dryRun?'Potensi penghematan: ':'Penyimpanan yang dihemat: ')+size(r.savedBytes)+' · '+r.files+' file identik.'+(r.warnings?.length?' '+r.warnings.join(' '):'')+(!dryRun?' Backup tetap tersedia. Ukuran logis folder dapat tetap terlihat sama karena file berbagi blok disk.':'');$('backupSpaceOptimize').hidden=!dryRun||!r.files;}catch(e){$('backupSpaceResult').textContent=e.message;window.kc.reportBug?.({type:'backup-compaction-failed',message:e.message,source:'storage',severity:'warning'});}finally{busy=false;$('backupSpaceAnalyze').disabled=$('backupSpaceOptimize').disabled=false;}}
 $('backupSpaceAnalyze').onclick=()=>run(true);$('backupSpaceOptimize').onclick=async()=>{if(!last||busy)return;if(await appConfirm({title:'Hemat '+size(last.savedBytes)+' dari salinan backup?',text:'File identik antar backup akan berbagi blok penyimpanan. Backup tetap bisa dipulihkan. Jangan mengedit isi folder backup secara manual. File aktif tidak diubah.',ok:'Optimalkan backup'}))run(false);};
 new MutationObserver(()=>{box.hidden=typeof cfg==='undefined'||!cfg?.isHost;}).observe(modal,{attributes:true,attributeFilter:['class']});
})();
(() => {
  const modal = document.getElementById('storageModal');
  if (!modal || typeof window.kc?.hostStorageInventory !== 'function') return;

  const panel = document.createElement('section');
  panel.className = 'backup-space-panel';
  panel.innerHTML = `<h3>Pemindai penyimpanan HOST</h3>
    <p>Lihat file terbesar dan ukuran backup ChatX. Untuk menghapus lampiran chat, gunakan File Storage.</p>
    <button type="button" class="secondary">Pindai penyimpanan</button>
    <p role="status"></p>
    <div class="largest-list"></div>`;
  modal.querySelector('.storage-actions').after(panel);
  const scan = panel.querySelector('button');
  const status = panel.querySelector('[role="status"]');
  const results = panel.querySelector('.largest-list');
  const formatBytes = bytes => bytes >= 1073741824
    ? (bytes / 1073741824).toLocaleString('id-ID', { maximumFractionDigits: 2 }) + ' GB'
    : (bytes / 1048576).toLocaleString('id-ID', { maximumFractionDigits: 2 }) + ' MB';

  function renderGroup(title, rows, backup) {
    const group = document.createElement('details');
    group.open = backup;
    const heading = document.createElement('summary');
    heading.textContent = title + ' (' + rows.length + ')';
    group.append(heading);
    for (const entry of rows) {
      const row = document.createElement('div');
      const label = document.createElement('p');
      label.style.overflowWrap = 'anywhere';
      label.textContent = formatBytes(entry.bytes) + ' · ' + entry.path;
      row.append(label);
      if (backup && entry.removable) {
        const remove = document.createElement('button');
        remove.type = 'button';
        remove.className = 'danger';
        remove.textContent = 'Hapus backup';
        remove.onclick = async () => {
          if (!await appConfirm({title:'Hapus backup lama?', text:entry.path, ok:'Hapus backup'})) return;
          await run(entry.path);
        };
        row.append(remove);
      } else if (backup) {
        const protectedLabel = document.createElement('small');
        protectedLabel.textContent = 'Dilindungi untuk pemulihan';
        row.append(protectedLabel);
      }
      group.append(row);
    }
    return group;
  }

  async function run(deleteBackup) {
    if (scan.disabled) return;
    scan.disabled = true;
    results.inert = true;
    status.textContent = deleteBackup ? 'Menghapus backup lama…' : 'Memindai penyimpanan…';
    try {
      const report = await window.kc.hostStorageInventory(deleteBackup ?? null);
      results.replaceChildren(
        renderGroup('Backup', report.backups, true),
        renderGroup('File terbesar', report.files, false)
      );
      status.textContent = (deleteBackup ? 'Backup lama dihapus. ' : '') +
        'Ukuran yang ditampilkan adalah ukuran logis file. Backup identik dapat berbagi ruang disk.' +
        (report.skipped ? ' Lokasi atau file yang dilewati: ' + report.skipped + '.' : '');
    } catch (error) {
      status.textContent = 'Operasi penyimpanan gagal. ' + String(error);
      window.kc.reportBug?.({type:'storage-inventory-failed',message:String(error),source:'storage',severity:'warning'});
    } finally {
      results.inert = false;
      scan.disabled = false;
    }
  }

  scan.onclick = () => run();
})();
