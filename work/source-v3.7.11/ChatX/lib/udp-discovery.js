'use strict';
const dgram=require('dgram');
const {probeHealth}=require('./health-probe');
function discoverUdp({targets,port=45821,timeout=1100,probe=probeHealth,createSocket=dgram.createSocket}={}){
  return new Promise(resolve=>{
    const socket=createSocket('udp4'),seen=new Set();let done=false;
    const finish=value=>{if(done)return;done=true;clearTimeout(timer);try{socket.close();}catch{}resolve(value);};
    const timer=setTimeout(()=>finish(null),timeout);
    socket.on('error',()=>finish(null));
    socket.on('message',(buffer,remote)=>{
      if(done||buffer.length>4096)return;
      let packet;try{packet=JSON.parse(buffer.toString());}catch{return;}
      const targetPort=packet.port===undefined?45820:Number(packet.port);
      if(packet.service!=='CHATX_COMMUNITY'||!Number.isInteger(targetPort)||targetPort<1||targetPort>65535)return;
      const address=`http://${remote.address}:${targetPort}`;
      if(seen.has(address)||seen.size>=32)return;seen.add(address);
      // A valid UDP response alone is insufficient: HTTP may be blocked/not ready.
      Promise.resolve().then(()=>probe(address,Math.min(600,timeout))).then(ok=>{if(ok)finish(address);},()=>{});
    });
    try{socket.bind(()=>{
      if(done){try{socket.close();}catch{}return;}
      try{socket.setBroadcast(true);const query=Buffer.from('CHATX_COMMUNITY_DISCOVER');
        for(const target of targets||[])socket.send(query,port,target,()=>{});
      }catch{finish(null);}
    });}catch{finish(null);}
  });
}
module.exports={discoverUdp};
