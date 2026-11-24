/* Animate small live insertions only; history/gallery batch loads stay immediate. */
(() => {
 'use strict';
 const preference=window.matchMedia('(prefers-reduced-motion: reduce)');
 const selector='.msg-row,.order-card,.conversation-row,.file-gallery-results>*,.new-chat-list>*';
 const running=new Set();
 const seen=new WeakSet(document.querySelectorAll(selector));
 const observer=new MutationObserver(records=>{
  if(preference.matches)return;
  const nodes=new Set();
  for(const record of records)for(const node of record.addedNodes){
   if(node.nodeType!==1)continue;
   const candidates=[...(node.matches(selector)?[node]:[]),...node.querySelectorAll(selector)];
   for(const item of candidates){if(seen.has(item))continue;seen.add(item);if(item.dataset.kcMotion!=='refresh')nodes.add(item);}
  }
  if(nodes.size>6)return;
  for(const node of nodes){
   if(!node.isConnected||!node.getClientRects().length||typeof node.animate!=='function')continue;
   const animation=node.animate([{opacity:0,translate:'0 5px'},{opacity:1,translate:'0 0'}],{duration:180,easing:'cubic-bezier(.2,.8,.2,1)'});
   running.add(animation);animation.finished.catch(()=>{}).finally(()=>running.delete(animation));
  }
 });
 observer.observe(document.body,{childList:true,subtree:true});
 preference.addEventListener('change',()=>{if(preference.matches){for(const animation of running)animation.cancel();running.clear();}});
 window.addEventListener('pagehide',()=>{observer.disconnect();for(const animation of running)animation.cancel();running.clear();},{once:true});
})();
