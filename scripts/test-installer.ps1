param([switch]$Production)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$pattern = if ($Production) { 'Coterm_*_x64-setup.exe' } else { 'Coterm Smoke_*_x64-setup.exe' }
$installer = Get-ChildItem -LiteralPath (Join-Path $repo 'target/release/bundle/nsis') -Filter $pattern | Select-Object -First 1
if (-not $installer) { throw 'Build the isolated smoke NSIS installer first.' }
$product = if ($Production) { 'Coterm' } else { 'Coterm Smoke' }
$registrations = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq $product }
if ($registrations) { throw "An existing $product installation is registered; refusing to overwrite it during acceptance." }
$installRoot = Join-Path ([IO.Path]::GetTempPath()) ('coterm-install-' + [guid]::NewGuid())
$resolvedRoot = [IO.Path]::GetFullPath($installRoot)
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
if (-not $resolvedRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected installation target.' }
try {
    $install = Start-Process -FilePath $installer.FullName -ArgumentList "/S /D=$installRoot" -Wait -PassThru -WindowStyle Hidden
    if ($install.ExitCode -ne 0) { throw "Installer failed: $($install.ExitCode)" }
    $application = Join-Path $installRoot 'coterm-desktop.exe'
    if (-not (Test-Path -LiteralPath $application)) { throw 'Installed application is missing.' }
    Push-Location $repo
    try {
        if (-not $Production) {
            & node scripts/test-native.mjs $application
            if ($LASTEXITCODE -ne 0) { throw 'Native acceptance failed.' }
        }
    } finally { Pop-Location }
} finally {
    $uninstaller = Join-Path $installRoot 'uninstall.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        $uninstall = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru -WindowStyle Hidden
        if ($uninstall.ExitCode -ne 0) { throw "Uninstaller failed: $($uninstall.ExitCode)" }
        for ($attempt = 0; $attempt -lt 100 -and (Test-Path -LiteralPath (Join-Path $installRoot 'coterm-desktop.exe')); $attempt++) {
            Start-Sleep -Milliseconds 100
        }
        if (Test-Path -LiteralPath (Join-Path $installRoot 'coterm-desktop.exe')) { throw 'Uninstaller did not remove the application.' }
    }
    # The target is a verified unique directory under the OS temp directory.
    if (Test-Path -LiteralPath $resolvedRoot) { Remove-Item -LiteralPath $resolvedRoot -Recurse -Force }
}
Write-Output "PASS NSIS install/uninstall: $($installer.Name)"
