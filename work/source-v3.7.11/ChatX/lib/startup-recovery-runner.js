'use strict';
const {Worker}=require('worker_threads');
const path=require('path');
function prepareStartupRecovery({onProgress=()=>{},...workerData}){
  return new Promise((resolve,reject)=>{
    const worker=new Worker(path.join(__dirname,'startup-recovery-worker.js'),{workerData});
    let complete=false;
    const finish=(error,report)=>{if(complete)return;complete=true;error?reject(error):resolve(report);};
    worker.on('message',event=>{
      if(event.type==='progress'){try{onProgress(event.message);}catch{}return;}
      if(event.type==='done')finish(null,event.report);
      if(event.type==='failed')finish(Error(event.message));
    });
    worker.on('error',error=>finish(error));
    worker.on('exit',code=>{if(!complete)finish(Error(`Pemulihan startup berhenti sebelum selesai (kode ${code}).`));});
  });
}
module.exports={prepareStartupRecovery};
