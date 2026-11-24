'use strict';
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const CACHE_DIRS = ['Cache', 'Code Cache', 'GPUCache', 'DawnCache'];
const RENAME_RETRYABLE = new Set(['EPERM', 'EACCES', 'EBUSY', 'ENOTEMPTY']);
async function renameWithRetry(source, destination, {attempts = 8, initialDelayMs = 60, maxDelayMs = 500} = {}) {
  let delay = initialDelayMs;
  for (let attempt = 1; ; attempt++) {
    try { fs.renameSync(source, destination); return; }
    catch (error) {
      if (attempt >= attempts || !RENAME_RETRYABLE.has(error?.code)) {
        const detail = error?.code || error?.message || 'unknown error';
        const wrapped = Error(`Gagal memindahkan checkpoint setelah ${attempt} percobaan (${detail}): ${path.basename(source)} → ${path.basename(destination)}`);
        wrapped.code = error?.code; wrapped.cause = error; throw wrapped;
      }
      await new Promise(resolve => setTimeout(resolve, delay));
      delay = Math.min(maxDelayMs, delay * 2);
    }
  }
}
function atomicJson(file, value) {
  fs.mkdirSync(path.dirname(file), {recursive:true});
  const temp = file + `.${process.pid}-${crypto.randomUUID()}.recovery-tmp`;
  try { fs.writeFileSync(temp, JSON.stringify(value, null, 2)); fs.renameSync(temp, file); }
  finally { try { fs.unlinkSync(temp); } catch {} }
}
function readObject(file) {
  const value = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw Error('Konfigurasi bukan objek JSON');
  return value;
}
function runStartupRecovery({userData, version, role}) {
  const root = path.resolve(userData), configFile = path.join(root, 'config.json');
  const recovery = path.join(root, 'recovery');
  const lastGood = path.join(recovery, 'config-last-good.json');
  const marker = path.join(recovery, `startup-${role}.json`);
  fs.mkdirSync(recovery, {recursive:true});
  let previous = {}; try { previous = readObject(marker); } catch {}
  let config = {}, restored = false;
  if (fs.existsSync(configFile)) {
    try { config = readObject(configFile); }
    catch (error) {
      try { config = readObject(lastGood); }
      catch { throw Error('Konfigurasi rusak dan salinan pemulihan belum tersedia. Data dipertahankan; ekspor log untuk pemulihan.'); }
      fs.copyFileSync(configFile, path.join(recovery, `config-damaged-${Date.now()}.json`));
      restored = true;
    }
  } else if (fs.existsSync(lastGood)) { config = readObject(lastGood); restored = true; }
  const changed = previous.version !== version || !previous.completed;
  const report = {version, role, completed:false, changed, restored, cleaned:[], warnings:[], startedAt:new Date().toISOString()};
  if (changed || restored) {
    // A checkpoint contains configuration only. Chat/WAL/uploads/outbox/IndexedDB are untouched.
    const checkpoint = path.join(recovery, `config-before-${role}-${String(version).replace(/[^0-9A-Za-z.-]/g,'_')}.json`);
    if (!fs.existsSync(checkpoint)) atomicJson(checkpoint, config);
    for (const key of ['serverAddress', 'lastServerAddress', 'lastDiscoveredServer', 'cachedServerUrl']) delete config[key];
    config.role = role === 'host' ? 'server' : 'client';
    config.configSchemaVersion = 1;
    atomicJson(configFile, config);
    for (const name of CACHE_DIRS) {
      const target = path.join(root, name);
      try {
        if (fs.existsSync(target) && !fs.lstatSync(target).isSymbolicLink()) {
          fs.rmSync(target, {recursive:true, force:true, maxRetries:2, retryDelay:100});
          report.cleaned.push(name);
        }
      } catch (error) { report.warnings.push(`${name}: ${error.message}`); }
    }
  }
  atomicJson(lastGood, config);
  report.completed = report.warnings.length === 0;
  atomicJson(marker, report);
  return report;
}
async function prepareUpgradeCheckpoint({userData, version, role, onProgress = () => {}}) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || !['host','client'].includes(role)) throw Error('Versi/peran checkpoint tidak valid');
  const root=path.resolve(userData),base=path.join(root,'recovery'),target=path.join(base,`checkpoint-${role}-${version}`);
  try { const prior=readObject(path.join(target,'CHECKPOINT.json')); if(prior.completed&&prior.version===version&&prior.role===role)return target; } catch {}
  fs.mkdirSync(base,{recursive:true});
  if(fs.existsSync(target)){
    if(fs.lstatSync(target).isSymbolicLink())throw Error('Checkpoint path tidak aman');
    // Preserve a damaged checkpoint, but allow a new valid backup on next start.
    await renameWithRetry(target,target+`.invalid-${Date.now()}-${crypto.randomUUID()}`);
    onProgress('Salinan backup yang tidak lengkap dipertahankan; membuat backup baru…');
  }
  const partial=target+'.incomplete';
  if(fs.existsSync(partial)&&fs.lstatSync(partial).isSymbolicLink())throw Error('Checkpoint path tidak aman');
  fs.rmSync(partial,{recursive:true,force:true});fs.mkdirSync(partial);
  const names=role==='host'?['config.json','outbox','server-data']:['config.json','outbox'];
  for(const name of names){const from=path.join(root,name);if(!fs.existsSync(from))continue;
    onProgress(`Menyalin backup ${name}…`);
    await fs.promises.cp(from,path.join(partial,name),{recursive:true,filter:source=>{
      if(fs.lstatSync(source).isSymbolicLink())throw Error('Symlink tidak didukung dalam checkpoint');
      const relative=path.relative(root,source).split(path.sep);
      return !(relative[0]==='server-data'&&['uploads','tmp','updates','quarantine'].includes(relative[1]));
    }});
  }
  const files=[];
  onProgress('Memeriksa integritas backup…');
  async function walk(dir){for(const entry of await fs.promises.readdir(dir,{withFileTypes:true})){const file=path.join(dir,entry.name);if(entry.isDirectory())await walk(file);else{
    const hash=crypto.createHash('sha256');for await(const chunk of fs.createReadStream(file))hash.update(chunk);
    files.push({path:path.relative(partial,file),size:(await fs.promises.stat(file)).size,sha256:hash.digest('hex')});
  }}}
  await walk(partial);
  atomicJson(path.join(partial,'CHECKPOINT.json'),{completed:true,version,role,createdAt:Date.now(),files,attachments:'Original uploads, installer cache and quarantine remain in active storage; this checkpoint contains application state only.'});
  if(fs.existsSync(target))throw Error('Checkpoint lama belum valid; data dipertahankan untuk pemeriksaan');
  await renameWithRetry(partial,target);
  const older=[];
  for(const name of fs.readdirSync(base)){if(!new RegExp(`^checkpoint-${role}-[0-9]+\\.[0-9]+\\.[0-9]+$`).test(name))continue;const dir=path.join(base,name);try{if(fs.lstatSync(dir).isSymbolicLink())continue;const info=readObject(path.join(dir,'CHECKPOINT.json'));if(info.completed&&info.role===role)older.push({dir,ts:info.createdAt});}catch{}}
  older.sort((a,b)=>Number(b.dir===target)-Number(a.dir===target)||b.ts-a.ts);
  for(const item of older.slice(2))try{fs.rmSync(item.dir,{recursive:true,force:true});}
  catch{onProgress('Backup siap; pembersihan backup lama tertunda karena file masih dipakai.');}
  return target;
}
module.exports = {runStartupRecovery, prepareUpgradeCheckpoint, atomicJson, CACHE_DIRS};
