Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!ifndef VERSION
  !define VERSION "1.3.0"
!endif
!ifndef STAGE
  !define STAGE "..\..\build\package-windows\stage"
!endif
!ifndef OUTFILE
  !define OUTFILE "..\..\dist\Setu-${VERSION}-setup.exe"
!endif
!define PRODUCT "Setu"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Setu"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "nsDialogs.nsh"

Name "${PRODUCT}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
BrandingText " "
ShowInstDetails nevershow
AutoCloseWindow true
ShowUninstDetails nevershow

Var StartAtSignIn
Var AddToMenu
Var StartCheck
Var MenuCheck
Var DeleteHistory
Var DeleteModels
Var DirText
Var HistCheck
Var ModelsCheck

!define MUI_ABORTWARNING
!define MUI_ABORTWARNING_TEXT "Cancel the installation? Nothing will be left on this computer."
!define MUI_CUSTOMFUNCTION_ABORT OnAbort
!define MUI_ICON "setu.ico"
!define MUI_UNICON "setu.ico"

Page custom InstallPage InstallPageLeave
!define MUI_PAGE_HEADER_TEXT "Installing ${PRODUCT}"
!define MUI_PAGE_HEADER_SUBTEXT " "
!insertmacro MUI_PAGE_INSTFILES

UninstPage custom un.ChoicesPage un.ChoicesLeave
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Function .onInit
  StrCpy $StartAtSignIn 1
  StrCpy $AddToMenu 1
FunctionEnd

Function InstallPage
  !insertmacro MUI_HEADER_TEXT "Install ${PRODUCT}" "Version ${VERSION}"
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 12u "Install folder:"
  Pop $0
  ${NSD_CreateText} 0 13u 75% 12u "$INSTDIR"
  Pop $DirText
  EnableWindow $DirText 0
  ${NSD_CreateButton} 77% 12u 23% 14u "Change folder"
  Pop $2
  ${NSD_OnClick} $2 ChangeFolder
  ${NSD_CreateLabel} 0 32u 100% 12u "Space needed: about 170 MB, plus about 1.5 GB of models that ${PRODUCT} downloads next."
  Pop $0
  ${NSD_CreateLink} 0 46u 100% 12u "Read the license"
  Pop $3
  ${NSD_OnClick} $3 ShowLicense
  ${NSD_CreateCheckbox} 0 66u 100% 12u "Start ${PRODUCT} when I sign in"
  Pop $StartCheck
  ${NSD_SetState} $StartCheck $StartAtSignIn
  ${NSD_CreateCheckbox} 0 80u 100% 12u "Add to Start menu"
  Pop $MenuCheck
  ${NSD_SetState} $MenuCheck $AddToMenu
  GetDlgItem $0 $HWNDPARENT 1
  SendMessage $0 ${WM_SETTEXT} 0 "STR:Install"
  nsDialogs::Show
FunctionEnd

Function ChangeFolder
  nsDialogs::SelectFolderDialog "Choose the install folder" "$INSTDIR"
  Pop $0
  ${If} $0 != "error"
    StrCpy $INSTDIR "$0\${PRODUCT}"
    SendMessage $DirText ${WM_SETTEXT} 0 "STR:$INSTDIR"
  ${EndIf}
FunctionEnd

Function ShowLicense
  ExecShell "open" "$PLUGINSDIR\LICENSE.txt"
FunctionEnd

Function InstallPageLeave
  ${NSD_GetState} $StartCheck $StartAtSignIn
  ${NSD_GetState} $MenuCheck $AddToMenu
FunctionEnd

Function .onInstSuccess
  ${If} ${Silent}
    Return
  ${EndIf}
  System::Call 'user32::AllowSetForegroundWindow(i -1)'
  SetOutPath "$INSTDIR"
  ${If} ${FileExists} "$LOCALAPPDATA\Setu\setup-complete"
    Exec '"$INSTDIR\setu-shell.exe"'
  ${Else}
    Exec '"$INSTDIR\setu-ui.exe" --page wizard'
  ${EndIf}
FunctionEnd

Section "Setu" Main
  InitPluginsDir
  File /oname=$PLUGINSDIR\LICENSE.txt "${STAGE}\licenses\setu.txt"
  nsExec::Exec 'taskkill /F /IM setu-shell.exe /IM setu-ui.exe /IM setu-daemon.exe /IM setu-engine.exe'
  SetOutPath "$INSTDIR"
  DetailPrint "Copying files"
  File /r "${STAGE}\*.*"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  DetailPrint "Registering ${PRODUCT}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Setu authors"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\setu-ui.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
  ${If} $StartAtSignIn == ${BST_CHECKED}
    DetailPrint "Registering sign-in start"
    WriteRegStr HKCU "${RUN_KEY}" "${PRODUCT}" '"$INSTDIR\setu-shell.exe"'
  ${EndIf}
  ${If} $AddToMenu == ${BST_CHECKED}
    CreateShortcut "$SMPROGRAMS\${PRODUCT}.lnk" "$INSTDIR\setu-ui.exe"
  ${EndIf}
SectionEnd

Function Rollback
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegValue HKCU "${RUN_KEY}" "${PRODUCT}"
  Delete "$SMPROGRAMS\${PRODUCT}.lnk"
FunctionEnd

Function .onInstFailed
  Call Rollback
FunctionEnd

Function OnAbort
  Call Rollback
FunctionEnd

Function un.onInit
  StrCpy $DeleteHistory 0
  StrCpy $DeleteModels 1
FunctionEnd

Function un.ChoicesPage
  !insertmacro MUI_HEADER_TEXT "Uninstall ${PRODUCT}" " "
  nsDialogs::Create 1018
  Pop $0
  ${GetSize} "$LOCALAPPDATA\Setu\models" "/S=0M" $4 $5 $6
  ${NSD_CreateCheckbox} 0 0 100% 12u "Also delete history, learned preferences and corrections"
  Pop $HistCheck
  ${NSD_SetState} $HistCheck $DeleteHistory
  ${NSD_CreateCheckbox} 0 16u 100% 12u "Delete downloaded models, freeing $4 MB"
  Pop $ModelsCheck
  ${NSD_SetState} $ModelsCheck $DeleteModels
  nsDialogs::Show
FunctionEnd

Function un.ChoicesLeave
  ${NSD_GetState} $HistCheck $DeleteHistory
  ${NSD_GetState} $ModelsCheck $DeleteModels
FunctionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /F /IM setu-shell.exe /IM setu-ui.exe /IM setu-daemon.exe /IM setu-engine.exe'
  nsExec::Exec '"$INSTDIR\setu-system.exe" --uninstall'
  DeleteRegValue HKCU "${RUN_KEY}" "${PRODUCT}"
  Delete "$SMPROGRAMS\${PRODUCT}.lnk"
  RMDir /r "$INSTDIR"
  ${If} $DeleteModels == ${BST_CHECKED}
    RMDir /r "$LOCALAPPDATA\Setu\models"
  ${EndIf}
  ${If} $DeleteHistory == ${BST_CHECKED}
    RMDir /r "$LOCALAPPDATA\Setu"
  ${EndIf}
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\ai.setu.browser"
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\ai.setu.browser"
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\ai.setu.browser"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\ai.setu.browser"
  DeleteRegKey HKCU "Software\Vivaldi\NativeMessagingHosts\ai.setu.browser"
  DeleteRegKey HKCU "Software\Opera Software\NativeMessagingHosts\ai.setu.browser"
  DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\ai.setu.browser"
SectionEnd
