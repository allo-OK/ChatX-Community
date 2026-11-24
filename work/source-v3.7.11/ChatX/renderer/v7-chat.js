(function(){
 document.addEventListener('keydown',event=>{
  if(event.isComposing)return;
  if(!(event.ctrlKey||event.metaKey)||event.altKey||event.key.toLowerCase()!=='c')return;
  const field=document.activeElement;let selected='';
  if(field?.matches('textarea,input:not([type=password])'))selected=field.value.slice(field.selectionStart||0,field.selectionEnd||0);
  else selected=window.getSelection()?.toString()||'';
  if(!selected)selected=window.getSelection()?.toString()||'';
  if(!selected)return;event.preventDefault();event.stopImmediatePropagation();
  Promise.resolve((window.kc.copyChatText?.(selected) ?? navigator.clipboard.writeText(selected))).catch(()=>window.showToast?.('Teks belum berhasil disalin. Coba lagi.'));
 },true);
 function place(){const button=document.getElementById('scrollToBottom'),composer=document.querySelector('.composer-wrap');if(!button||!composer)return;const parent=button.offsetParent||button.parentElement,bounds=parent.getBoundingClientRect(),rect=composer.getBoundingClientRect();button.style.bottom=Math.max(14,bounds.bottom-rect.top+14)+'px';}
 const composer=document.querySelector('.composer-wrap');if(composer){if(window.ResizeObserver)new ResizeObserver(place).observe(composer);new MutationObserver(place).observe(composer,{childList:true,subtree:true,attributes:true,attributeFilter:['class']});}window.addEventListener('resize',place);place();
})();
