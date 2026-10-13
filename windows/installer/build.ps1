# Seamtype for Windows: builds the installer, windows\target\installer\Seamtype-Setup-<version>.exe.
#
#   powershell -ExecutionPolicy Bypass -File windows\installer\build.ps1
#
# Needs Rust with the i686-pc-windows-msvc target (rustup target add i686-pc-windows-msvc) and
# Inno Setup 6 (winget install JRSoftware.InnoSetup).
$ErrorActionPreference = 'Stop'
$Workspace = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $Workspace 'Cargo.toml'
$version = (Select-String -Path $manifest -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
& $cargo build --release -p seamtype-tip --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw 'build failed (x64)' }
& $cargo build --release -p seamtype-tip --target i686-pc-windows-msvc --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw 'build failed (x86)' }

$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw 'Inno Setup 6 not found (winget install JRSoftware.InnoSetup)' }
& $iscc "/DVersion=$version" (Join-Path $PSScriptRoot 'seamtype.iss')
if ($LASTEXITCODE -ne 0) { throw 'installer build failed' }
Write-Host "windows\target\installer\Seamtype-Setup-$version.exe"
