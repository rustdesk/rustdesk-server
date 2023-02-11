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
!define VERSION "1.1.7"

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

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

####################################################################
# Sections

Section "Install"
  SetShellVarContext all
  nsExec::Exec 'sc stop hbbr'
  nsExec::Exec 'sc stop hbbs'
  nsExec::Exec 'taskkill /F /IM ${PRODUCT_NAME}.exe'
  Sleep 500 ;

  SetOutPath $INSTDIR
  File /r "setup\*.*"
  WriteUninstaller $INSTDIR\uninstall.exe

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${PRODUCT_NAME}.exe"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk" "$INSTDIR\uninstall.exe"
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${PRODUCT_NAME}.exe"
  CreateShortCut "$SMSTARTUP\${APP_NAME}.lnk" "$INSTDIR\${PRODUCT_NAME}.exe"

  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=in action=allow program="$INSTDIR\hbbs.exe" enable=yes'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=out action=allow program="$INSTDIR\hbbs.exe" enable=yes'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=in action=allow program="$INSTDIR\hbbr.exe" enable=yes'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${APP_NAME}" dir=out action=allow program="$INSTDIR\hbbr.exe" enable=yes'
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  nsExec::Exec 'sc stop hbbr'
  nsExec::Exec 'sc stop hbbs'
  nsExec::Exec 'taskkill /F /IM ${PRODUCT_NAME}.exe'
  Sleep 500 ;

  RMDir /r "$SMPROGRAMS\${APP_NAME}"
  Delete "$SMSTARTUP\${APP_NAME}.lnk"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  nsExec::Exec 'sc delete hbbr'
  nsExec::Exec 'sc delete hbbs'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${APP_NAME}"'
SectionEnd

####################################################################
# Functions

Function CreateStartupShortcut
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${PRODUCT_NAME}.exe"
FunctionEnd