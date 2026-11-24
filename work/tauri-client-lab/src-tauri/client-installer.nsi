Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "chatx-update-shutdown.nsh"
!define PRODUCT "ChatX Community CLIENT"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ChatX Community CLIENT"

Name "ChatX"
!define MUI_ICON "icons\icon.ico"
!define MUI_UNICON "icons\icon.ico"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
VIProductVersion "1.1.43.0"
VIAddVersionKey "ProductName" "ChatX"
VIAddVersionKey "FileDescription" "Installer ChatX"
VIAddVersionKey "FileVersion" "1.1.43"
VIAddVersionKey "ProductVersion" "1.1.43"
VIAddVersionKey "CompanyName" "Allo"
VIAddVersionKey "LegalCopyright" "Allo"
Var ClientLaunchArgs

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Indonesian"

Section "ChatX Community CLIENT"
  SetShellVarContext current
  !insertmacro CloseChatXForUpdate "$INSTDIR\ChatX-CLIENT.exe"
  SetOutPath "$INSTDIR"
  ClearErrors
  File /oname=ChatX-Emblem.ico "icons\icon.ico"
  File /oname=ChatX-CLIENT.exe "${RELEASE}\chatx-client.exe"
  File "${RELEASE}\libvips-42.dll"
  File "${RELEASE}\libvips-LICENSE.txt"
  IfErrors install_failed
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  IfErrors install_failed
  StrCpy $1 ""
  CreateShortCut "$SMPROGRAMS\ChatX CLIENT.lnk" "$INSTDIR\ChatX-CLIENT.exe" "$1" "$INSTDIR\ChatX-Emblem.ico" 0
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ChatX Community CLIENT" '"$INSTDIR\ChatX-CLIENT.exe" '

  shortcut_ready:
  IfErrors install_failed
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "ChatX"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "1.1.43"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Allo"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  IfErrors install_failed
  Goto install_finished

  install_failed:
    SetErrorLevel 1
    Abort "Pemasangan HOST gagal. Tutup CLIENT jika masih berjalan, lalu coba lagi."
  install_finished:
    StrCpy $ClientLaunchArgs $1
SectionEnd

Function .onInstSuccess
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/NOLAUNCH" $1
  IfErrors +2
  Return
  Exec '"$INSTDIR\ChatX-CLIENT.exe" $ClientLaunchArgs'
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$INSTDIR\ChatX-Emblem.ico"
  Delete "$INSTDIR\ChatX-CLIENT.exe"
  Delete "$INSTDIR\libvips-42.dll"
  Delete "$INSTDIR\libvips-LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\ChatX CLIENT.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ChatX Community CLIENT"
  RMDir "$INSTDIR"
SectionEnd
