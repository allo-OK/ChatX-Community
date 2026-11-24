'use strict';
(function(root){
  function resolveHandoverAddress(announced,hostMode){
    const value=String(announced||'');
    if(!/^https?:\/\/[^\s]+$/i.test(value)||!hostMode)return value;
    try{
      const url=new URL(value);
      url.hostname='127.0.0.1';
      url.pathname='/';url.search='';url.hash='';
      return url.origin;
    }catch{return 'http://127.0.0.1:45820'}
  }
  root.KCHandoverAddress={resolve:resolveHandoverAddress};
  if(typeof module!=='undefined'&&module.exports)module.exports={resolveHandoverAddress};
})(typeof globalThis!=='undefined'?globalThis:this);
