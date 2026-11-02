# Uninstalls DepRadar on Windows by locating the uninstaller it registered
# with Windows (same registry keys "Apps & Features" reads from), then removes
# the data it keeps (saved projects, webview cache).
$ErrorActionPreference = "Stop"

$uninstallKeys = @(
    "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
    "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
    "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
)

$entry = Get-ItemProperty $uninstallKeys -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -eq "DepRadar" } |
    Select-Object -First 1

if (-not $entry -or -not $entry.UninstallString) {
    Write-Warning "Could not find DepRadar's registered uninstaller. Remove it from Settings > Apps > Installed apps instead."
    exit 1
}

Write-Host "Running: $($entry.UninstallString)"
Start-Process -FilePath "cmd.exe" -ArgumentList "/c", $entry.UninstallString -Wait

foreach ($dir in @("$env:APPDATA\com.sikander.depradar", "$env:LOCALAPPDATA\com.sikander.depradar")) {
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
}

Write-Host "DepRadar uninstalled."
