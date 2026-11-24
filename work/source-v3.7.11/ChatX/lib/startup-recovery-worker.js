'use strict';
const {parentPort,workerData}=require('worker_threads');
const {prepareUpgradeCheckpoint,runStartupRecovery}=require('./startup-recovery');
(async()=>{
  const onProgress=message=>parentPort.postMessage({type:'progress',message});
  await prepareUpgradeCheckpoint({...workerData,onProgress});
  onProgress('Memeriksa konfigurasi dan cache…');
  const report=runStartupRecovery(workerData);
  parentPort.postMessage({type:'done',report});
})().catch(error=>parentPort.postMessage({type:'failed',message:error.message}));
