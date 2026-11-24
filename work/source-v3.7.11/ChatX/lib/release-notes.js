(function(root,factory){const api=factory();if(typeof module==='object'&&module.exports)module.exports=api;else root.KCReleaseNotes=api;})(globalThis,()=>{
 const releases={};
 function get(version, notes='', custom='') {
   const summary=String(notes||'').trim().slice(0,400);
   return {
     title:'ChatX '+String(version||''),
     highlights:summary?[summary]:[],
     changelog:String(custom||'').trim().slice(0,16000),
   };
 }
 return {get,releases};
});
