#Requires -Version 5.1
[CmdletBinding()]
param([switch]$SkipBuild, [string]$OutputDir = '', [string]$MakeNsis = '', [string]$RuntimePath = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if (!$OutputDir) { $OutputDir = Join-Path $repo 'target\windows-installer' }
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$payload = Join-Path $OutputDir 'payload'
New-Item -ItemType Directory -Path $OutputDir, $payload -Force | Out-Null
Push-Location $repo
try {
    $sourceCommit = (& git rev-parse HEAD).Trim()
    if (& git status --porcelain --untracked-files=no) { throw 'Commit source changes before creating a distributable installer.' }
    $sourceRepository = (& git remote get-url origin).Trim() -replace '\.git$', ''
    if ($sourceRepository -match '^git@github\.com:(.+)$') { $sourceRepository = 'https://github.com/' + $Matches[1] }
    if ($sourceRepository -notmatch '^https://github\.com/[^/]+/[^/]+$') { throw 'The origin must identify an accessible GitHub source repository.' }
    if (!$SkipBuild) {
        & cargo build --locked --release --bin hbbs --bin hbbr --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw 'Server build failed.' }
    }
    foreach ($name in @('hbbs', 'hbbr')) {
        Copy-Item -LiteralPath (Join-Path $repo "target\x86_64-pc-windows-msvc\release\$name.exe") -Destination $payload -Force
    }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Install-Services.ps1') -Destination $payload -Force
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $payload -Force
    Copy-Item -LiteralPath (Join-Path $repo 'docs\windows-native-services.md') -Destination (Join-Path $payload 'README.txt') -Force
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    if (!$RuntimePath) {
        # Refresh on every build rather than keeping an old checked-in package.
        $RuntimePath = Join-Path $payload 'vc_redist.x64.exe'
        Invoke-WebRequest -UseBasicParsing -Uri 'https://aka.ms/vc14/vc_redist.x64.exe' -OutFile $RuntimePath
    } else {
        Copy-Item -LiteralPath $RuntimePath -Destination (Join-Path $payload 'vc_redist.x64.exe') -Force
        $RuntimePath = Join-Path $payload 'vc_redist.x64.exe'
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $RuntimePath
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw 'Refusing to package a runtime without a valid Microsoft signature.'
    }
    $version = (Get-Item -LiteralPath $RuntimePath).VersionInfo.ProductVersion
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $dumpbin = Get-ChildItem -LiteralPath (Join-Path $vs 'VC\Tools\MSVC') -Directory |
        Sort-Object Name -Descending | ForEach-Object { Join-Path $_.FullName 'bin\Hostx64\x64\dumpbin.exe' } |
        Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (!$dumpbin) { throw 'MSVC dumpbin is required to verify runtime dependencies.' }
    $toolVersion = [version](Get-Item -LiteralPath $dumpbin).Directory.Parent.Parent.Parent.Name
    if ($version -notmatch '\d+\.\d+\.\d+(\.\d+)?' -or [version]$Matches[0] -lt $toolVersion) {
        throw "The bundled runtime must be at least as recent as MSVC $toolVersion."
    }
    $imports = @{}
    foreach ($name in @('hbbs', 'hbbr')) {
        $output = & $dumpbin /dependents (Join-Path $payload "$name.exe")
        if ($LASTEXITCODE -ne 0) { throw "Cannot inspect imports for $name." }
        $dlls = @($output | ForEach-Object {
            if ($_ -match '^\s+([a-zA-Z0-9_.-]+\.dll)\s*$') { $Matches[1] }
        } | Sort-Object -Unique)
        if ($dlls.Count -eq 0) { throw "No PE imports found for $name." }
        foreach ($dll in $dlls) {
            if ($dll -match '^(?i:VCRUNTIME140|MSVCP140|CONCRT140|VCOMP140).*\.dll$') { continue }
            if ($dll -match '^(?i:api-ms-|ext-ms-)') { continue }
            if (!(Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32\$dll"))) {
                throw "Unpackaged DLL required by ${name}: $dll"
            }
        }
        if (!($dlls -match '^(?i:VCRUNTIME140|MSVCP140).*\.dll$')) {
            throw "$name must link the shared v14 runtime so installed runtime security updates apply."
        }
        $imports[$name] = $dlls
    }
    [ordered]@{
        BuiltAt=(Get-Date).ToUniversalTime().ToString('o'); RuntimeVersion=$version
        SourceCommit=$sourceCommit; SourceURL=($sourceRepository + '/tree/' + $sourceCommit)
        RuntimeSource='https://aka.ms/vc14/vc_redist.x64.exe'
        RuntimeSHA256=(Get-FileHash -LiteralPath $RuntimePath -Algorithm SHA256).Hash
        Imports=$imports; TargetServers=@('2016','2019','2022','2025')
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $payload 'build-info.json') -Encoding UTF8
    if (!$MakeNsis) {
        $found = Get-Command makensis.exe -ErrorAction SilentlyContinue
        if ($found) { $MakeNsis = $found.Source }
        foreach ($candidate in @("${env:ProgramFiles(x86)}\NSIS\makensis.exe", "$env:ProgramFiles\NSIS\makensis.exe")) {
            if (!$MakeNsis -and (Test-Path -LiteralPath $candidate)) { $MakeNsis = $candidate }
        }
    }
    if (!$MakeNsis) { throw 'Install NSIS 3, or supply -MakeNsis with its compiler path.' }
    $installer = Join-Path $OutputDir 'RustDeskServer-Native-x64-Setup.exe'
    & $MakeNsis /V2 "/DPAYLOAD=$payload" "/DOUTPUT=$installer" (Join-Path $repo 'ui\setup.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'NSIS compilation failed.' }
    Get-FileHash -LiteralPath $installer -Algorithm SHA256 | Format-List
    Write-Output "Installer: $installer"
} finally { Pop-Location }
