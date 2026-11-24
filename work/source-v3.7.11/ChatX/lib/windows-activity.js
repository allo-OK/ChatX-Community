const fs=require('fs'),path=require('path');
const fallback={app:'ChatX Chat',title:'',mediaTitle:'',mediaArtist:'',mediaAlbum:'',mediaSource:'',playback:''};
function parseOutput(stdout){
 const line=String(stdout||'').split(/\r?\n/).find(line=>line.startsWith('KC_ACTIVITY:'));
 if(!line)throw Error('Windows tidak mengembalikan data aktivitas');
 const raw=JSON.parse(Buffer.from(line.slice(12).trim(),'base64').toString('utf8'));
 if(!raw||typeof raw!=='object'||Array.isArray(raw))throw Error('Data aktivitas Windows tidak valid');
 const activity={};for(const [key,limit]of Object.entries({app:80,title:260,mediaTitle:260,mediaArtist:160,mediaAlbum:160,mediaSource:180,playback:40}))activity[key]=String(raw[key]||'').replace(/[\r\n\t]+/g,' ').trim().slice(0,limit);return activity;
}
module.exports=function({execFile,platform=process.platform,onFailure=()=>{},now=Date.now}){
 let inflight=null,next=0,last={ok:true,activity:{...fallback}};
 const encoded=Buffer.from(fs.readFileSync(path.join(__dirname,'windows-activity.ps1'),'utf8'),'utf16le').toString('base64');
 return function query(){
  if(platform!=='win32'||now()<next)return Promise.resolve(last);if(inflight)return inflight;
  inflight=new Promise(resolve=>{
   function complete(err,stdout){try{last={ok:true,activity:parseOutput(stdout)};next=now()+10000;}catch{last={ok:true,activity:{...fallback},unavailable:true};next=now()+60000;onFailure(err?.killed?'Pembacaan aktivitas Windows melewati batas waktu':'Pembacaan aktivitas Windows belum tersedia');}resolve(last);}
   try{execFile('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand',encoded],{windowsHide:true,timeout:10000,maxBuffer:1024*1024,encoding:'utf8'},complete);}catch(error){complete(error,'');}
  }).finally(()=>{inflight=null;});return inflight;
 };
};
module.exports.parseOutput=parseOutput;
