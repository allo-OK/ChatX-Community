
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding($false)
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class KCForeground {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
}
'@
$h=[KCForeground]::GetForegroundWindow()
$sb=New-Object System.Text.StringBuilder 2048
[KCForeground]::GetWindowText($h,$sb,$sb.Capacity)|Out-Null
[uint32]$pid2=0
[KCForeground]::GetWindowThreadProcessId($h,[ref]$pid2)|Out-Null
$p=Get-Process -Id $pid2 -ErrorAction SilentlyContinue
$o=[ordered]@{app=if($p){[string]$p.ProcessName}else{''};title=[string]$sb.ToString();mediaTitle='';mediaArtist='';mediaAlbum='';mediaSource='';playback=''}
try {
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  [void][Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager,Windows.Media.Control,ContentType=WindowsRuntime]
  [void][Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties,Windows.Media.Control,ContentType=WindowsRuntime]
  $asTask=([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {$_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Count -eq 1} | Select-Object -First 1)
  function Await-KC($op,$type){$t=$asTask.MakeGenericMethod($type).Invoke($null,@($op));if(!$t.Wait(900)){throw 'Media query timeout'};return $t.Result}
  $mgr=Await-KC ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
  $session=$mgr.GetCurrentSession()
  if($session){
    $props=Await-KC ($session.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
    $o.mediaTitle=[string]$props.Title
    $o.mediaArtist=[string]$props.Artist
    $o.mediaAlbum=[string]$props.AlbumTitle
    $o.mediaSource=[string]$session.SourceAppUserModelId
    $o.playback=[string]$session.GetPlaybackInfo().PlaybackStatus
  }
} catch {}
$json=$o | ConvertTo-Json -Compress
'KC_ACTIVITY:'+ [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json))
