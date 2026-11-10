#Requires -Version 5.1
#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [ValidateSet('Install', 'Uninstall')][string]$Mode = 'Install',
    [Parameter(Mandatory)][string]$InstallDir,
    [string]$PayloadDir = $PSScriptRoot,
    [string]$RelayServer = '',
    [ValidateSet('LAN', 'Internet', 'Preserve')][string]$NetworkScope = 'Preserve'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$dataDir = Join-Path $env:ProgramData 'RustDeskServer'
$serviceNames = @('hbbs', 'hbbr')
$rulePrefix = 'RustDeskServer-Native-'
$manifestPath = Join-Path $InstallDir 'native-install.json'
$logPath = Join-Path $env:TEMP ('RustDeskServer-Setup-' + [guid]::NewGuid().ToString('N') + '.log')
Start-Transcript -LiteralPath $logPath | Out-Null

function ConvertTo-NativeArguments {
    param([string[]]$Arguments)
    ($Arguments | ForEach-Object {
        '"' + (($_ -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
    }) -join ' '
}

function Invoke-Sc {
    param([string[]]$Arguments)
    # Quote native argv explicitly: Windows PowerShell 5.1 otherwise strips the
    # quotes inside ImagePath, breaking paths containing spaces.
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = "$env:SystemRoot\System32\sc.exe"
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Arguments = ConvertTo-NativeArguments -Arguments $Arguments
    $process = [Diagnostics.Process]::Start($start)
    $result = $process.StandardOutput.ReadToEnd() + $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $code = $process.ExitCode
    $process.Dispose()
    if ($code -ne 0) { throw "Service operation failed: $($Arguments -join ' '): $result" }
}

function Stop-ServerServices {
    foreach ($name in @('hbbr', 'hbbs')) {
        $service = Get-Service -Name $name -ErrorAction SilentlyContinue
        if ($null -ne $service -and $service.Status -ne 'Stopped') {
            Stop-Service -Name $name -ErrorAction Stop
            $service.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        }
    }
}

function Remove-ServerServices {
    Stop-ServerServices
    foreach ($name in $serviceNames) {
        if (Get-Service -Name $name -ErrorAction SilentlyContinue) {
            Invoke-Sc -Arguments @('delete', $name)
        }
    }
}

function Set-ServerFirewall {
    param([string]$Scope, [switch]$PreserveExisting)
    # Persist the selected scope; upgrades preserve it unless explicitly changed.
    $profiles = @('Domain', 'Private')
    $remote = 'LocalSubnet'
    if ($Scope -eq 'Internet') { $profiles = @('Any'); $remote = 'Any' }
    $rules = @(
        @{ Suffix='ID-TCP'; Binary='hbbs.exe'; Protocol='TCP'; Ports='21115-21116' },
        @{ Suffix='ID-UDP'; Binary='hbbs.exe'; Protocol='UDP'; Ports='21116' },
        @{ Suffix='Relay-TCP'; Binary='hbbr.exe'; Protocol='TCP'; Ports='21117' }
    )
    foreach ($rule in $rules) {
        $name = $rulePrefix + $rule.Suffix
        if ($PreserveExisting -and (Get-NetFirewallRule -Name $name -ErrorAction SilentlyContinue)) { continue }
        $properties = @{
            Direction='Inbound'; Action='Allow'; Enabled='True'; Profile=$profiles
            Program=(Join-Path $InstallDir $rule.Binary); Protocol=$rule.Protocol
            LocalPort=$rule.Ports; RemoteAddress=$remote
        }
        if (Get-NetFirewallRule -Name $name -ErrorAction SilentlyContinue) {
            Set-NetFirewallRule -Name $name @properties | Out-Null
        } else {
            New-NetFirewallRule -Name $name -DisplayName ("RustDesk Server " + $rule.Suffix) `
                -Group 'RustDesk Server (Native)' @properties | Out-Null
        }
    }
}

function Get-ServerFirewallSnapshot {
    foreach ($suffix in @('ID-TCP', 'ID-UDP', 'Relay-TCP')) {
        $name = $rulePrefix + $suffix
        $rule = Get-NetFirewallRule -Name $name -ErrorAction SilentlyContinue
        if ($null -eq $rule) { [pscustomobject]@{ Name=$name; Properties=$null }; continue }
        $application = $rule | Get-NetFirewallApplicationFilter
        $port = $rule | Get-NetFirewallPortFilter
        $address = $rule | Get-NetFirewallAddressFilter
        [pscustomobject]@{
            Name=$name
            Properties=@{
                Direction=$rule.Direction; Action=$rule.Action; Enabled=$rule.Enabled; Profile=$rule.Profile
                Program=$application.Program; Protocol=$port.Protocol; LocalPort=$port.LocalPort
                RemoteAddress=$address.RemoteAddress
            }
        }
    }
}

function Restore-ServerFirewall {
    param([object[]]$Snapshot)
    foreach ($entry in $Snapshot) {
        if ($null -eq $entry.Properties) {
            Get-NetFirewallRule -Name $entry.Name -ErrorAction SilentlyContinue | Remove-NetFirewallRule
        } else {
            $properties = $entry.Properties
            Set-NetFirewallRule -Name $entry.Name @properties | Out-Null
        }
    }
}

function Install-Runtime {
    $runtime = Join-Path $PayloadDir 'vc_redist.x64.exe'
    if (!(Test-Path -LiteralPath $runtime)) { throw 'The bundled Microsoft Visual C++ runtime is missing.' }
    $signature = Get-AuthenticodeSignature -LiteralPath $runtime
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw 'The runtime does not have a valid Microsoft signature.'
    }
    $versionText = (Get-Item -LiteralPath $runtime).VersionInfo.ProductVersion
    if ($versionText -notmatch '\d+\.\d+\.\d+(\.\d+)?') { throw 'Cannot read bundled runtime version.' }
    $required = [version]$Matches[0]
    $installed = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64' -ErrorAction SilentlyContinue
    if ($null -ne $installed -and $installed.Installed -eq 1 -and
        [version]($installed.Version -replace '^v', '') -ge $required) {
        Write-Output "Visual C++ runtime $($installed.Version) already satisfies $required."
        return
    }
    $runtimeLog = Join-Path $env:TEMP 'RustDeskServer-VisualCpp.log'
    $process = Start-Process -FilePath $runtime -ArgumentList "/install /quiet /norestart /log `"$runtimeLog`"" `
        -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -eq 3010) { $script:rebootRequired = $true }
    elseif ($process.ExitCode -ne 0) { throw "Visual C++ runtime installation failed ($($process.ExitCode)). See $runtimeLog" }
}

$createdServices = @()
$previouslyRunning = @()
$previousManifest = $null
$backupDir = $null
$configBackup = $null
$configExisted = $false
$installStarted = $false
$retainBackup = $false
$rebootRequired = $false
$firewallChanged = $false
$firewallSnapshot = @()
$dataAcl = $null
try {
    if (![Environment]::Is64BitProcess) { throw 'Run setup using 64-bit PowerShell.' }
    $os = Get-CimInstance Win32_OperatingSystem
    if ([version]$os.Version -lt [version]'10.0.14393') {
        throw 'Windows Server 2016 or later (or Windows 10 version 1607 or later) is required.'
    }
    $InstallDir = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
    if ($InstallDir -eq [IO.Path]::GetPathRoot($InstallDir).TrimEnd('\')) { throw 'The installation directory cannot be a drive root.' }
    # Never replace an unrelated or legacy NSSM service automatically.
    if (Test-Path -LiteralPath $manifestPath) {
        $previousManifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    }
    foreach ($name in $serviceNames) {
        $existing = Get-CimInstance Win32_Service -Filter "Name='$name'"
        if ($null -ne $existing) {
            $expected = '"' + (Join-Path $InstallDir "$name.exe") + '" --service --data-dir "' + $dataDir + '"'
            if ($null -eq $previousManifest -or $existing.PathName -ne $expected) {
                throw "Service '$name' belongs to a different or legacy installation. Back up its data and remove that installation first. No services were changed."
            }
            if ($existing.State -eq 'Running') { $previouslyRunning += $name }
        }
    }
    if ($Mode -eq 'Uninstall') {
        if ($null -eq $previousManifest) { throw 'Native installation manifest not found; refusing to remove unknown services.' }
        Remove-ServerServices
        Get-NetFirewallRule -Name "$rulePrefix*" -ErrorAction SilentlyContinue | Remove-NetFirewallRule
        # The runtime may be shared with other applications; retain it and all server data.
        Write-Output "Services and firewall rules removed. Server data retained at $dataDir."
        exit 0
    }
    if ($RelayServer -and $RelayServer -notmatch '^[a-zA-Z0-9.\-\[\]:]+$') {
        throw 'The relay address must be a hostname or IP address, optionally followed by a port.'
    }
    $selectedScope = $NetworkScope
    if ($selectedScope -eq 'Preserve') {
        $selectedScope = if ($null -ne $previousManifest) { $previousManifest.NetworkScope } else { 'LAN' }
    }
    foreach ($binary in @('hbbs.exe', 'hbbr.exe')) {
        if (!(Test-Path -LiteralPath (Join-Path $PayloadDir $binary))) { throw "Missing payload: $binary" }
    }
    Install-Runtime
    $firewallSnapshot = @(Get-ServerFirewallSnapshot)
    if (Test-Path -LiteralPath $dataDir) { $dataAcl = Get-Acl -LiteralPath $dataDir }
    New-Item -ItemType Directory -Path $InstallDir, $dataDir, (Join-Path $dataDir 'logs\hbbs'), (Join-Path $dataDir 'logs\hbbr') -Force | Out-Null
    $backupDir = Join-Path $InstallDir ('rollback-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $backupDir | Out-Null
    foreach ($binary in @('hbbs.exe', 'hbbr.exe')) {
        $path = Join-Path $InstallDir $binary
        if (Test-Path -LiteralPath $path) { Copy-Item -LiteralPath $path -Destination $backupDir }
    }
    $configPath = Join-Path $dataDir '.env'
    $configExisted = Test-Path -LiteralPath $configPath
    if ($configExisted) { $configBackup = [IO.File]::ReadAllBytes($configPath) }
    $installStarted = $true
    Stop-ServerServices
    foreach ($binary in @('hbbs.exe', 'hbbr.exe')) {
        Copy-Item -LiteralPath (Join-Path $PayloadDir $binary) -Destination (Join-Path $InstallDir $binary) -Force
    }
    if (!$configExisted) {
        @('# RustDesk Server native service configuration', '# Both services share this file.', 'KEY=-', 'DB_URL=db_v2.sqlite3') |
            Set-Content -LiteralPath $configPath -Encoding ASCII
    }
    if ($RelayServer) {
        $lines = @(Get-Content -LiteralPath $configPath | Where-Object { $_ -notmatch '^\s*RELAY_SERVERS\s*=' })
        $lines += "RELAY_SERVERS=$RelayServer"
        [IO.File]::WriteAllLines($configPath, $lines, (New-Object Text.UTF8Encoding($false)))
    }
    foreach ($name in $serviceNames) {
        if (!(Get-Service -Name $name -ErrorAction SilentlyContinue)) {
            $display = if ($name -eq 'hbbs') { 'RustDesk ID Server' } else { 'RustDesk Relay Server' }
            $imagePath = '"' + (Join-Path $InstallDir "$name.exe") + '" --service --data-dir "' + $dataDir + '"'
            # LocalService has no administrative privileges or interactive-login dependency.
            Invoke-Sc -Arguments @('create', $name, 'binPath=', $imagePath, 'start=', 'auto', 'obj=', 'NT AUTHORITY\LocalService', 'DisplayName=', $display)
            $createdServices += $name
        }
        Invoke-Sc -Arguments @('description', $name, "RustDesk native $name service")
        Invoke-Sc -Arguments @('failure', $name, 'reset=', '86400', 'actions=', 'restart/5000/restart/15000/restart/60000')
        Invoke-Sc -Arguments @('failureflag', $name, '1')
        Invoke-Sc -Arguments @('sidtype', $name, 'unrestricted')
    }
    Invoke-Sc -Arguments @('config', 'hbbr', 'depend=', 'hbbs')
    # Authorize only these service SIDs, not every process using LocalService.
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @('S-1-5-18', 'S-1-5-32-544')) {
        $identity = New-Object Security.Principal.SecurityIdentifier($sid)
        $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($identity, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
    }
    foreach ($name in $serviceNames) {
        $identity = (New-Object Security.Principal.NTAccount('NT SERVICE', $name)).Translate([Security.Principal.SecurityIdentifier])
        $rights = if ($name -eq 'hbbs') { 'Modify' } else { 'ReadAndExecute' }
        $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($identity, $rights, 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
    }
    Set-Acl -LiteralPath $dataDir -AclObject $acl
    $relayLogs = Join-Path $dataDir 'logs\hbbr'
    $logAcl = Get-Acl -LiteralPath $relayLogs
    $relayIdentity = New-Object Security.Principal.NTAccount('NT SERVICE', 'hbbr')
    $logAcl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($relayIdentity, 'Modify', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
    Set-Acl -LiteralPath $relayLogs -AclObject $logAcl
    $firewallChanged = $true
    Set-ServerFirewall -Scope $selectedScope -PreserveExisting:($NetworkScope -eq 'Preserve')
    # Start ID first so it creates key material before the relay reads it.
    foreach ($name in $serviceNames) {
        Start-Service -Name $name
        (Get-Service -Name $name).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    }
    if (!(Test-Path -LiteralPath (Join-Path $dataDir 'id_ed25519.pub'))) { throw 'The server did not publish its public key.' }
    $manifest = [ordered]@{ Version='1.1.17'; NetworkScope=$selectedScope; DataDir=$dataDir; InstalledAt=(Get-Date).ToUniversalTime().ToString('o') }
    $manifest | ConvertTo-Json | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $InstallDir 'Install-Services.ps1') -Force
    $publicKey = (Get-Content -LiteralPath (Join-Path $dataDir 'id_ed25519.pub') -Raw).Trim()
    $buildInfo = Get-Content -LiteralPath (Join-Path $PayloadDir 'build-info.json') -Raw | ConvertFrom-Json
    $summary = @(
        'RustDesk Server is installed as two native Windows services.',
        "ID server: $(if ($RelayServer) { $RelayServer -replace ':21117$', '' } else { $env:COMPUTERNAME })",
        "Public key: $publicKey", "Data and configuration: $dataDir", "Firewall scope: $selectedScope",
        "Corresponding source (including build/install scripts): $($buildInfo.SourceURL)",
        'Windows Firewall: TCP 21115-21117, UDP 21116; browser ports remain closed.',
        'Internet access also requires router/cloud firewall configuration.',
        'For VPNs on a different subnet, adjust the firewall remote-address scope.',
        'Keep the Microsoft Visual C++ v14 x64 runtime patched through your server update process.'
    )
    $summary | Set-Content -LiteralPath (Join-Path $InstallDir 'Connection-Details.txt') -Encoding UTF8
    Write-Output ($summary -join [Environment]::NewLine)
    if ($rebootRequired) { Write-Output 'Microsoft runtime setup requests a reboot. Reboot at your next maintenance window.' }
    exit $(if ($rebootRequired) { 3010 } else { 0 })
} catch {
    $failure = $_
    if ($installStarted) {
        Write-Warning 'Installation failed. Attempting to restore the previous installation.'
        try {
            Stop-ServerServices
            foreach ($name in $createdServices) { Invoke-Sc -Arguments @('delete', $name) }
            foreach ($binary in @('hbbs.exe', 'hbbr.exe')) {
                $backup = Join-Path $backupDir $binary
                if (Test-Path -LiteralPath $backup) { Copy-Item -LiteralPath $backup -Destination (Join-Path $InstallDir $binary) -Force }
            }
            if ($configExisted) { [IO.File]::WriteAllBytes((Join-Path $dataDir '.env'), $configBackup) }
            if ($dataAcl) { Set-Acl -LiteralPath $dataDir -AclObject $dataAcl }
            if ($firewallChanged) { Restore-ServerFirewall -Snapshot $firewallSnapshot }
            foreach ($name in $previouslyRunning) { Start-Service -Name $name }
        } catch {
            $retainBackup = $true
            Write-Warning "Rollback needs attention: $_. Backup: $backupDir"
        }
    }
    Write-Output "SETUP FAILED: $failure"
    Write-Output "Installation log: $logPath"
    exit 1
} finally {
    # Only remove the exact generated rollback directory under this installation.
    if ($backupDir -and (Test-Path -LiteralPath $backupDir) -and
        [IO.Path]::GetDirectoryName($backupDir) -eq $InstallDir -and
        [IO.Path]::GetFileName($backupDir) -match '^rollback-[0-9a-f]{32}$') {
        if (!$retainBackup) { Remove-Item -LiteralPath $backupDir -Recurse -Force }
    }
    Stop-Transcript | Out-Null
}
