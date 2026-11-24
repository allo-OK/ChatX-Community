!define CHATX_SHUTDOWN_SCRIPT "${__FILEDIR__}\close-chatx-for-update.ps1"

!macro CloseChatXForUpdate EXECUTABLE
  InitPluginsDir
  File /oname=$PLUGINSDIR\close-chatx-for-update.ps1 "${CHATX_SHUTDOWN_SCRIPT}"
  Push $R9
  StrCpy $R9 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe" 0 +2
  StrCpy $R9 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  nsExec::ExecToLog /TIMEOUT=90000 '"$R9" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\close-chatx-for-update.ps1" "${EXECUTABLE}"'
  Pop $0
  Pop $R9
  StrCmp $0 "0" +3
    SetErrorLevel 1
    Abort "ChatX belum bisa ditutup dengan aman. Keluar dari tray, lalu jalankan installer lagi."
!macroend
