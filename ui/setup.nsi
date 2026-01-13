Unicode true

####################################################################
# Includes

!include nsDialogs.nsh
!include MUI2.nsh
!include x64.nsh
!include LogicLib.nsh

####################################################################
# File Info

!define APP_NAME "RustDeskServer"
!define PRODUCT_NAME "rustdesk_server"
!define PRODUCT_DESCRIPTION "Installer for ${PRODUCT_NAME}"
!define COPYRIGHT "Copyright © 2021"
!define VERSION "1.1.15"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${PRODUCT_DESCRIPTION}"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"

####################################################################
# Installer Attributes

Name "${APP_NAME}"
Outfile "${APP_NAME}.Setup.exe"
Caption "Setup - ${APP_NAME}"
BrandingText "${APP_NAME}"

ShowInstDetails show
RequestExecutionLevel admin
SetOverwrite on
 
InstallDir "$PROGRAMFILES64\${APP_NAME}"

####################################################################
# Pages

!define MUI_ICON "icons\icon.ico"
!define MUI_ABORTWARNING
!define MUI_LANGDLL_ALLLANGUAGES
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Create Startup Shortcut"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateStartupShortcut
!define MUI_FINISHPAGE_RUN "$INSTDIR\${PRODUCT_NAME}.exe"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

####################################################################
# Language

!insertmacro MUI_LANGUAGE "English" ; The first language is the default language
!insertmacro MUI_LANGUAGE "French"
!insertmacro MUI_LANGUAGE "German"
!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "SpanishInternational"
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "TradChinese"
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "Korean"
!insertmacro MUI_LANGUAGE "Italian"
!insertmacro MUI_LANGUAGE "Dutch"
!insertmacro MUI_LANGUAGE "Danish"
!insertmacro MUI_LANGUAGE "Swedish"
!insertmacro MUI_LANGUAGE "Norwegian"
!insertmacro MUI_LANGUAGE "NorwegianNynorsk"
!insertmacro MUI_LANGUAGE "Finnish"
!insertmacro MUI_LANGUAGE "Greek"
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "Portuguese"
!insertmacro MUI_LANGUAGE "PortugueseBR"
!insertmacro MUI_LANGUAGE "Polish"
!insertmacro MUI_LANGUAGE "Ukrainian"
!insertmacro MUI_LANGUAGE "Czech"
!insertmacro MUI_LANGUAGE "Slovak"
!insertmacro MUI_LANGUAGE "Croatian"
!insertmacro MUI_LANGUAGE "Bulgarian"
!insertmacro MUI_LANGUAGE "Hungarian"
!insertmacro MUI_LANGUAGE "Thai"
!insertmacro MUI_LANGUAGE "Romanian"
!insertmacro MUI_LANGUAGE "Latvian"
!insertmacro MUI_LANGUAGE "Macedonian"
!insertmacro MUI_LANGUAGE "Estonian"
!insertmacro MUI_LANGUAGE "Turkish"
!insertmacro MUI_LANGUAGE "Lithuanian"
!insertmacro MUI_LANGUAGE "Slovenian"
!insertmacro MUI_LANGUAGE "Serbian"
!insertmacro MUI_LANGUAGE "SerbianLatin"
!insertmacro MUI_LANGUAGE "Arabic"
!insertmacro MUI_LANGUAGE "Farsi"
!insertmacro MUI_LANGUAGE "Hebrew"
!insertmacro MUI_LANGUAGE "Indonesian"
!insertmacro MUI_LANGUAGE "Mongolian"
!insertmacro MUI_LANGUAGE "Luxembourgish"
!insertmacro MUI_LANGUAGE "Albanian"
!insertmacro MUI_LANGUAGE "Breton"
!insertmacro MUI_LANGUAGE "Belarusian"
!insertmacro MUI_LANGUAGE "Icelandic"
!insertmacro MUI_LANGUAGE "Malay"
!insertmacro MUI_LANGUAGE "Bosnian"
!insertmacro MUI_LANGUAGE "Kurdish"
!insertmacro MUI_LANGUAGE "Irish"
!insertmacro MUI_LANGUAGE "Uzbek"
!insertmacro MUI_LANGUAGE "Galician"
!insertmacro MUI_LANGUAGE "Afrikaans"
!insertmacro MUI_LANGUAGE "Catalan"
!insertmacro MUI_LANGUAGE "Esperanto"
!insertmacro MUI_LANGUAGE "Asturian"
!insertmacro MUI_LANGUAGE "Basque"
!insertmacro MUI_LANGUAGE "Pashto"
!insertmacro MUI_LANGUAGE "ScotsGaelic"
!insertmacro MUI_LANGUAGE "Georgian"
!insertmacro MUI_LANGUAGE "Vietnamese"
!insertmacro MUI_LANGUAGE "Welsh"
!insertmacro MUI_LANGUAGE "Armenian"
!insertmacro MUI_LANGUAGE "Corsican"
!insertmacro MUI_LANGUAGE "Tatar"
!insertmacro MUI_LANGUAGE "Hindi"

####################################################################
# Sections

Section "Install"
  SetShellVarContext all
  nsExec::Exec 'sc stop hbbr'
  nsExec::Exec 'sc stop hbbs'
  nsExec::Exec 'taskkill /F /IM ${PRODUCT_NAME}.exe'
  Sleep 500

  SetOutPath $INSTDIR
  File /r "setup\*.*"
  WriteUninstaller $INSTDIR\uninstall.exe

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${PRODUCT_NAME}.exe"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk" "$INSTDIR\uninstall.exe"
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${PRODUCT_NAME}.exe"

  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=in action=allow program="$INSTDIR\bin\hbbs.exe" enable=yes'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=out action=allow program="$INSTDIR\bin\hbbs.exe" enable=yes'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=in action=allow program="$INSTDIR\bin\hbbr.exe" enable=yes'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=out action=allow program="$INSTDIR\bin\hbbr.exe" enable=yes'
  ExecWait 'powershell.exe -NoProfile -windowstyle hidden try { [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.SecurityProtocolType]::Tls12 } catch {}; Invoke-WebRequest -Uri "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile "$$env:TEMP\MicrosoftEdgeWebview2Setup.exe" ; Start-Process -FilePath "$$env:TEMP\MicrosoftEdgeWebview2Setup.exe" -ArgumentList ($\'/silent$\', $\'/install$\') -Wait'
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  nsExec::Exec 'sc stop hbbr'
  nsExec::Exec 'sc stop hbbs'
  nsExec::Exec 'taskkill /F /IM ${PRODUCT_NAME}.exe'
  Sleep 500

  RMDir /r "$SMPROGRAMS\${APP_NAME}"
  Delete "$SMSTARTUP\${APP_NAME}.lnk"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  nsExec::Exec 'sc delete hbbr'
  nsExec::Exec 'sc delete hbbs'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${APP_NAME}"'
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\logs"
  RMDir /r "$INSTDIR\service"
  Delete "$INSTDIR\${PRODUCT_NAME}.exe"
  Delete "$INSTDIR\uninstall.exe"
SectionEnd

####################################################################
# Functions

Function CreateStartupShortcut
  CreateShortCut "$SMSTARTUP\${APP_NAME}.lnk" "$INSTDIR\${PRODUCT_NAME}.exe"
FunctionEnd
