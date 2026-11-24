function gameTypePolicy(value){
  const requested=String(value||'chess');
  if(requested==='slither')return{ok:false,error:'Slither sudah dihapus dari aplikasi.'};
  const type=['ludo','snakes'].includes(requested)?requested:'chess';
  return{ok:true,type,maxInvites:type==='ludo'?3:1};
}
module.exports={gameTypePolicy};
