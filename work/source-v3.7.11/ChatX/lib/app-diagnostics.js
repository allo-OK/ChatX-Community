const crypto=require('node:crypto');
function create({now=Date.now,limit=600,onEvent=()=>{},sessionId=crypto.randomUUID()}={}){
 const events=[],operations=new Map(),active=new Map();let sequence=0,dropped=0;
 const allowed=['operation','phase','ok','durationMs','requestId','code','windowId','count','memoryMB','lagMs','visible','online','width','height','exitCode'];
 function record(type,detail={}){
  const event={id:++sequence,at:now(),sessionId,type:String(type).replace(/[^a-zA-Z0-9_.:-]/g,'_').slice(0,80)};
  for(const key of allowed){const value=detail[key];if(typeof value==='number'&&Number.isFinite(value)||typeof value==='boolean')event[key]=value;else if(typeof value==='string'&&['operation','phase','code'].includes(key))event[key]=value.replace(/[^a-zA-Z0-9_.:-]/g,'_').slice(0,120);}
  const key=event.windowId+':'+event.requestId;
  if(event.type==='ipc'&&event.phase==='start'){if(active.size>=300){active.delete(active.keys().next().value);dropped++;}active.set(key,event);}
  if(event.type==='ipc'&&event.phase==='end')active.delete(key);
  events.push(event);if(events.length>limit)events.shift();
  if(event.operation&&event.phase==='end'){
   const previous=operations.get(event.operation)||{count:0,failed:0,slow:0,totalMs:0,maxMs:0};previous.count++;if(event.ok===false)previous.failed++;const ms=Math.max(0,Number(event.durationMs)||0);if(ms>3000)previous.slow++;previous.totalMs+=ms;previous.maxMs=Math.max(previous.maxMs,ms);if(operations.size<300||operations.has(event.operation))operations.set(event.operation,previous);
  }
  try{onEvent(event);}catch{}
 }
 return {record,snapshot(){return {schema:2,sessionId,dropped,events:events.slice(),pending:[...active.values()].map(e=>({...e,elapsedMs:Math.max(0,now()-e.at)})),operations:Object.fromEntries([...operations].map(([key,value])=>[key,{...value,averageMs:Math.round(value.totalMs/value.count)}]))};}};
}
function observeContents(contents,record){
 const windowId=contents.id;
 for(const name of ['did-start-loading','dom-ready','did-finish-load','did-stop-loading','destroyed','unresponsive','responsive'])contents.on(name,()=>record('window:'+name,{windowId}));
 contents.on('did-fail-load',(_event,errorCode,_description,_url,isMainFrame)=>{if(isMainFrame)record('window:load-failed',{windowId,code:String(errorCode)});});
 contents.on('render-process-gone',(_event,details)=>record('window:renderer-gone',{windowId,code:details.reason,exitCode:details.exitCode}));
}
module.exports={create,observeContents};
