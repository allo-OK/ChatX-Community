async function openDevices() {
  const request = async (path, body) => {
    const options = body ? {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    } : { cache: 'no-store' };
    const response = await hostAdminRequest(path, options);
    const result = await response.json();
    if (!response.ok || !result.ok) throw Error(result.error || 'Pengaturan perangkat gagal.');
    return result;
  };
  try {
    const { devices } = await request('/admin/devices');
    const rows = devices.map(device => {
      const id = esc(device.deviceId);
      return `<article>
        <b>${esc(device.name)}${device.pc ? ' · ' + esc(device.pc) : ''}</b>
        <p>${device.isHost ? 'HOST utama' : device.revoked ? 'Akses dicabut' : 'Akses aktif'}</p>
        ${device.isHost ? '' : `<button data-revoke="${id}">Cabut akses</button>
        <button data-remove="${id}">Aktifkan kembali</button>`}
      </article>`;
    }).join('');
    showReliabilityPanel('Akses CLIENT', `
      <button id="refreshDevices">Muat ulang</button><h4>Perangkat terdaftar</h4>${rows || '<p>Belum ada perangkat.</p>'}`);
    const panel = document.getElementById('reliabilityPanel');
    panel.querySelector('#refreshDevices').onclick = openDevices;
    for (const button of panel.querySelectorAll('[data-revoke], [data-remove]')) {
      button.onclick = async () => {
        const remove = 'remove' in button.dataset;
        if (!confirm(remove ? 'Aktifkan kembali akses perangkat ini?' : 'Cabut akses perangkat ini?')) return;
        try {
          await request('/admin/devices', { deviceId: button.dataset.remove || button.dataset.revoke, remove });
          await openDevices();
        } catch (error) {
          showToast(error.message);
        }
      };
    }
  } catch (error) {
    showToast(error.message);
  }
}
