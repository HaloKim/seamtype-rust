# Seamtype for Windows: removes the input method.
#
#   powershell -ExecutionPolicy Bypass -File windows\scripts\uninstall.ps1 [-RemoveSettings]
#
# Takes Seamtype off your keyboards, unregisters it (one UAC prompt) and deletes its files.
# Settings and learned words (%APPDATA%\Seamtype) stay unless -RemoveSettings. A DLL still loaded
# by running apps is renamed and goes when they restart (or after the next sign-in).
[CmdletBinding()]
param(
    [switch]$RemoveSettings,
    # Internal: the unregister-and-delete part, run as administrator.
    [switch]$Elevated
)
$ErrorActionPreference = 'Stop'
$Clsid = '{2FF7B2B1-D7D9-4EA7-A2A5-4CC3EDAC15D8}'
$ProfileGuid = '{A13E3052-EF12-4BA3-BD09-87BA0C88957F}'
$Tip = "0412:$Clsid$ProfileGuid"
$Dest = Join-Path $env:ProgramFiles 'Seamtype'

if ($Elevated) {
    $x64 = Join-Path $Dest 'seamtype.dll'
    $x86 = Join-Path $Dest 'x86\seamtype.dll'
    if (Test-Path $x86) { Start-Process "$env:windir\SysWOW64\regsvr32.exe" -ArgumentList '/u', '/s', "`"$x86`"" -Wait }
    if (Test-Path $x64) { Start-Process "$env:windir\System32\regsvr32.exe" -ArgumentList '/u', '/s', "`"$x64`"" -Wait }
    Get-ChildItem $Dest -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object {
        try { Remove-Item $_.FullName -Force } catch { Rename-Item $_.FullName "$($_.Name).old" -Force -ErrorAction SilentlyContinue }
    }
    Remove-Item $Dest -Recurse -Force -ErrorAction SilentlyContinue
    exit 0
}

Add-Type -Namespace SeamtypeSetup -Name Input -MemberDefinition @'
[DllImport("input.dll", CharSet = CharSet.Unicode)]
public static extern bool InstallLayoutOrTip(string psz, uint flags);
'@
# ILOT_UNINSTALL
[void][SeamtypeSetup.Input]::InstallLayoutOrTip($Tip, 1)

Write-Host 'Unregistering Seamtype (Windows asks for administrator permission)...'
$arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Elevated')
$p = Start-Process powershell.exe -Verb RunAs -ArgumentList $arguments -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "unregistering failed (exit $($p.ExitCode))" }

if ($RemoveSettings) {
    Remove-Item (Join-Path $env:APPDATA 'Seamtype') -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host 'Seamtype is removed. / 제거했습니다.' -ForegroundColor Green
