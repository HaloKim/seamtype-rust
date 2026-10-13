# Seamtype for Windows: adds Seamtype to (or removes it from) this user's Korean keyboards.
# Run by the installer as the user who installed, not as administrator.
#
#   keyboard.ps1 -Add [-Default]   add it; with -Default, first in the list and the default input
#                                  method (what Windows starts with after signing in)
#   keyboard.ps1 -Remove           remove it, and the default if it was Seamtype
[CmdletBinding()]
param([switch]$Add, [switch]$Default, [switch]$Remove)
$ErrorActionPreference = 'Stop'
$Tip = '0412:{2FF7B2B1-D7D9-4EA7-A2A5-4CC3EDAC15D8}{A13E3052-EF12-4BA3-BD09-87BA0C88957F}'

Add-Type -Namespace SeamtypeSetup -Name Input -MemberDefinition @'
[DllImport("input.dll", CharSet = CharSet.Unicode)]
public static extern bool InstallLayoutOrTip(string psz, uint flags);
'@

if ($Remove) {
    # ILOT_UNINSTALL
    [void][SeamtypeSetup.Input]::InstallLayoutOrTip($Tip, 1)
    $profileKey = 'HKCU:\Control Panel\International\User Profile'
    if ((Get-ItemProperty $profileKey -ErrorAction SilentlyContinue).InputMethodOverride -eq $Tip) {
        Remove-ItemProperty $profileKey -Name InputMethodOverride -ErrorAction SilentlyContinue
    }
    exit 0
}

if ($Add) {
    if (-not [SeamtypeSetup.Input]::InstallLayoutOrTip($Tip, 0)) { exit 1 }
    if ($Default) {
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
    exit 0
}
