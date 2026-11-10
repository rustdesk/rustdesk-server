#Requires -Version 5.1
#Requires -RunAsAdministrator
[CmdletBinding()]
param([Parameter(Mandatory)][string]$InstallerPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$installDir = Join-Path $env:ProgramFiles 'RustDeskServer-Native-Test'
$dataDir = Join-Path $env:ProgramData 'RustDeskServer'
foreach ($name in @('hbbs', 'hbbr')) {
    if (Get-Service -Name $name -ErrorAction SilentlyContinue) { throw "Existing service ${name}: use a clean test VM." }
}
if ((Test-Path -LiteralPath $installDir) -or (Test-Path -LiteralPath $dataDir)) { throw 'Existing installation/data: use a clean test VM.' }

function Assert-True { param([bool]$Condition, [string]$Message) if (!$Condition) { throw $Message } }
function Run-Setup {
    $process = Start-Process -FilePath $InstallerPath -ArgumentList "/S /SCOPE=LAN /D=$installDir" -Wait -PassThru -WindowStyle Hidden
    Assert-True ($process.ExitCode -in @(0, 3010)) "Setup failed with exit code $($process.ExitCode)."
}
function Assert-Running {
    foreach ($name in @('hbbs', 'hbbr')) {
        $service = Get-Service -Name $name
        $service.WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
        $details = Get-CimInstance Win32_Service -Filter "Name='$name'"
        Assert-True ($details.StartMode -eq 'Auto') "$name must start automatically."
        Assert-True ($details.StartName -eq 'NT AUTHORITY\LocalService') "$name must use LocalService."
        Assert-True ($details.PathName -notmatch 'nssm') "$name must run natively."
        $connection = New-Object Net.Sockets.TcpClient
        try {
            $port = if ($name -eq 'hbbs') { 21116 } else { 21117 }
            $connection.Connect('127.0.0.1', $port)
        } finally { $connection.Dispose() }
    }
}
Run-Setup
Assert-Running
$access = (Get-Acl -LiteralPath $dataDir).GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])
Assert-True (@($access | Where-Object { $_.IdentityReference.Value -eq 'S-1-5-19' }).Count -eq 0) 'Generic LocalService must not have data access.'
foreach ($name in @('hbbs', 'hbbr')) {
    $sid = (New-Object Security.Principal.NTAccount('NT SERVICE', $name)).Translate([Security.Principal.SecurityIdentifier]).Value
    Assert-True (@($access | Where-Object { $_.IdentityReference.Value -eq $sid }).Count -gt 0) "Missing service SID access for $name."
}
$keyPath = Join-Path $dataDir 'id_ed25519'
$keyHash = (Get-FileHash -LiteralPath $keyPath -Algorithm SHA256).Hash
$rules = @(Get-NetFirewallRule -Name 'RustDeskServer-Native-*')
Assert-True ($rules.Count -eq 3) 'Expected three program-bound firewall rules.'
foreach ($rule in $rules) {
    $ports = @($rule | Get-NetFirewallPortFilter | Select-Object -ExpandProperty LocalPort)
    Assert-True (!($ports -match '21118|21119')) 'Browser ports must remain closed.'
    $remote = @($rule | Get-NetFirewallAddressFilter | Select-Object -ExpandProperty RemoteAddress)
    Assert-True ($remote -contains 'LocalSubnet') 'LAN scope must restrict remote addresses.'
}
foreach ($name in @('hbbr', 'hbbs')) {
    Stop-Service -Name $name
    (Get-Service -Name $name).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
}
foreach ($name in @('hbbs', 'hbbr')) { Start-Service -Name $name }
Assert-Running
# A crash must trigger recovery; a deliberate Stop must not.
$before = (Get-CimInstance Win32_Service -Filter "Name='hbbr'").ProcessId
Stop-Process -Id $before -Force
$deadline = (Get-Date).AddSeconds(30)
do {
    Start-Sleep -Milliseconds 500
    $relay = Get-CimInstance Win32_Service -Filter "Name='hbbr'"
} until (($relay.State -eq 'Running' -and $relay.ProcessId -ne $before) -or (Get-Date) -ge $deadline)
Assert-True ($relay.State -eq 'Running' -and $relay.ProcessId -ne $before) 'Relay did not recover after a crash.'
Assert-Running
Run-Setup
Assert-Running
Assert-True ((Get-FileHash -LiteralPath $keyPath -Algorithm SHA256).Hash -eq $keyHash) 'Upgrade changed the private key.'
$uninstaller = Join-Path $installDir 'Uninstall.exe'
# _?= runs the uninstaller in place so -Wait observes the actual uninstall.
$process = Start-Process -FilePath $uninstaller -ArgumentList "/S _?=$installDir" -Wait -PassThru -WindowStyle Hidden
Assert-True ($process.ExitCode -eq 0) "Uninstall failed: $($process.ExitCode)."
foreach ($name in @('hbbs', 'hbbr')) {
    Assert-True ($null -eq (Get-Service -Name $name -ErrorAction SilentlyContinue)) "Service $name remains after uninstall."
}
Assert-True ((Get-FileHash -LiteralPath $keyPath -Algorithm SHA256).Hash -eq $keyHash) 'Uninstall did not preserve server identity.'
Assert-True (@(Get-NetFirewallRule -Name 'RustDeskServer-Native-*' -ErrorAction SilentlyContinue).Count -eq 0) 'Firewall rules remain after uninstall.'
Write-Output "Native installation lifecycle passed on $((Get-CimInstance Win32_OperatingSystem).Caption)."
Write-Output "Retained test data: $dataDir. Reboot-before-login and real remote sessions require separate validation."
