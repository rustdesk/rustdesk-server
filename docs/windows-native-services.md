# Native Windows services

The x64 installer targets Windows Server 2016 (build 14393), 2019 (17763),
2022 (20348), and 2025 (26100), including Server Core. These are target platforms,
not a claim that each release has been tested. Run the validation below on each
server version before production use. The native service API also works on
Windows 10 version 1607 and later.

## Install

Run `RustDeskServer-Native-x64-Setup.exe` as administrator. The installer installs
Microsoft's current supported Visual C++ v14 x64 Redistributable when the bundled
version is newer than the installed version. It does not downgrade newer runtimes.
Each installer build downloads the latest official package and validates its
Microsoft signature. The installed programs link the shared runtime, so runtime
patches can apply without recompiling the server. Keep that shared runtime patched
through your normal Windows/Microsoft Update, WSUS, or software-management process;
setup is not a background updater and does not change enterprise update policies.

No NSSM, Node.js, WebView2, interactive session, or tray application is required.
The installer registers `hbbs` (RustDesk ID Server) and `hbbr` (RustDesk Relay Server)
with automatic startup, LocalService identity, and crash/failure recovery.
Startup is reported complete only after the listening sockets are ready.
Stop and shutdown requests cancel the async server loop; active sessions end.

Programs are installed under `%ProgramFiles%\RustDeskServer`. Keys, database,
configuration (`.env`), and rotating logs live under `%ProgramData%\RustDeskServer`.
Only SYSTEM, Administrators, and the two RustDesk service SIDs have access to that
data directory. Both services run as LocalService, but other LocalService processes
receive no data access. The ID service can modify its data; the relay can read
keys/configuration and modify only its own log directory (`logs\hbbr`). Its service
depends on the ID service so keys exist before the relay starts. Back up this
directory before upgrades. Uninstall retains it and the
shared Microsoft runtime. Upgrading/re-running setup preserves keys and database.

Setup shows the client public key in `Connection-Details.txt`. Enter your server's
reachable hostname/IP and that public key in each client's ID Server and Key
settings. `RELAY_SERVERS` in `.env` optionally advertises a separate relay address.
Restart both services after changing configuration. This package expects standard
ports; custom listening ports require matching manual firewall changes.

### Firewall and network

Setup creates three inbound Windows Firewall rules tied to the installed programs:

| Program | Protocol | Ports |
| --- | --- | --- |
| hbbs.exe | TCP | 21115-21116 |
| hbbs.exe | UDP | 21116 |
| hbbr.exe | TCP | 21117 |

The default LAN scope permits LocalSubnet on Domain and Private profiles. A VPN
using another subnet requires an administrator to add its CIDR to the rules'
RemoteAddress. Internet scope permits remote addresses on all profiles. Upgrades
preserve existing rules (including manually added VPN subnets) unless a scope is
explicitly selected. Failed upgrades restore only the properties changed on the
three owned rules, preserving custom restrictions. No outbound allow rule is
added; environments blocking outbound traffic need administrator configuration.
Browser/WebSocket ports 21118 and 21119 remain closed. Router/NAT forwarding,
cloud firewalls, DNS, and NAT loopback remain external network configuration.

### Server Core and unattended setup

```powershell
.\RustDeskServer-Native-x64-Setup.exe /S /SCOPE=LAN /RELAY=rustdesk.example.com
```

Use `/SCOPE=Internet` for internet-facing rules, or `/SCOPE=Preserve` to preserve
existing rules (LAN for a new install). Optional `/D=C:\Your Installation Folder`
must be last, following NSIS conventions. Setup never automatically reboots;
exit code 3010 indicates the runtime requested a reboot.

Logs: `%TEMP%\RustDeskServer-Setup-*.log`, `%TEMP%\RustDeskServer-VisualCpp.log`,
and `%ProgramData%\RustDeskServer\logs`.

## Existing NSSM / GUI installations

Setup refuses to take over an existing `hbbs` or `hbbr` service without its native
installation manifest. Back up the old keys/database/configuration, stop and remove
the old services with the old installation's tools, and copy the preserved data
into ProgramData before installing. This avoids silently generating a new server
identity or overwriting an unrelated service. Legacy GUI packaging has been
replaced; the optional old GUI source can control already-installed native services.

## Build

Install Rust stable, MSVC C++ build tools with a Windows SDK, and NSIS 3. A licensed
Visual Studio/Build Tools environment must satisfy Microsoft's redistribution
terms for the runtime. From the repository root:

```powershell
.\packaging\windows\Build-Installer.ps1
```

Output: `target\windows-installer\RustDeskServer-Native-x64-Setup.exe`. Build metadata
records the bundled runtime version/hash and executable DLL imports. The build
fails if the binaries do not use the shared runtime or require unpackaged DLLs.
The installer and server binaries are unsigned unless your release process signs
them. Signing credentials are not included in this repository.

The build requires committed source and records a link to that exact revision in
the installed connection details. Preserve this source access with distributed
installers and prominently provide the corresponding source offer to network
users of modified deployments under AGPL section 13. The source checkout includes
the pinned submodule; clone recursively to retrieve it. This Windows deployment
support modifies the original packaging and service integration (October 2026).

For diagnostics, binaries still run in foreground mode. Service mode is explicit:
`hbbs.exe --service --data-dir C:\ProgramData\RustDeskServer` (launched by SCM).
Foreground runs do not use the service data directory automatically.

## Validation

On a disposable, elevated test machine for **each** target release:

```powershell
.\packaging\windows\Test-Installation.ps1 -InstallerPath .\target\windows-installer\RustDeskServer-Native-x64-Setup.exe
```

The integration test installs, validates ports/identities/firewall, stops/restarts
services, checks crash recovery, upgrades without replacing keys, and uninstalls
while retaining data. It refuses pre-existing services/installations/data. Use a
clean VM with no VC++ runtime to exercise the missing-runtime case, and another
with a newer runtime to confirm setup does not downgrade it. Also test runtime
failure/reboot conditions, a port conflict, and a malformed private key.

After installation, reboot and verify both services/listeners come online **before
any interactive login**. Test real remote sessions between two clients, including
a forced-relay connection across networks. Hosted CI cannot establish that all four
Server releases or those network scenarios have passed.
