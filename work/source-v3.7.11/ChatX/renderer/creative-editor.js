/* Local canvas editing. Image contents never leave this PC for crop/drawing. */
(()=>{'use strict';
 const by=id=>document.getElementById(id),N=(v,a,b)=>Math.max(a,Math.min(b,Number(v)||a));
 const load=async blob=>{const u=URL.createObjectURL(blob);try{const im=new Image();im.src=u;await im.decode();return im;}finally{URL.revokeObjectURL(u);}};
 const error=(kind,e)=>{showToast(e.message||String(e));reportBug(kind,e,'warning',{});};
 const photo=document.createElement('div');photo.id='photoEditModal';photo.className='modal hidden';photo.innerHTML='<div class="modal-card photo-edit-card"><div class="modal-head"><b id="photoEditTitle">Foto profil</b><button id="photoEditClose" aria-label="Tutup foto">×</button></div><div id="photoView"><img id="photoFull" alt="Foto profil"></div><div id="photoCrop"><canvas id="photoCropCanvas" width="512" height="512"></canvas><label>Perbesar <input id="photoZoom" type="range" min="1" max="5" step="0.01" value="1"></label><p>Geser foto untuk mengatur posisi. Preview profil ditampilkan bulat.</p><button id="photoCropApply" class="primary">Gunakan foto</button></div></div>';document.body.append(photo);
 const cv=by('photoCropCanvas'),cx=cv.getContext('2d');let image=null,zoom=1,offset={x:0,y:0},drag=null,resolveCrop=null;
 function closePhoto(){photo.classList.add('hidden');by('photoFull').removeAttribute('src');image=null;const fn=resolveCrop;resolveCrop=null;fn?.(null);}
 function renderCrop(){if(!image)return;const scale=Math.max(512/image.width,512/image.height)*zoom,w=image.width*scale,h=image.height*scale;offset.x=N(offset.x,-(w-512)/2,(w-512)/2);offset.y=N(offset.y,-(h-512)/2,(h-512)/2);cx.clearRect(0,0,512,512);cx.drawImage(image,(512-w)/2+offset.x,(512-h)/2+offset.y,w,h);}
 by('photoEditClose').onclick=closePhoto;by('photoZoom').oninput=e=>{zoom=Number(e.target.value);renderCrop();};
 cv.onpointerdown=e=>{drag={x:e.clientX,y:e.clientY,ox:offset.x,oy:offset.y};cv.setPointerCapture(e.pointerId);};cv.onpointermove=e=>{if(!drag)return;const k=512/cv.getBoundingClientRect().width;offset={x:drag.ox+(e.clientX-drag.x)*k,y:drag.oy+(e.clientY-drag.y)*k};renderCrop();};cv.onpointerup=cv.onpointercancel=()=>drag=null;
 by('photoCropApply').onclick=()=>{const fn=resolveCrop;resolveCrop=null;cv.toBlob(blob=>{closePhoto();fn?.(blob);},'image/png');};
 window.KCPhotoEditor={crop:async file=>{closePhoto();image=await load(file);zoom=1;offset={x:0,y:0};by('photoZoom').value=1;by('photoEditTitle').textContent='Atur foto profil';by('photoView').hidden=true;by('photoCrop').hidden=false;photo.classList.remove('hidden');renderCrop();return new Promise(r=>resolveCrop=r);},view:(url,name)=>{closePhoto();by('photoEditTitle').textContent=name||'Foto profil';by('photoView').hidden=false;by('photoCrop').hidden=true;by('photoFull').src=fullUrl(url);photo.classList.remove('hidden');}};
 document.addEventListener('click',e=>{const el=e.target?.closest?.('[data-profile-photo]');if(!el?.dataset.profilePhoto||['railProfile','myAvatarButton'].includes(el.id)||el.closest('#statusList')||el.closest('#photoEditModal'))return;e.preventDefault();e.stopImmediatePropagation();window.KCPhotoEditor.view(el.dataset.profilePhoto,el.dataset.profileName);},true);
 document.addEventListener('keydown',e=>{if(e.key==='Escape'&&!photo.classList.contains('hidden')){e.preventDefault();e.stopImmediatePropagation();closePhoto();}},true);

 const sc=by('stickerCanvas'),ctx=sc.getContext('2d'),base=document.createElement('canvas'),ink=document.createElement('canvas');base.width=ink.width=512;base.height=ink.height=512;const bx=base.getContext('2d'),ix=ink.getContext('2d');
 let texts=[],selected=-1,tool='move',gesture=null,history=[],redo=[],busy=false,epoch=0;
 const tools=by('stickerEditorModal').querySelector('.sticker-editor-tools');tools.innerHTML=`
   <section class="sticker-tool-group">
     <h3>Gambar</h3>
     <button type="button" id="chooseStickerImage">Pilih gambar</button>
     <div class="sticker-tool-grid">
       <button id="stMove" data-tool="move">Pindah</button><button id="stCrop" data-tool="crop">Potong</button>
       <button id="stApplyCrop" hidden>Terapkan crop</button><button id="stRemoveBg">Hapus latar AI</button>
       <button id="stUndo">Undo</button><button id="stRedo">Redo</button>
     </div>
   </section>
   <section class="sticker-tool-group">
     <h3>Coretan</h3>
     <div class="sticker-tool-grid">
       <button id="stBrush" data-tool="brush">Kuas</button><button id="stErase" data-tool="erase">Penghapus</button>
     </div>
     <div class="sticker-tool-grid">
       <label>Ukuran <output id="stBrushValue">12 px</output><input id="stBrushSize" type="range" min="1" max="100" value="12"></label>
       <label>Warna<input id="stBrushColor" type="color" value="#ff2f55"></label>
     </div>
     <button id="clearStickerDraw">Hapus coretan</button>
   </section>
   <section class="sticker-tool-group">
     <h3>Teks</h3>
     <label>Tulisan<input id="stickerTextInput" maxlength="200" placeholder="Tulis sesuatu"></label>
     <label>Font<select id="stFont"><option>Segoe UI</option><option>Arial</option><option>Impact</option><option>Georgia</option><option>Comic Sans MS</option><option>Courier New</option></select></label>
     <div class="sticker-tool-grid">
       <label>Ukuran<input id="stFontSize" type="number" min="12" max="200" value="48"></label>
       <label>Warna<input id="stTextColor" type="color" value="#111111"></label>
     </div>
     <div class="sticker-tool-grid"><button id="addStickerText">Tambah teks</button><button id="stDeleteText">Hapus layer</button></div>
   </section>
   <section class="sticker-tool-group">
     <h3>Emoji</h3>
     <div id="stEmojiTabs" class="st-emoji" role="group" aria-label="Kategori emoji"></div>
     <div id="stEmoji" class="st-emoji sticker-emoji-grid"></div>
   </section>
   <p id="stEditorStatus" role="status"></p>`;
 const categories = Object.entries(EMOJI_GROUPS);
 const categoryNames = ['Wajah','Gestur','Hati','Hewan','Makanan','Aktivitas','Kendaraan','Benda','Simbol'];
 function showEmojiGroup(index) {
   by('stEmoji').replaceChildren(...categories[index][1].map(emoji => {
     const button = document.createElement('button');
     button.type = 'button';
     button.textContent = emoji;
     button.onclick = () => addText(emoji, true);
     return button;
   }));
   [...by('stEmojiTabs').children].forEach((button, i) => button.setAttribute('aria-pressed', String(i === index)));
 }
 categories.forEach(([emoji], index) => {
   const button = document.createElement('button');
   button.type = 'button';
   button.textContent = emoji;
   button.title = categoryNames[index];
   button.setAttribute('aria-label', categoryNames[index]);
   button.onclick = () => showEmojiGroup(index);
   by('stEmojiTabs').append(button);
 });
 showEmojiGroup(0);
 function status(t){by('stEditorStatus').textContent=t;}
 function point(e){const r=sc.getBoundingClientRect();return{x:N((e.clientX-r.left)*512/r.width,0,512),y:N((e.clientY-r.top)*512/r.height,0,512)};}
 function font(t){return `${t.emoji?'':'900 '}${t.size}px "${t.font}", "Segoe UI Emoji", sans-serif`;}
 function drawText(x,t){x.save();x.font=font(t);x.textAlign='center';x.textBaseline='middle';if(!t.emoji){x.lineWidth=Math.max(2,t.size/8);x.strokeStyle='#ffffff';x.strokeText(t.text,t.x,t.y);}x.fillStyle=t.color;x.fillText(t.text,t.x,t.y);x.restore();}
 function composite(x){x.clearRect(0,0,512,512);x.drawImage(base,0,0);x.drawImage(ink,0,0);for(const t of texts)drawText(x,t);}
 let cropBox=null;
 function render(){composite(ctx);if(selected>=0&&texts[selected]&&tool==='move'){const t=texts[selected];ctx.save();ctx.font=font(t);const w=ctx.measureText(t.text).width;ctx.strokeStyle='#53c69a';ctx.setLineDash([6,4]);ctx.strokeRect(t.x-w/2-6,t.y-t.size*.65,w+12,t.size*1.3);ctx.restore();}if(cropBox){ctx.save();ctx.strokeStyle='#53c69a';ctx.lineWidth=2;ctx.setLineDash([8,4]);ctx.strokeRect(cropBox.x,cropBox.y,cropBox.w,cropBox.h);ctx.restore();}}
 function snapshot(){return{base:bx.getImageData(0,0,512,512),ink:ix.getImageData(0,0,512,512),texts:texts.map(t=>({...t}))};}
 function remember(){history.push(snapshot());if(history.length>16)history.shift();redo=[];by('stUndo').disabled=false;by('stRedo').disabled=true;}
 function setTool(t){tool=t;cropBox=null;by('stApplyCrop').hidden=true;for(const b of tools.querySelectorAll('[data-tool]'))b.classList.toggle('active',b.dataset.tool===t);sc.style.cursor=t==='move'?'grab':'crosshair';render();}
 function syncText(){const t=texts[selected];if(!t)return;by('stickerTextInput').value=t.text;by('stFont').value=t.font;by('stFontSize').value=t.size;by('stTextColor').value=t.color;}
 function reset(){by('saveSticker').disabled=by('sendSticker').disabled=false;epoch++;bx.clearRect(0,0,512,512);ix.clearRect(0,0,512,512);texts=[];selected=-1;history=[];redo=[];by('stRedo').disabled=true;gesture=null;busy=false;by('stRemoveBg').disabled=false;by('stUndo').disabled=true;setTool('move');status('Paste atau pilih gambar. Teks dan emoji bisa digeser.');}
 async function imageFile(file){if(busy)return;if(!file?.type.startsWith('image/'))return showToast('Pilih gambar.');const id=++epoch;try{const im=await load(file);if(id!==epoch)return;remember();bx.clearRect(0,0,512,512);ix.clearRect(0,0,512,512);const k=Math.min(512/im.width,512/im.height);bx.drawImage(im,(512-im.width*k)/2,(512-im.height*k)/2,im.width*k,im.height*k);texts=[];selected=-1;setTool('move');status('Gambar siap. Crop area atau hapus background bila diperlukan.');}catch(e){error('sticker-image-failed',e);}}
 function addText(value,emoji=false){if(busy)return;const text=String(value||'').trim().slice(0,200);if(!text)return;remember();texts.push({text,x:256,y:256,size:N(by('stFontSize').value,12,200),font:emoji?'Segoe UI Emoji':by('stFont').value,color:by('stTextColor').value,emoji});selected=texts.length-1;setTool('move');syncText();}
 by('newSticker').onclick=()=>{by('stickerModal').classList.add('hidden');by('stickerEditorModal').classList.remove('hidden');reset();sc.focus();};by('chooseStickerImage').onclick=()=>by('stickerImageInput').click();by('stickerImageInput').onchange=e=>{const f=e.target.files?.[0];e.target.value='';if(f)imageFile(f);};by('closeStickerEditor').onclick=()=>{epoch++;by('stickerEditorModal').classList.add('hidden');};
 document.addEventListener('paste',e=>{if(by('stickerEditorModal').classList.contains('hidden'))return;const f=[...(e.clipboardData?.items||[])].find(i=>i.kind==='file'&&i.type.startsWith('image/'))?.getAsFile();if(f){e.preventDefault();e.stopImmediatePropagation();imageFile(f);}},true);
 for(const b of tools.querySelectorAll('[data-tool]'))b.onclick=()=>{if(!busy)setTool(b.dataset.tool);};
 by('stBrushSize').oninput=e=>by('stBrushValue').textContent=e.target.value+' px';by('addStickerText').onclick=()=>addText(by('stickerTextInput').value);
 for(const id of ['stickerTextInput','stFont','stFontSize','stTextColor'])by(id).addEventListener('change',()=>{const t=texts[selected];if(!t||busy)return;remember();t.text=by('stickerTextInput').value.slice(0,200);t.size=N(by('stFontSize').value,12,200);t.font=by('stFont').value;t.color=by('stTextColor').value;render();});
 by('stDeleteText').onclick=()=>{if(selected<0||busy)return;remember();texts.splice(selected,1);selected=-1;render();};
 by('clearStickerDraw').onclick=()=>{if(busy)return;remember();ix.clearRect(0,0,512,512);render();};function restore(from, to) {
   if (busy || !from.length) return;
   epoch++;
   to.push(snapshot());
   const state = from.pop();
   bx.putImageData(state.base,0,0);
   ix.putImageData(state.ink,0,0);
   texts=state.texts;
   selected=-1;
   by('stUndo').disabled=!history.length;
   by('stRedo').disabled=!redo.length;
   setTool('move');
 }
 by('stUndo').onclick=()=>restore(history,redo);
 by('stRedo').onclick=()=>restore(redo,history);
 sc.addEventListener('keydown', event => {
   if ((event.ctrlKey || event.metaKey) && ['z','y'].includes(event.key.toLowerCase())) {
     event.preventDefault();
     if (event.shiftKey || event.key.toLowerCase()==='y') restore(redo,history);
     else restore(history,redo);
   } else if (event.key==='Delete' && selected>=0) {
     event.preventDefault();
     by('stDeleteText').click();
   }
 });
 sc.addEventListener('dragover', event => event.preventDefault());
 sc.addEventListener('drop', event => {
   event.preventDefault();
   const file = event.dataTransfer.files[0];
   if (file) void imageFile(file);
 });
 function stroke(a,b){ix.save();ix.globalCompositeOperation=tool==='erase'?'destination-out':'source-over';ix.strokeStyle=by('stBrushColor').value;ix.lineWidth=N(by('stBrushSize').value,1,100);ix.lineCap='round';ix.lineJoin='round';ix.beginPath();if(Math.hypot(b.x-a.x,b.y-a.y)<.1){ix.fillStyle=by('stBrushColor').value;ix.arc(a.x,a.y,ix.lineWidth/2,0,Math.PI*2);ix.fill();}else{ix.moveTo(a.x,a.y);ix.lineTo(b.x,b.y);ix.stroke();}ix.restore();}
 sc.onpointerdown=e=>{if(busy||e.button!==0)return;e.preventDefault();const p=point(e);if(tool==='move'){selected=-1;for(let i=texts.length-1;i>=0;i--){const t=texts[i];ctx.font=font(t);if(Math.abs(p.x-t.x)<=ctx.measureText(t.text).width/2+12&&Math.abs(p.y-t.y)<=t.size*.7){selected=i;break;}}if(selected>=0){remember();gesture={p,x:texts[selected].x,y:texts[selected].y};syncText();}}else if(tool==='crop'){gesture={p};cropBox=null;}else{remember();gesture={p};stroke(p,p);}sc.focus({preventScroll:true});sc.setPointerCapture(e.pointerId);render();};
 sc.onpointermove=e=>{if(!gesture||busy)return;const p=point(e);if(tool==='move'&&selected>=0){texts[selected].x=N(gesture.x+p.x-gesture.p.x,0,512);texts[selected].y=N(gesture.y+p.y-gesture.p.y,0,512);}else if(tool==='crop'){const a=gesture.p;cropBox={x:Math.min(a.x,p.x),y:Math.min(a.y,p.y),w:Math.abs(p.x-a.x),h:Math.abs(p.y-a.y)};}else{stroke(gesture.p,p);gesture.p=p;}render();};sc.onpointerup=sc.onpointercancel=()=>{gesture=null;by('stApplyCrop').hidden=!(cropBox?.w>4&&cropBox?.h>4);};
 by('stApplyCrop').onclick=()=>{if(!cropBox||busy)return;remember();const c={...cropBox},tmp=document.createElement('canvas');tmp.width=tmp.height=512;composite(tmp.getContext('2d'));bx.clearRect(0,0,512,512);ix.clearRect(0,0,512,512);const k=Math.min(512/c.w,512/c.h);bx.drawImage(tmp,c.x,c.y,c.w,c.h,(512-c.w*k)/2,(512-c.h*k)/2,c.w*k,c.h*k);texts=[];selected=-1;setTool('move');status('Crop diterapkan. Undo untuk mengembalikan.');};
 by('stRemoveBg').onclick=async()=>{if(busy)return;busy=true;const id=++epoch;by('stRemoveBg').disabled=true;by('saveSticker').disabled=by('sendSticker').disabled=true;status('AI sedang menghapus background gambar…');try{const result=await window.kc.aiRemoveBackground({base64:base.toDataURL('image/png').split(',')[1],mime:'image/png',name:'sticker.png',model:'medium',computeMode:'auto'});if(!result?.ok)throw Error(result?.error||'Hapus background gagal');const blob=await(await fetch('data:image/png;base64,'+result.base64)).blob(),im=await load(blob);if(id!==epoch)return;remember();bx.clearRect(0,0,512,512);bx.drawImage(im,0,0,512,512);render();status('Background transparan. Teks dan coretan tetap utuh.');}catch(e){if(id===epoch){status('Gagal menghapus background: '+(e.message||e));error('sticker-remove-background-failed',e);}}finally{if(id===epoch){busy=false;by('stRemoveBg').disabled=false;by('saveSticker').disabled=by('sendSticker').disabled=false;}}};
 function exportCanvas(){const out=document.createElement('canvas');out.width=out.height=512;composite(out.getContext('2d'));return out;}
 window.KCStickerEditor={
  exportPng:()=>exportCanvas().toDataURL('image/png'),
  exportBlob:()=>new Promise((resolve,reject)=>exportCanvas().toBlob(blob=>blob?resolve(blob):reject(Error('PNG stiker gagal dibuat.')),'image/png')),
  reset
 };reset();
})();
