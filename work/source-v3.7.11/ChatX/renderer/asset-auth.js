(function(root,factory){
  const authenticatedAssetUrl=factory();
  if(typeof module==='object'&&module.exports)module.exports=authenticatedAssetUrl;
  if(root)root.KCAuthenticatedAssetUrl=authenticatedAssetUrl;
})(typeof window!=='undefined'?window:globalThis,function(){
  return function(value,{serverBase,cfg,sessionToken}={}){
    if(!value)return'';
    try{
      const url=new URL(value,serverBase),active=new URL(serverBase);
      if(url.origin!==active.origin)return'';
      let token=sessionToken||'';
      if(cfg?.isHost&&cfg?.adminToken){
        try{if(cfg.serverAddress&&url.origin===new URL(cfg.serverAddress).origin)token=cfg.adminToken;}catch{}
      }
      if(!token)return'';
      url.searchParams.set('kc_token',token);
      return url.href;
    }catch{return''}
  };
});
