'use strict';
const queue=require('./notification-queue');
function create(options){
 const make=lane=>queue.create({...options,backlogVisibleMs:lane==='normal'?1500:5000,show:(payload,life)=>options.show({...payload,_notificationLane:lane},life)});
 const normal=make('normal'),mention=make('mention');
 return{push:payload=>(payload?.mention?mention:normal).push(payload),state:()=>({normal:normal.state(),mention:mention.state()}),diagnostics:()=>({normal:normal.diagnostics(),mention:mention.diagnostics()})};
}
module.exports={create};
