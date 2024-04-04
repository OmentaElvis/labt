!include "MUI2.nsh"
!include "nsDialogs.nsh"
!addplugindir  "plugins/"

Name "LABt v${VERSION}"
OutFile "labt-v${VERSION}_x86_64_windows-installer.exe"
InstallDir "$PROGRAMFILES\LABt\"

; Request admin
RequestExecutionLevel admin

InstallDirRegKey HKLM "Software\LABt" "Install_Dir"

; Installer icon
!define MUI_ICON "assets/icon.ico"

!define MUI_LICENSEPAGE_CHECKBOX
!insertmacro MUI_PAGE_LICENSE "LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
Page custom addToPathPage addToPathPageLeave
!insertmacro MUI_PAGE_INSTFILES

!insertmacro MUI_LANGUAGE English

Var Dialog
Var Label
Var Checkbox
Var Checkbox_State

; "Add LABt to path page" 
Function addToPathPage
	nsDialogs::Create 1018
	Pop $Dialog

	${If} $Dialog == error
		Abort
	${EndIf}

	${NSD_CreateLabel} 0 0 100% 12u "Add LABt to your current user environment PATH variable?"
	Pop $Label

	${NSD_CreateCheckbox} 0 12u 100% 12u "Add to PATH"
	Pop $Checkbox

	${If} $Checkbox_State == ${BST_CHECKED}
		${NSD_Check} $Checkbox
	${EndIf}

	nsDialogs::Show
FunctionEnd

Function addToPathPageLeave

	${NSD_GetState} $Checkbox $Checkbox_State

FunctionEnd


Section "labt (required)"
    WriteRegStr HKLM SOFTWARE\LABt "Install_Dir" "$INSTDIR"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\LABt" "DisplayName" "LABt"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\LABt" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\LABt" "NoModify" 1
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\LABt" "NoRepair" 1

    WriteUninstaller "$INSTDIR\uninstall.exe"

    SetOutPath $INSTDIR
    File "LICENSE"


    SetOutPath "$INSTDIR\bin\"
    File "labt.exe"

    ${If} $Checkbox_State == ${BST_CHECKED}
		EnVar::SetHKCU
    	EnVar::AddValue "PATH" "$INSTDIR\bin"
		Pop $0
		DetailPrint "EnVar::AddValue returned=|$0|"
    ${EndIf}


SectionEnd

Section "Uninstall"
  
  ; Remove registry keys
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\LABt"
  DeleteRegKey HKLM SOFTWARE\LABt

  ; Remove files and uninstaller
  Delete $INSTDIR\bin\*
  Delete $INSTDIR\*

  ; Remove shortcuts, if any
  Delete "$SMPROGRAMS\LABt\*.lnk"

  ; Remove directories
  RMDir "$SMPROGRAMS\LABt"
  RMDir "$INSTDIR\bin"
  RMDir "$INSTDIR"

SectionEnd
