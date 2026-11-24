'use strict';

const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('kcSchedulePopup', {
  close: () => ipcRenderer.send('work-schedule-popup-close')
});
