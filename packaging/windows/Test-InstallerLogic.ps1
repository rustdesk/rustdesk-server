#Requires -Version 5.1
# Tests the runtime decisions without requiring elevation or installing anything.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$tokens = $null
$errors = $null
$installer = Join-Path $PSScriptRoot 'Install-Services.ps1'
$ast = [Management.Automation.Language.Parser]::ParseFile($installer, [ref]$tokens, [ref]$errors)
if ($errors) { throw ($errors.Message -join '; ') }
foreach ($definition in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
# Exercise the actual Windows argument parser, especially quoted ImagePath and
# empty/trailing-backslash arguments, under Windows PowerShell 5.1.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class RustDeskArgumentParser {
    [DllImport("shell32.dll", SetLastError=true)]
    private static extern IntPtr CommandLineToArgvW([MarshalAs(UnmanagedType.LPWStr)] string command, out int count);
    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr memory);
    public static string[] Parse(string command) {
        int count;
        IntPtr memory = CommandLineToArgvW(command, out count);
        if (memory == IntPtr.Zero) throw new InvalidOperationException("CommandLineToArgvW failed");
        try {
            string[] result = new string[count];
            for (int i = 0; i < count; i++) result[i] = Marshal.PtrToStringUni(Marshal.ReadIntPtr(memory, i * IntPtr.Size));
            return result;
        } finally { LocalFree(memory); }
    }
}
'@
$expectedArguments = @('create', 'hbbs', 'binPath=', '"C:\Program Files\RustDeskServer\hbbs.exe" --service --data-dir "C:\ProgramData\RustDeskServer"', 'trailing\', '', 'NT AUTHORITY\LocalService')
$commandLine = ConvertTo-NativeArguments -Arguments $expectedArguments
$parsed = [RustDeskArgumentParser]::Parse('sc.exe ' + $commandLine)
if ($parsed.Count -ne $expectedArguments.Count + 1) { throw 'Native argument count changed.' }
for ($i = 0; $i -lt $expectedArguments.Count; $i++) {
    if ($parsed[$i + 1] -cne $expectedArguments[$i]) { throw "Native argument $i did not round-trip." }
}
Write-Output 'PASS: native service ImagePath quoting, spaces, empty values, and trailing backslashes round-trip.'
$PayloadDir = $PSScriptRoot
$signatureValid = $true
$installedVersion = 'v99.0.0.0'
$processCalls = 0
$processResult = 0
$rebootRequired = $false
function Test-Path { param($LiteralPath) return $true }
function Get-AuthenticodeSignature {
    param($LiteralPath)
    [pscustomobject]@{ Status=$(if ($signatureValid) { 'Valid' } else { 'NotSigned' }); SignerCertificate=[pscustomobject]@{ Subject='CN=Microsoft Corporation, O=Microsoft Corporation, C=US' } }
}
function Get-Item { param($LiteralPath) [pscustomobject]@{ VersionInfo=[pscustomobject]@{ ProductVersion='14.51.36247.0' } } }
function Get-ItemProperty { param($Path, $ErrorAction) [pscustomobject]@{ Installed=1; Version=$installedVersion } }
function Start-Process {
    param($FilePath, $ArgumentList, [switch]$Wait, [switch]$PassThru, $WindowStyle)
    if ($ArgumentList -notmatch '/quiet /norestart') { throw 'Runtime must install quietly without automatically rebooting.' }
    $script:processCalls++
    [pscustomobject]@{ ExitCode=$processResult }
}
Install-Runtime | Out-Null
if ($processCalls -ne 0) { throw 'Setup attempted to downgrade a newer runtime.' }
$installedVersion = 'v14.0.0.0'
Install-Runtime | Out-Null
if ($processCalls -ne 1) { throw 'Setup did not update an older runtime.' }
$processResult = 3010
Install-Runtime | Out-Null
if (!$rebootRequired) { throw 'Setup lost the runtime reboot-required result.' }
$processResult = 1603
$failed = $false
try { Install-Runtime | Out-Null } catch { $failed = $true }
if (!$failed) { throw 'Setup ignored a runtime installation failure.' }
$signatureValid = $false
$callsBefore = $processCalls
$failed = $false
try { Install-Runtime | Out-Null } catch { $failed = $true }
if (!$failed -or $processCalls -ne $callsBefore) { throw 'Setup accepted a runtime without a Microsoft signature.' }
Write-Output 'PASS: newer runtime retained; older runtime updated; reboot/failure results handled; unsigned runtime rejected.'

# Snapshot/restore only the properties setup changes, without replacing rules.
$rulePrefix = 'RustDeskServer-Native-'
$InstallDir = 'C:\Program Files\RustDeskServer'
$fixture = @{}
foreach ($suffix in @('ID-TCP', 'ID-UDP')) {
    $name = $rulePrefix + $suffix
    $fixture[$name] = [pscustomobject]@{
        Name=$name; Direction='Inbound'; Action='Allow'; Enabled='True'; Profile='Domain'
        Program='C:\Original\hbbs.exe'; Protocol=$(if ($suffix -eq 'ID-UDP') { 'UDP' } else { 'TCP' })
        LocalPort='21116'; RemoteAddress=@('10.40.0.0/16'); EdgeTraversalPolicy='Block'
    }
}
function Get-NetFirewallRule { param($Name, $ErrorAction) if ($fixture.ContainsKey($Name)) { $fixture[$Name] } }
function Get-NetFirewallApplicationFilter { param([Parameter(ValueFromPipeline)]$InputObject) process { [pscustomobject]@{ Program=$InputObject.Program } } }
function Get-NetFirewallPortFilter { param([Parameter(ValueFromPipeline)]$InputObject) process { [pscustomobject]@{ Protocol=$InputObject.Protocol; LocalPort=$InputObject.LocalPort } } }
function Get-NetFirewallAddressFilter { param([Parameter(ValueFromPipeline)]$InputObject) process { [pscustomobject]@{ RemoteAddress=$InputObject.RemoteAddress } } }
function Set-NetFirewallRule {
    param($Name, $Direction, $Action, $Enabled, $Profile, $Program, $Protocol, $LocalPort, $RemoteAddress)
    foreach ($key in @('Direction','Action','Enabled','Profile','Program','Protocol','LocalPort','RemoteAddress')) {
        $fixture[$Name].$key = $PSBoundParameters[$key]
    }
}
function New-NetFirewallRule {
    param($Name, $DisplayName, $Group, $Direction, $Action, $Enabled, $Profile, $Program, $Protocol, $LocalPort, $RemoteAddress)
    $fixture[$Name] = [pscustomobject]@{ Name=$Name; Direction=$Direction; Action=$Action; Enabled=$Enabled; Profile=$Profile; Program=$Program; Protocol=$Protocol; LocalPort=$LocalPort; RemoteAddress=$RemoteAddress }
}
function Remove-NetFirewallRule { param([Parameter(ValueFromPipeline)]$InputObject) process { $fixture.Remove($InputObject.Name) } }
$snapshot = @(Get-ServerFirewallSnapshot)
Set-ServerFirewall -Scope Internet
Restore-ServerFirewall -Snapshot $snapshot
if ($fixture.ContainsKey($rulePrefix + 'Relay-TCP')) { throw 'Rollback did not remove a newly created rule.' }
foreach ($rule in $fixture.Values) {
    if (($rule.RemoteAddress -join ',') -ne '10.40.0.0/16' -or $rule.Profile -ne 'Domain' -or $rule.Program -ne 'C:\Original\hbbs.exe') { throw 'Rollback lost the original firewall restrictions.' }
    if ($rule.EdgeTraversalPolicy -ne 'Block') { throw 'An unrelated firewall property was changed.' }
}
Write-Output 'PASS: rollback restores custom firewall scope/program and retains unrelated restrictions.'
