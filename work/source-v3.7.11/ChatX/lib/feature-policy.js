const keys=['personal','editor','everyone','sendChat','sendFiles','downloadFiles','designAI','tagPrintTest','tagDesignRevision','tagDesignRequest','banned'];
function defaultPermissions(){return{personal:true,editor:false,everyone:false,sendChat:true,sendFiles:true,downloadFiles:true,designAI:true,tagPrintTest:true,tagDesignRevision:true,tagDesignRequest:true,banned:false};}
function cleanPermissions(raw={}){const result=defaultPermissions();for(const key of keys)if(typeof raw?.[key]==='boolean')result[key]=raw[key];return result;}
module.exports={keys,defaultPermissions,cleanPermissions};
