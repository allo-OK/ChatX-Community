'use strict';
// Closed launcher/worker output pipes are not application failures. Other errors still propagate.
const fs=require('node:fs'),path=require('node:path');
const installed=new WeakSet();
function install({streams=[process.stdout,process.stderr],file,consoleObject=console}={}){
 let recorded=false;
 const isPipeError=error=>error?.code==='EPIPE';
 function record(name,error){if(recorded)return;recorded=true;try{if(file){fs.mkdirSync(path.dirname(file),{recursive:true});if(fs.existsSync(file)&&fs.statSync(file).size>256*1024)fs.renameSync(file,file+'.previous');fs.appendFileSync(file,JSON.stringify({at:new Date().toISOString(),type:'stdio-pipe-closed',stream:name,code:error.code,message:'Output launcher/worker closed; app continues logging to files'})+'\n');}}catch{}}
 for(const [index,stream]of streams.entries()){
  if(!stream||installed.has(stream))continue;installed.add(stream);let broken=false;const write=stream.write;
  stream.on('error',error=>{if(!isPipeError(error))throw error;broken=true;record(index===0?'stdout':'stderr',error);queueMicrotask(()=>stream.emit('drain'));});
  stream.write=function(...args){if(broken){const callback=typeof args.at(-1)==='function'?args.at(-1):null;if(callback)queueMicrotask(()=>callback());return true;}try{return write.apply(this,args);}catch(error){if(!isPipeError(error))throw error;broken=true;record(index===0?'stdout':'stderr',error);const callback=typeof args.at(-1)==='function'?args.at(-1):null;if(callback)queueMicrotask(()=>callback());return true;}};
 }
 // Console may itself throw synchronously when its destination was detached.
 for(const name of ['log','info','warn','error','debug']){const original=consoleObject?.[name];if(typeof original!=='function')continue;consoleObject[name]=function(...args){try{return original.apply(this,args);}catch(error){if(!isPipeError(error))throw error;record('console',error);}};}
}
module.exports={install};
