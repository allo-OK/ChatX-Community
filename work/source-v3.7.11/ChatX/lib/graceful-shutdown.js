'use strict';
function installGracefulShutdown(app,{getServer,log=()=>{},timeoutMs=10000}){
  let state='idle';
  app.on('before-quit',event=>{
    const server=getServer();if(state==='done'||!server)return;
    event.preventDefault();app.isQuitting=true;if(state==='draining')return;state='draining';
    let timer;
    Promise.race([
      Promise.resolve().then(()=>server.shutdown()),
      new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Batas waktu shutdown HOST tercapai')),timeoutMs);})
    ]).then(()=>log('host-shutdown-complete'),error=>log('host-shutdown-warning',{message:error.message}))
      .finally(()=>{clearTimeout(timer);state='done';app.quit();});
  });
}
module.exports={installGracefulShutdown};
