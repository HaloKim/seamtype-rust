# Seamtype for Windows: installs or updates the input method.
#
#   powershell -ExecutionPolicy Bypass -File windows\scripts\install.ps1 [-NoBuild] [-NotDefault]
#
# Builds the text service (x64, and x86 for 32-bit apps when that Rust target is installed)
# unless -NoBuild, copies it to %ProgramFiles%\Seamtype, registers it (one UAC prompt), and adds
# Seamtype to your Korean keyboards: first in the list and the default input method (what Windows
# starts with after signing in), unless -NotDefault. Apps already running keep the copy they loaded until they
# restart; a new window of theirs may need the app restarted to pick up the update.
[CmdletBinding()]
param(
    [switch]$NoBuild,
    [switch]$NotDefault,
    # Internal: the copy-and-register part, run as administrator from the staged files.
    [switch]$Elevated,
    [string]$Source
)
$ErrorActionPreference = 'Stop'
$Clsid = '{2FF7B2B1-D7D9-4EA7-A2A5-4CC3EDAC15D8}'
$ProfileGuid = '{A13E3052-EF12-4BA3-BD09-87BA0C88957F}'
$Tip = "0412:$Clsid$ProfileGuid"
$Workspace = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $env:ProgramFiles 'Seamtype'

function Install-Dll([string]$From, [string]$To) {
    if (Test-Path $To) {
        try {
            Remove-Item $To -Force
        } catch {
            # Loaded by running apps: a loaded DLL can still be renamed, and they keep using it.
            $old = "$(Split-Path -Leaf $To).$(Get-Date -Format yyyyMMddHHmmss).old"
            Rename-Item $To $old -Force
        }
    }
    Copy-Item $From $To -Force
    Get-ChildItem (Split-Path $To) -Filter '*.old' -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
}

function Register-Dll([string]$Regsvr32, [string]$Dll) {
    $p = Start-Process $Regsvr32 -ArgumentList '/s', "`"$Dll`"" -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "regsvr32 failed for $Dll (exit $($p.ExitCode))" }
}

if ($Elevated) {
    try {
        New-Item -ItemType Directory -Force $Dest | Out-Null
        Install-Dll (Join-Path $Source 'x64\seamtype.dll') (Join-Path $Dest 'seamtype.dll')
        Register-Dll "$env:windir\System32\regsvr32.exe" (Join-Path $Dest 'seamtype.dll')
        $x86 = Join-Path $Source 'x86\seamtype.dll'
        if (Test-Path $x86) {
            New-Item -ItemType Directory -Force (Join-Path $Dest 'x86') | Out-Null
            Install-Dll $x86 (Join-Path $Dest 'x86\seamtype.dll')
            Register-Dll "$env:windir\SysWOW64\regsvr32.exe" (Join-Path $Dest 'x86\seamtype.dll')
        }
        foreach ($doc in 'LICENSE', 'ThirdPartyNotices.txt') {
            $file = Join-Path $Source $doc
            if (Test-Path $file) { Copy-Item $file $Dest -Force }
        }
        exit 0
    } catch {
        Write-Host $_ -ForegroundColor Red
        Start-Sleep -Seconds 5
        exit 1
    }
}

# 1. Build.
$x64 = Join-Path $Workspace 'target\release\seamtype.dll'
$x86 = Join-Path $Workspace 'target\i686-pc-windows-msvc\release\seamtype.dll'
if (-not $NoBuild) {
    $cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
    if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
    $manifest = Join-Path $Workspace 'Cargo.toml'
    & $cargo build --release -p seamtype-tip --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw 'build failed (x64)' }
    $rustup = Join-Path (Split-Path $cargo) 'rustup.exe'
    if ((Test-Path $rustup) -and ((& $rustup target list --installed) -contains 'i686-pc-windows-msvc')) {
        & $cargo build --release -p seamtype-tip --target i686-pc-windows-msvc --manifest-path $manifest
        if ($LASTEXITCODE -ne 0) { throw 'build failed (x86)' }
    } else {
        Write-Host 'x86 target not installed: 32-bit apps will not get Seamtype (rustup target add i686-pc-windows-msvc).'
        $x86 = $null
    }
}
if (-not (Test-Path $x64)) { throw "not built: $x64" }

# 2. Stage the files and copy and register them as administrator.
$stage = Join-Path $env:TEMP 'seamtype-install'
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $stage 'x64') | Out-Null
Copy-Item $x64 (Join-Path $stage 'x64\seamtype.dll')
if ($x86 -and (Test-Path $x86)) {
    New-Item -ItemType Directory -Force (Join-Path $stage 'x86') | Out-Null
    Copy-Item $x86 (Join-Path $stage 'x86\seamtype.dll')
}
$repo = Split-Path -Parent $Workspace
Copy-Item (Join-Path $repo 'LICENSE') $stage -ErrorAction SilentlyContinue
Copy-Item (Join-Path $Workspace 'ThirdPartyNotices.txt') $stage -ErrorAction SilentlyContinue

Write-Host 'Registering Seamtype (Windows asks for administrator permission)...'
$arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Elevated', '-Source', "`"$stage`"")
$p = Start-Process powershell.exe -Verb RunAs -ArgumentList $arguments -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "copying or registering failed (exit $($p.ExitCode))" }
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue

# 3. Add Seamtype to this user's Korean keyboards.
Add-Type -Namespace SeamtypeSetup -Name Input -MemberDefinition @'
[DllImport("input.dll", CharSet = CharSet.Unicode)]
public static extern bool InstallLayoutOrTip(string psz, uint flags);
'@
if (-not [SeamtypeSetup.Input]::InstallLayoutOrTip($Tip, 0)) {
    Write-Warning 'Could not add Seamtype to your keyboards: add it in Settings > Time & language > Language & region > Korean > Language options > Keyboards.'
} elseif (-not $NotDefault) {
    # First among the Korean keyboards, and the default input method: otherwise Windows starts
    # every sign-in with the keyboard listed first (usually Microsoft's).
    $languages = Get-WinUserLanguageList
    $korean = $languages | Where-Object { $_.LanguageTag -like 'ko*' } | Select-Object -First 1
    if ($korean) {
        [string[]]$others = @($korean.InputMethodTips | Where-Object { $_ -ne $Tip } | ForEach-Object { [string]$_ })
        $korean.InputMethodTips.Clear()
        $korean.InputMethodTips.Add($Tip)
        foreach ($other in $others) { $korean.InputMethodTips.Add($other) }
        Set-WinUserLanguageList $languages -Force -WarningAction SilentlyContinue
    }
    Set-WinDefaultInputMethodOverride -InputTip $Tip
}

Write-Host ''
Write-Host 'Seamtype is installed. Press Win+Space and choose Seamtype (Korean).' -ForegroundColor Green
Write-Host '설치했습니다. Win+Space로 한국어 > Seamtype을 고르세요.' -ForegroundColor Green
