param(
    [string]$NsisCompiler = "$env:LOCALAPPDATA/tauri/NSIS/Bin/makensis.exe"
)

$ErrorActionPreference = 'Stop'
$compiler = (Resolve-Path -LiteralPath $NsisCompiler).Path
Push-Location (Join-Path $PSScriptRoot 'src-tauri')
try {
    $version = (Get-Content -LiteralPath 'tauri.conf.json' -Raw | ConvertFrom-Json).version
    cargo build --locked --release --bins
    if ($LASTEXITCODE) { throw "Rust build failed ($LASTEXITCODE)." }

    $release = (Resolve-Path -LiteralPath 'target/release').Path
    $output = Join-Path $release 'community'
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    foreach ($role in 'HOST', 'CLIENT') {
        $script = Join-Path $PWD ($role.ToLowerInvariant() + '-installer.nsi')
        $installer = Join-Path $output "ChatX-$role-SETUP-v$version-Andromeda.exe"
        & $compiler /V2 "/DRELEASE=$release" "/DOUTPUT=$installer" $script
        if ($LASTEXITCODE) { throw "NSIS build for $role failed ($LASTEXITCODE)." }
        $file = Get-Item -LiteralPath $installer
        if ($file.VersionInfo.ProductVersion -ne $version) {
            throw "Unexpected product version in $($file.Name)."
        }
        Get-FileHash -LiteralPath $installer -Algorithm SHA256
    }
} finally {
    Pop-Location
}
