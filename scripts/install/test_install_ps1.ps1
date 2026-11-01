# Run with Windows PowerShell 5.1. All downloads and installations are isolated.
$ErrorActionPreference = "Stop"
$root = Join-Path ([IO.Path]::GetTempPath()) ("codex-installer-test-" + [guid]::NewGuid())
$installer = Join-Path $PSScriptRoot "install.ps1"
$savedEnvironment = @{}
foreach ($name in @("CODEX_HOME", "CODEX_INSTALL_DIR", "CODEX_INSTALL_DAEMON_ONLY", "CODEX_NON_INTERACTIVE", "CODEX_RELEASE", "CODEX_INSTALL_IF_LATEST", "CODEX_INSTALL_IF_CURRENT", "CODEX_UPDATE_FROM_RELEASE")) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
    [Environment]::SetEnvironmentVariable($name, $null)
}
function Assert-Equal($Actual, $Expected) {
    if ($Actual -cne $Expected) { throw "Expected '$Expected', got '$Actual'." }
}
function Invoke-RestMethod {
    param($Uri)
    $script:requests.Add([string]$Uri)
    return $script:metadata
}
function Invoke-WebRequest {
    param($Uri, $OutFile, [switch]$UseBasicParsing, $TimeoutSec)
    $script:requests.Add([string]$Uri)
    Copy-Item -LiteralPath $script:downloads[[string]$Uri] -Destination $OutFile
}
function New-TestRelease($Version) {
    $source = Join-Path $root $Version
    $package = Join-Path $source "package"
    foreach ($dir in @("bin", "codex-path", "codex-resources")) {
        New-Item -ItemType Directory -Force (Join-Path $package $dir) | Out-Null
    }
    foreach ($file in @("bin/codex.exe", "bin/codex-code-mode-host.exe", "bin/codex-exec-server.exe", "bin/hpatch.exe", "codex-path/rg.exe", "codex-resources/codex-command-runner.exe", "codex-resources/codex-windows-sandbox-setup.exe")) {
        Copy-Item $script:stub (Join-Path $package $file)
    }
    [IO.File]::WriteAllText((Join-Path $package "bin/version.txt"), $Version)
    @{
        layoutVersion = 1
        version = $Version
        target = "x86_64-pc-windows-msvc"
        entrypoint = "bin/codex.exe"
        execServer = "bin/codex-exec-server.exe"
    } | ConvertTo-Json | Set-Content (Join-Path $package "codex-package.json")
    $asset = "codex-doca-x86_64-pc-windows-msvc.zip"
    $archive = Join-Path $source $asset
    Compress-Archive -Path "$package/*" -DestinationPath $archive
    $checksum = Join-Path $source "SHA256SUMS"
    $archiveDigest = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($checksum, "$archiveDigest  $asset`n")
    $baseUrl = "https://github.com/DOCaCola/codex/releases/download/doca-v$Version"
    $script:downloads = @{ "$baseUrl/$asset" = $archive; "$baseUrl/SHA256SUMS" = $checksum }
    $script:metadata = [pscustomobject]@{
        tag_name = "doca-v$Version"
        assets = @(
            [pscustomobject]@{ name = $asset; browser_download_url = "$baseUrl/$asset"; digest = "sha256:$archiveDigest" }
            [pscustomobject]@{ name = "SHA256SUMS"; browser_download_url = "$baseUrl/SHA256SUMS"; digest = "sha256:$((Get-FileHash $checksum -Algorithm SHA256).Hash.ToLowerInvariant())" }
        )
    }
    return $archive
}
try {
    New-Item -ItemType Directory $root | Out-Null
    $stub = Join-Path $root "stub.exe"
    $sourceCode = Join-Path $root "stub.cs"
    @'
using System;
using System.IO;
using System.Reflection;
class Stub {
    static int Main(string[] args) {
        if (args.Length == 1 && args[0] == "--version")
            Console.WriteLine("codex-cli " + File.ReadAllText(Path.Combine(Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location), "version.txt")));
        return 0;
    }
}
'@ | Set-Content $sourceCode
    & "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319/csc.exe" /nologo /target:exe "/out:$stub" $sourceCode
    if ($LASTEXITCODE -ne 0) { throw "Could not compile test executable." }
    $requests = [Collections.Generic.List[string]]::new()
    $env:CODEX_HOME = Join-Path $root "codex home"
    $env:CODEX_INSTALL_DIR = Join-Path $root "visible bin"
    $env:CODEX_INSTALL_DAEMON_ONLY = "1"
    $env:CODEX_NON_INTERACTIVE = "1"
    $archive = New-TestRelease "0.166.0-doca"
    . $installer -Release latest
    $managed = Join-Path $env:CODEX_HOME "packages/app-server-daemon"
    $current = Join-Path $managed "current"
    $old = Join-Path $managed "releases/0.166.0-doca-x86_64-pc-windows-msvc"
    Assert-Equal ([CodexInstaller.JunctionV3]::ResolveDirectory($current)) ([CodexInstaller.JunctionV3]::ResolveDirectory($old))
    Assert-Equal $requests[0] "https://api.github.com/repos/DOCaCola/codex/releases/latest"
    # Expose the same junction chain used by the standalone CLI.
    Ensure-Junction -LinkPath $env:CODEX_INSTALL_DIR -TargetPath (Join-Path $current "bin") -InstallerOwnedTargetPrefix $managed
    Assert-Equal (& "$env:CODEX_INSTALL_DIR/codex.exe" --version) "codex-cli 0.166.0-doca"
    $heldExe = [IO.File]::Open((Join-Path $old "bin/codex.exe"), "Open", "Read", "Read")
    try {
        $archive = New-TestRelease "0.166.0-doca.1"
        . $installer -Release latest
        Assert-Equal (& "$env:CODEX_INSTALL_DIR/codex.exe" --version) "codex-cli 0.166.0-doca.1"
        Assert-Equal (Test-Path (Join-Path $old "bin/codex.exe")) $true
    } finally { $heldExe.Dispose() }
    $selected = [CodexInstaller.JunctionV3]::ResolveDirectory($current)
    $archive = New-TestRelease "0.166.0-doca.2"
    [IO.File]::WriteAllText($archive, "corrupt archive")
    try {
        . $installer -Release latest
        throw "Corrupt archive was accepted."
    } catch {
        if ($_.Exception.Message -notmatch "checksum") { throw }
    }
    Assert-Equal ([CodexInstaller.JunctionV3]::ResolveDirectory($current)) $selected
    $archive = New-TestRelease "0.166.0-doca.3"
    $env:CODEX_INSTALL_IF_LATEST = "1"
    $env:CODEX_UPDATE_FROM_RELEASE = Split-Path $selected -Leaf
    Remove-Item (Join-Path $managed "auto-update-version")
    . $installer -Release latest
    Assert-Equal ([CodexInstaller.JunctionV3]::ResolveDirectory($current)) $selected
    $env:CODEX_INSTALL_IF_LATEST = "0"
    $env:CODEX_INSTALL_IF_CURRENT = "1"
    . $installer -Release latest
    Assert-Equal (& "$env:CODEX_INSTALL_DIR/codex.exe" --version) "codex-cli 0.166.0-doca.3"
    Write-Host "PASS: fresh install, locked executable upgrade, checksum failure, pin preservation, explicit daemon update."
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name])
    }
    # Remove junctions separately so cleanup never follows them.
    foreach ($link in @((Join-Path $root "visible bin"), (Join-Path $root "codex home/packages/app-server-daemon/current"))) {
        if (Test-Path $link) { [IO.Directory]::Delete($link) }
    }
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
