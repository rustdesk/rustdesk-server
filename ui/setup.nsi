Unicode true
!include MUI2.nsh
!include LogicLib.nsh
!include x64.nsh
!include WinVer.nsh
!include nsDialogs.nsh
!include FileFunc.nsh
!ifndef PAYLOAD
  !error "Build using packaging/windows/Build-Installer.ps1"
!endif
!ifndef OUTPUT
  !define OUTPUT "RustDeskServer-Native-x64-Setup.exe"
!endif
Name "RustDesk Server (Native Windows Services)"
OutFile "${OUTPUT}"
InstallDir "$PROGRAMFILES64\RustDeskServer"
InstallDirRegKey HKLM "Software\RustDeskServerNative" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
ShowInstDetails show
VIProductVersion "1.1.17.0"
VIAddVersionKey "ProductName" "RustDesk Server Native"
VIAddVersionKey "FileDescription" "RustDesk Server installer for Windows Server 2016, 2019, 2022 and 2025"
VIAddVersionKey "FileVersion" "1.1.17.0"
VIAddVersionKey "LegalCopyright" "RustDesk contributors"
Var RelayAddress
Var RelayInput
Var Scope
Var InternetCheck
Var Dialog
Var Result
Var Parameters
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_NOREBOOTSUPPORT
!define MUI_FINISHPAGE_RUN "$WINDIR\System32\notepad.exe"
!define MUI_FINISHPAGE_RUN_PARAMETERS '$\"$INSTDIR\Connection-Details.txt$\"'
!define MUI_FINISHPAGE_RUN_TEXT "Show connection details"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PAYLOAD}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
Page custom Configure ConfigureLeave
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "This installer requires 64-bit Windows." /SD IDOK
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "Windows Server 2016 or later is required." /SD IDOK
    Abort
  ${EndIf}
  SetRegView 64
  StrCpy $Scope "Preserve"
  StrCpy $RelayAddress ""
  ${GetParameters} $Parameters
  ${GetOptions} $Parameters "/RELAY=" $RelayAddress
  ${GetOptions} $Parameters "/SCOPE=" $Result
  ${If} $Result != ""
    StrCpy $Scope $Result
  ${EndIf}
FunctionEnd

Function Configure
  !insertmacro MUI_HEADER_TEXT "Server connection" "Configure the server address and Windows Firewall."
  nsDialogs::Create 1018
  Pop $Dialog
  ${NSD_CreateLabel} 0 0 100% 28u "Relay hostname or IP address (optional). Use the address clients can reach. Leave blank to preserve an existing setting."
  Pop $Result
  ${NSD_CreateText} 0 30u 100% 13u "$RelayAddress"
  Pop $RelayInput
  ${NSD_CreateCheckbox} 0 55u 100% 24u "Enable internet access (unchecked preserves existing scope)"
  Pop $InternetCheck
  ${If} $Scope == "Internet"
    ${NSD_Check} $InternetCheck
  ${EndIf}
  ${NSD_CreateLabel} 0 86u 100% 52u "New installations default to local-subnet access on Domain/Private networks. Upgrades preserve the existing firewall scope. Internet access also needs router or cloud firewall configuration. Browser ports remain closed."
  Pop $Result
  nsDialogs::Show
FunctionEnd

Function ConfigureLeave
  ${NSD_GetText} $RelayInput $RelayAddress
  ${NSD_GetState} $InternetCheck $Result
  ${If} $Result == ${BST_CHECKED}
    StrCpy $Scope "Internet"
  ${ElseIf} $Scope == "Internet"
    StrCpy $Scope "LAN"
  ${EndIf}
  StrCpy $Result 0
  loop:
    StrCpy $Dialog $RelayAddress 1 $Result
    StrCmp $Dialog "" done
    StrCmp $Dialog '$\"' invalid
    StrCmp $Dialog "$\r" invalid
    StrCmp $Dialog "$\n" invalid
    IntOp $Result $Result + 1
    Goto loop
  invalid:
    MessageBox MB_ICONSTOP "Enter a hostname or IP address without quotes or line breaks."
    Abort
  done:
FunctionEnd

Section "Install"
  SetRegView 64
  SetShellVarContext all
  InitPluginsDir
  SetOutPath "$PLUGINSDIR\payload"
  File "${PAYLOAD}\hbbs.exe"
  File "${PAYLOAD}\hbbr.exe"
  File "${PAYLOAD}\vc_redist.x64.exe"
  File "${PAYLOAD}\Install-Services.ps1"
  File "${PAYLOAD}\build-info.json"
  File "${PAYLOAD}\README.txt"
  File "${PAYLOAD}\LICENSE"
  ${DisableX64FSRedirection}
  nsExec::ExecToLog '"$WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\payload\Install-Services.ps1" -Mode Install -InstallDir "$INSTDIR" -PayloadDir "$PLUGINSDIR\payload" -RelayServer "$RelayAddress" -NetworkScope "$Scope"'
  Pop $Result
  ${EnableX64FSRedirection}
  ${If} $Result == 3010
    SetRebootFlag true
    SetErrorLevel 3010
  ${ElseIf} $Result != 0
    MessageBox MB_ICONSTOP "Setup failed. See the details above and RustDeskServer-Setup logs in your temporary folder." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  SetOutPath "$INSTDIR"
  CopyFiles /SILENT "$PLUGINSDIR\payload\build-info.json" "$INSTDIR"
  CopyFiles /SILENT "$PLUGINSDIR\payload\README.txt" "$INSTDIR"
  CopyFiles /SILENT "$PLUGINSDIR\payload\LICENSE" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "Software\RustDeskServerNative" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\RustDeskServerNative" "DisplayName" "RustDesk Server (Native Services)"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\RustDeskServerNative" "DisplayVersion" "1.1.17"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\RustDeskServerNative" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\RustDeskServerNative" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\RustDeskServerNative" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetRegView 64
  ${DisableX64FSRedirection}
  nsExec::ExecToLog '"$WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\Install-Services.ps1" -Mode Uninstall -InstallDir "$INSTDIR"'
  Pop $Result
  ${EnableX64FSRedirection}
  ${If} $Result != 0
    MessageBox MB_ICONSTOP "Services could not be removed. Files have been retained; inspect the setup log." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  Delete "$INSTDIR\hbbs.exe"
  Delete "$INSTDIR\hbbr.exe"
  Delete "$INSTDIR\Install-Services.ps1"
  Delete "$INSTDIR\Connection-Details.txt"
  Delete "$INSTDIR\native-install.json"
  Delete "$INSTDIR\build-info.json"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\RustDeskServerNative"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\RustDeskServerNative"
SectionEnd
