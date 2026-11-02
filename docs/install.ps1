# Installs DepRadar on Windows. Always fetches whatever release is
# currently "latest" on GitHub, so this script doesn't need editing when a
# new version ships. The installer is verified against the release's
# SHA256SUMS file before it runs.
$ErrorActionPreference = "Stop"

$headers = @{}
if ($env:GITHUB_TOKEN) { $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN" }

try {
    $release = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/depradar-app/depradar/releases/latest"
} catch {
    Write-Error "Could not query GitHub for the latest release (rate limit? set `$env:GITHUB_TOKEN, or download it from https://github.com/depradar-app/depradar/releases/latest)."
    exit 1
}

# `*x64-setup.exe` matches the END of the name, so `...-setup.exe.sig` is skipped.
$asset = $release.assets | Where-Object { $_.name -like "*x64-setup.exe" } | Select-Object -First 1
if (-not $asset) {
    Write-Error "Could not find a Windows installer in the latest release."
    exit 1
}

$out = Join-Path $env:TEMP $asset.name
Write-Host "Downloading $($asset.browser_download_url)"
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $out

$sums = $release.assets | Where-Object { $_.name -eq "SHA256SUMS" } | Select-Object -First 1
if ($sums) {
    $sumsPath = Join-Path $env:TEMP "depradar-SHA256SUMS"
    Invoke-WebRequest -Uri $sums.browser_download_url -OutFile $sumsPath
    $line = Get-Content $sumsPath | Where-Object { $_ -match [regex]::Escape($asset.name) } | Select-Object -First 1
    Remove-Item $sumsPath -ErrorAction SilentlyContinue
    if (-not $line) {
        Remove-Item $out -ErrorAction SilentlyContinue
        Write-Error "No checksum listed for $($asset.name) in SHA256SUMS."
        exit 1
    }
    $expected = ($line -split "\s+")[0]
    $actual = (Get-FileHash -Algorithm SHA256 -Path $out).Hash
    if ($actual -ne $expected) {
        Remove-Item $out -ErrorAction SilentlyContinue
        Write-Error "Checksum mismatch for $($asset.name) (expected $expected, got $actual). Aborting."
        exit 1
    }
    Write-Host "Checksum OK."
} else {
    Write-Warning "This release has no SHA256SUMS file; the download could not be verified."
}

Write-Host "Running installer..."
Start-Process -FilePath $out -Wait

Write-Host "DepRadar installed."
