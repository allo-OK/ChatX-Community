param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference = 'Stop'
$path = [IO.Path]::GetFullPath($Executable)
$name = [IO.Path]::GetFileNameWithoutExtension($path)
$sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
$running = @(Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object {
    $_.SessionId -eq $sessionId -and $_.Path -eq $path
})
if (!$running.Count) { exit 0 }

Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

public static class ChatXUpdateShutdown {
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    public struct App {
        public int Pid;
        public long StartTime;
    }

    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmStartSession(out uint session, int flags, StringBuilder key);
    [DllImport("rstrtmgr.dll")]
    static extern int RmRegisterResources(uint session, uint files, IntPtr filenames,
        uint count, [In] App[] apps, uint services, IntPtr names);
    [DllImport("rstrtmgr.dll")]
    static extern int RmShutdown(uint session, uint flags, IntPtr callback);
    [DllImport("rstrtmgr.dll")]
    static extern int RmEndSession(uint session);

    public static int Close(Process[] processes) {
        var apps = Array.ConvertAll(processes, p => new App {
            Pid = p.Id, StartTime = p.StartTime.ToFileTimeUtc()
        });
        uint session;
        int result = RmStartSession(out session, 0, new StringBuilder(33));
        if (result != 0) return result;
        try {
            result = RmRegisterResources(session, 0, IntPtr.Zero, (uint)apps.Length,
                apps, 0, IntPtr.Zero);
            return result == 0 ? RmShutdown(session, 0, IntPtr.Zero) : result;
        } finally {
            RmEndSession(session);
        }
    }
}
'@

$result = [ChatXUpdateShutdown]::Close([Diagnostics.Process[]]$running)
foreach ($process in $running) {
    if (!$process.WaitForExit(3000)) {
        throw "ChatX masih berjalan. Windows Restart Manager: $result"
    }
}
