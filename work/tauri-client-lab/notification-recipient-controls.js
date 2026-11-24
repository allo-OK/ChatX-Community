if (canManage) {
  const group = document.createElement('fieldset');
  const legend = document.createElement('legend');
  legend.textContent = 'Notifikasi yang diterima ' + u.name;
  group.append(legend);
  for (const [key, text] of [
    ['receiveTagSoundEnabled', 'Bunyikan suara BLEDAR'],
    ['receiveTagVideoEnabled', 'Tampilkan video BLEDAR'],
  ]) {
    const label = document.createElement('label');
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.dataset.notificationRecipient = key;
    checkbox.dataset.member = u.name;
    checkbox.checked = u[key] !== false;
    label.append(checkbox, document.createTextNode(text));
    checkbox.onchange = async () => {
      checkbox.disabled = true;
      try {
        await req('/cohost/user', {name:u.name, [key]:checkbox.checked});
        u[key] = checkbox.checked;
        $('cohostStatus').textContent = 'Notifikasi penerima ' + u.name + ' disimpan.';
      } catch (error) {
        checkbox.checked = u[key] !== false;
        fail(error);
      } finally {
        checkbox.disabled = false;
      }
    };
    group.append(label);
  }
  card.append(group);
}
