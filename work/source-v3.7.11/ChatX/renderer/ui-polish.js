/* Normalize functional icons without replacing controls, listeners or data attributes. */
(() => {
 const paths={
 undo:'<path d="M9 14 4 9l5-5"/><path d="M4 9h9a7 7 0 1 1 0 14h-2"/>',
 redo:'<path d="m15 14 5-5-5-5"/><path d="M20 9h-9a7 7 0 1 0 0 14h2"/>',
 minus:'<path d="M5 12h14"/>',
 add:'<path d="M12 5v14M5 12h14"/>',
 reset:'<path d="M20 11a8 8 0 1 0 1 4"/><path d="M20 4v7h-7"/>',
 search:'<circle cx="10.8" cy="10.8" r="6.8"/><path d="m16 16 4.5 4.5"/>',
 check:'<path d="m5 12 4.5 4.5L19 7"/>',
 photo:'<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="8.5" cy="9" r="1.5"/><path d="m4 17 5-5 3.5 3 2.5-2 5 5"/>',
 text:'<path d="M4 6V4h16v2M12 4v16M8 20h8"/>',
 clipboard:'<rect x="5" y="5" width="14" height="16" rx="2"/><path d="M9 5.5V4h6v1.5M8 11h8M8 15h8"/>',
 calendar:'<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M7 3v4m10-4v4M3 10h18"/>',
 left:'<path d="m15 18-6-6 6-6"/>',
 right:'<path d="m9 18 6-6-6-6"/>',
 image:'<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8" cy="8" r="1.5"/><path d="m4 18 5-5 4 3 3-5 5 7"/>',
 more:'<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
 eye:'<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/>',
 print:'<path d="M6 8V3h12v5M6 17H3V8h18v9h-3M6 14h12v7H6Z M17 11h1"/>',
 game:'<path d="M7 7h10l4 11-3 2-4-4h-4l-4 4-3-2Zm-1 5h5m-2-2v4m6-2h.01m3 2h.01"/>',
 music:'<path d="M9 18V5l11-2v13M9 8l11-2"/><ellipse cx="6" cy="18" rx="3" ry="3"/><ellipse cx="17" cy="16" rx="3" ry="3"/>',
 shield:'<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6Z M8 12l3 3 5-6"/>',
 activity:'<circle cx="12" cy="12" r="9"/><path d="M3 12h5l2-5 4 10 2-5h5"/>',
 copy:'<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V4H4v12h4"/>',
 download:'<path d="M12 3v12m-4-4 4 4 4-4M4 17v4h16v-4"/>',
 select:'<rect x="3" y="3" width="18" height="18" rx="3"/><path d="m7 12 3 3 7-7"/>',
 reply:'<path d="m9 5-6 6 6 6M3 11h11a7 7 0 0 1 7 7"/>',
 forward:'<path d="m15 5 6 6-6 6M21 11H10a7 7 0 0 0-7 7"/>',
 info:'<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v1"/>',
 edit:'<path d="m4 16-1 5 5-1L21 7l-4-4Z M14 6l4 4"/>',
 trash:'<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7"/>',
 star:'<path d="m12 3 2.8 5.8 6.4.9-4.6 4.5 1.1 6.3-5.7-3-5.7 3 1.1-6.3-4.6-4.5 6.4-.9Z"/>',
 close:'<path d="m6 6 12 12M18 6 6 18"/>',
 plus:'<path d="M12 4v16M4 12h16"/>',
 attachment:'<path d="m8.5 13.5 6.1-6.1a2.1 2.1 0 0 1 3 3l-7.5 7.5a3.8 3.8 0 1 1-5.4-5.4l8.2-8.2a5 5 0 1 1 7.1 7.1l-8.2 8.2"/>',
 send:'<path d="m4 4 16 8-16 8 3-8Zm3 8h7"/>',
 emoji:'<circle cx="12" cy="12" r="9"/><circle cx="9" cy="10" r=".9" fill="currentColor" stroke="none"/><circle cx="15" cy="10" r=".9" fill="currentColor" stroke="none"/><path d="M8.5 14.2a4.2 4.2 0 0 0 7 0"/>',
 sticker:'<path d="M12.5 21H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3h12a3 3 0 0 1 3 3v6.5a3 3 0 0 1-.9 2.1l-5.5 5.5a3 3 0 0 1-2.1.9Z"/><path d="M13 21v-5a3 3 0 0 1 3-3h5"/><path d="M7.5 8.5h4"/>',
 pin:'<path d="M9 3h6l-1 6 4 4H6l4-4ZM12 13v8"/>',
 bell:'<path d="M6 16h12l-2-3V9a4 4 0 0 0-8 0v4Zm4 4h4"/>',
 archive:'<path d="M3 3h18v5H3Zm2 5v13h14V8M9 12h6"/>',
 leave:'<path d="M10 3H3v18h7m-2-9h13m-4-4 4 4-4 4"/>'
 };
 function icon(key){return `<svg class="kc-control-icon" viewBox="0 0 24 24" aria-hidden="true">${paths[key]||paths.info}</svg>`;}
 const actions={'copy-image':'copy','copy-text':'copy','download-image':'download',select:'select',reply:'reply',forward:'forward',details:'info',edit:'edit',delete:'trash','save-sticker':'star',pin:'pin',favorite:'star',mute:'bell',archive:'archive','delete-local':'trash',leave:'leave','delete-everyone':'trash'};
 document.querySelectorAll('#messageContextMenu button,#conversationMenu button').forEach(button=>{
  const label=button.textContent.trim().replace(/^[^\p{L}\p{N}]+/u,'');
  button.innerHTML=icon(actions[button.dataset.msgAction||button.dataset.action])+`<span class="kc-control-label"></span>`;
  button.querySelector('span').textContent=label;
 });
 const controls={attach:['attachment','Lampiran'],emojiButton:['emoji','Emoji'],stickerButton:['sticker','Stiker'],send:['send','Kirim pesan']};
 Object.entries(controls).forEach(([id,[key,label]])=>{const button=document.getElementById(id);if(button){button.innerHTML=icon(key);button.setAttribute('aria-label',label);button.title=label;}});
 const railTooltip=document.createElement('div');
 railTooltip.className='chatx-rail-tooltip';
 railTooltip.hidden=true;
 railTooltip.setAttribute('aria-hidden','true');
 document.body.append(railTooltip);
 let tooltipTimer;
 const hideRailTooltip=()=>{clearTimeout(tooltipTimer);railTooltip.hidden=true;};
 document.querySelectorAll('.rail-btn[title]').forEach(button=>{
  const label=button.title;
  if(!button.getAttribute('aria-label'))button.setAttribute('aria-label',label);
  button.removeAttribute('title');
  const show=()=>{
   hideRailTooltip();
   tooltipTimer=setTimeout(()=>{
    const bounds=button.getBoundingClientRect();
    railTooltip.textContent=label;
    railTooltip.style.left=(bounds.right+8)+'px';
    railTooltip.style.top=Math.max(20,Math.min(innerHeight-20,bounds.top+bounds.height/2))+'px';
    railTooltip.hidden=false;
   },350);
  };
  button.addEventListener('pointerenter',show);
  button.addEventListener('pointerleave',hideRailTooltip);
  button.addEventListener('focus',()=>{if(button.matches(':focus-visible'))show();});
  button.addEventListener('blur',hideRailTooltip);
  button.addEventListener('click',hideRailTooltip);
 });
 window.addEventListener('blur',hideRailTooltip);
 document.addEventListener('scroll',hideRailTooltip,true);
 ['chatImagePrev','chatImageNext'].forEach((id,index)=>{const b=document.getElementById(id);if(b)b.innerHTML=`<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${index?'m9 5 7 7-7 7':'m15 5-7 7 7 7'}"/></svg>`;});
 const headingIcons={clipboardModal:'copy',activityModal:'activity',fileGalleryModal:'archive',spotifyModal:'music',systemModal:'activity',removeBgModal:'edit',gameInviteModal:'game',psdLibraryModal:'archive',vectorModal:'edit',headCropModal:'edit',upscaleModal:'plus'};
 Object.entries(headingIcons).forEach(([id,key])=>{const heading=document.querySelector('#'+id+' .modal-head b');if(!heading)return;const label=heading.textContent.trim().replace(/^[^\p{L}\p{N}]+/u,'');heading.innerHTML=icon(key);heading.appendChild(document.createTextNode(label));});
 document.querySelectorAll('.modal-head>button').forEach(button=>{if(button.textContent.trim()==='\u00d7'){button.innerHTML=icon('close');if(!button.getAttribute('aria-label'))button.setAttribute('aria-label','Tutup');}});
 function polishDynamic(root){
  const targets=[['#editGroupIdentity,#editOfficeGroup','edit',true],['#changeGroupAvatar,#changeOfficeGroupAvatar','image',true],['.order-more','more',false],['.order-view','eye',false],['.order-example','print',true]];
  for(const [selector,key,labelled]of targets){
   const buttons=root.matches?.(selector)?[root,...root.querySelectorAll(selector)]:root.querySelectorAll(selector);
   buttons.forEach(button=>{if(button.dataset.iconPolished)return;button.dataset.iconPolished='1';const label=button.textContent.trim().replace(/^[^\p{L}\p{N}]+/u,'');button.innerHTML=icon(key);if(labelled){const span=document.createElement('span');span.textContent=label;button.appendChild(span);}else button.setAttribute('aria-label',button.title||label);});
  }
 }
 const dynamicObserver = new MutationObserver(records => {
  const added = new Set();
  for (const record of records) {
   for (const node of record.addedNodes) if (node instanceof Element && node.isConnected) added.add(node);
  }
  for (const node of added) {
   if (!added.has(node.parentElement)) polishDynamic(node);
  }
 });
 for(const id of ['chatInfoBody','dailyBoard']){
  const root=document.getElementById(id);
  if(!root)continue;
  polishDynamic(root);
  dynamicObserver.observe(root,{childList:true,subtree:true});
 }
})();
