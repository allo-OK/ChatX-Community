const invoke = window.__TAURI__.core.invoke;
const statusLabel = document.getElementById('status');
const details = document.getElementById('details');
let readyReported = false;

async function refresh() {
  try {
    const status = await invoke('host_status');
    statusLabel.dataset.state = status.state;
    details.hidden = status.state !== 'running';
    if (status.state === 'running') {
      statusLabel.textContent = 'Server aktif';
      document.getElementById('address').textContent = status.address;
      document.getElementById('data-path').textContent = status.dataPath;
      if (!readyReported) {
        readyReported = true;
        await invoke('host_ui_ready');
      }
    } else if (status.state === 'failed') {
      statusLabel.textContent = 'Server gagal dibuka · ' + status.error;
    }
  } catch (error) {
    statusLabel.dataset.state = 'failed';
    statusLabel.textContent = 'Gagal membaca status HOST · ' + String(error);
    details.hidden = true;
  }
  window.setTimeout(refresh, 1000);
}

void refresh();
