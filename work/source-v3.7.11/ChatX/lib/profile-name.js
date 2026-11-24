function normalizeProfileName(value){
  const name=String(value??'').replace(/\s+/g,' ').trim();
  if(!name||name.length>40||/[\x00-\x1f\x7f]/.test(name))throw new Error('Nama profil harus diisi, maksimal 40 karakter.');
  return name;
}
function updateProfileName(profiles,identity,value,known=[]){
  const key=String(identity||'editor').trim().toLowerCase().replace(/[^a-z0-9._-]+/g,'-').replace(/^-+|-+$/g,'')||'editor',displayName=normalizeProfileName(value);
  const candidates=[...Object.values(profiles||{}),...known];
  if(candidates.some(p=>String(p.name||'').trim().toLowerCase()!==String(identity||'').trim().toLowerCase()&&String(p.displayName||p.name||'').trim().toLowerCase()===displayName.toLowerCase()))throw new Error('Nama profil sudah dipakai pengguna lain.');
  return {...profiles[key],name:identity,displayName,updatedAt:Date.now()};
}
module.exports={normalizeProfileName,updateProfileName};
