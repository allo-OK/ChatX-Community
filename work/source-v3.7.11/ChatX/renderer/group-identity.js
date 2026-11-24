/* Electron does not support window.prompt: edit identity using a real form. */
(()=>{
 window.KCGroupIdentity={open(conversation,save){
  if(document.getElementById('groupIdentityModal'))return;
  const previous=document.activeElement,modal=document.createElement('div');modal.id='groupIdentityModal';modal.className='modal';modal.setAttribute('role','dialog');modal.setAttribute('aria-modal','true');modal.setAttribute('aria-labelledby','groupIdentityTitle');
  modal.innerHTML='<form class="modal-card group-identity-card"><div class="modal-head"><b id="groupIdentityTitle">Edit grup</b><button type="button" data-close aria-label="Tutup">×</button></div><label>Nama grup<input name="name" required maxlength="80" autocomplete="off"></label><label>Deskripsi<textarea name="description" maxlength="300" rows="3"></textarea></label><p class="group-identity-error" role="alert"></p><div class="group-identity-actions"><button type="button" data-close>Batal</button><button type="submit" class="primary">Simpan perubahan</button></div></form>';
  const form=modal.querySelector('form'),name=form.elements.name,description=form.elements.description,submit=form.querySelector('[type=submit]');name.value=conversation.name||'';description.value=conversation.description||'';let busy=false;
  const close=()=>{if(busy)return;modal.remove();previous?.focus();};modal.querySelectorAll('[data-close]').forEach(b=>b.onclick=close);
  modal.addEventListener('keydown',e=>{e.stopPropagation();if(e.key==='Escape'){e.preventDefault();close();}if(e.key==='Tab'){const controls=[...modal.querySelectorAll('button,input,textarea')].filter(x=>!x.disabled),first=controls[0],last=controls.at(-1);if(e.shiftKey&&document.activeElement===first){e.preventDefault();last.focus();}else if(!e.shiftKey&&document.activeElement===last){e.preventDefault();first.focus();}}});
  form.onsubmit=async e=>{e.preventDefault();if(busy)return;if(!name.value.trim()){name.focus();return;}busy=true;submit.disabled=true;submit.textContent='Menyimpan…';form.querySelector('[role=alert]').textContent='';try{await save({name:name.value.trim(),description:description.value.trim()});busy=false;close();}catch(error){form.querySelector('[role=alert]').textContent=error.message||'Gagal menyimpan grup';}finally{busy=false;submit.disabled=false;submit.textContent='Simpan perubahan';}};
  document.body.appendChild(modal);name.focus();name.select();
 }};
})();
