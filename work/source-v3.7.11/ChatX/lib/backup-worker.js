const {parentPort,workerData}=require('worker_threads');
(async()=>{try{const service=require('./backup-service');const result=await service[workerData.action](workerData.options);if(result.manifest)delete result.manifest;parentPort.postMessage(result)}catch(e){parentPort.postMessage({ok:false,error:e.message})}})();
