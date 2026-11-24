'use strict';
const path = require('path');

function createStartupWindow({BrowserWindow, onClose = () => {}, timeoutMs = 15000}) {
  const win = new BrowserWindow({show:true,width:560,height:330,resizable:false,
    autoHideMenuBar:true,title:'ChatX Chat — Memulai',backgroundColor:'#0b1017',
    icon:path.join(__dirname,'../assets/app_icon.png'),
    webPreferences:{contextIsolation:true,nodeIntegration:false,sandbox:true}});
  let closedByApp = false, status = 'Menyiapkan aplikasi…';
  const update = message => {
    status = String(message);
    if(win.isDestroyed())return;
    win.webContents.executeJavaScript(`document.getElementById('status').textContent=${JSON.stringify(status)}`).catch(()=>{});
  };
  win.on('close',()=>{if(!closedByApp)onClose();});
  let timer;
  const ready = Promise.race([
    win.loadFile(path.join(__dirname,'../renderer/startup.html')),
    new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Jendela startup gagal dimuat dalam 15 detik.')),timeoutMs);})
  ]).finally(()=>clearTimeout(timer)).then(()=>update(status));
  return {ready,update,
    focus(){if(!win.isDestroyed()){if(win.isMinimized())win.restore();win.show();win.focus();}},
    finish(){closedByApp=true;if(!win.isDestroyed())win.destroy();}
  };
}

function revealMainWindow(win, onVisible = () => {}) {
  let shown = false;
  const show = () => {if(shown || win.isDestroyed())return;shown=true;win.show();onVisible();};
  win.once('ready-to-show',show);
  // A successful load must also reveal the window if ready-to-show is missed.
  win.webContents.once('did-finish-load',show);
}
module.exports = {createStartupWindow,revealMainWindow};
