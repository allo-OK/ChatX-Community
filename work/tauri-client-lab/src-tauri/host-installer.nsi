Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "chatx-update-shutdown.nsh"
!define PRODUCT "ChatX Community HOST"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ChatX Community HOST"

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
Var HostLaunchArgs

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Indonesian"

Section "ChatX Community HOST"
  SetShellVarContext current
  !insertmacro CloseChatXForUpdate "$INSTDIR\ChatX-HOST.exe"
  SetOutPath "$INSTDIR"
  ClearErrors
  File /oname=ChatX-Emblem.ico "icons\icon.ico"
  File /oname=ChatX-HOST.exe "${RELEASE}\chatx-host.exe"
  File "${RELEASE}\libvips-42.dll"
  File "${RELEASE}\libvips-LICENSE.txt"
  IfErrors install_failed
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  IfErrors install_failed
  StrCpy $1 "--lan-host-auto"
  CreateShortCut "$SMPROGRAMS\ChatX HOST.lnk" "$INSTDIR\ChatX-HOST.exe" "$1" "$INSTDIR\ChatX-Emblem.ico" 0
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ChatX Community HOST" '"$INSTDIR\ChatX-HOST.exe" --lan-host-auto'

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
    Abort "Pemasangan HOST gagal. Tutup HOST Rust jika masih berjalan, lalu coba lagi."
  install_finished:
    StrCpy $HostLaunchArgs $1
SectionEnd

Function .onInstSuccess
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/NOLAUNCH" $1
  IfErrors +2
  Return
  Exec '"$INSTDIR\ChatX-HOST.exe" $HostLaunchArgs'
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$INSTDIR\ChatX-Emblem.ico"
  Delete "$INSTDIR\ChatX-HOST.exe"
  Delete "$INSTDIR\libvips-42.dll"
  Delete "$INSTDIR\libvips-LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\ChatX HOST.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ChatX Community HOST"
  RMDir "$INSTDIR"
SectionEnd
