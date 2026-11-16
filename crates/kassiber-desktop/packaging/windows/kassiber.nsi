; Kassiber per-user setup (NSIS 3) for the Rune desktop: the Tauri NSIS bundle's install mode
; (currentUser, %LOCALAPPDATA%\Kassiber) and its PATH hooks (installer-hooks.nsh ran
; windows\update-path.ps1 after install and before uninstall). Layout as in kassiber.wxs.
; Build (CI, Git Bash): packaging/windows/build-installers.sh. NEEDS A WINDOWS RUNNER.
; VERIFY before release that the uninstall key and directory match the installed Tauri NSIS
; app, so this setup upgrades it in place.
Unicode true
!include "MUI2.nsh"
!ifndef VERSION
  !error "pass /DVERSION=x.y.z"
!endif
!ifndef STAGE
  !error "pass /DSTAGE=<staged layout dir>"
!endif
Name "Kassiber"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Kassiber"
InstallDirRegKey HKCU "Software\Bitcoin Austria\Kassiber" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Kassiber"
!define MUI_ICON "${ICONS}\icon.ico"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "German"

Section "Kassiber"
  SetOutPath "$INSTDIR"
  File "${STAGE}\kassiber-ui.exe"
  SetOutPath "$INSTDIR\binaries"
  File "${STAGE}\binaries\kassiber-cli-x86_64-pc-windows-msvc.exe"
  File "${STAGE}\binaries\BUILD_INFO.json"
  SetOutPath "$INSTDIR\bin"
  File "${STAGE}\bin\kassiber.exe"
  SetOutPath "$INSTDIR\windows"
  File "${STAGE}\windows\update-path.ps1"
  CreateShortcut "$SMPROGRAMS\Kassiber.lnk" "$INSTDIR\kassiber-ui.exe"
  ; kassiber:// deep links, per user.
  WriteRegStr HKCU "Software\Classes\kassiber" "" "URL:kassiber"
  WriteRegStr HKCU "Software\Classes\kassiber" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\kassiber\DefaultIcon" "" "$INSTDIR\kassiber-ui.exe,0"
  WriteRegStr HKCU "Software\Classes\kassiber\shell\open\command" "" '"$INSTDIR\kassiber-ui.exe" "%1"'
  WriteRegStr HKCU "Software\Bitcoin Austria\Kassiber" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Kassiber"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Bitcoin Austria"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\kassiber-ui.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  ; The CLI on the user's PATH (Tauri: NSIS_HOOK_POSTINSTALL).
  nsExec::ExecToLog 'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\windows\update-path.ps1" -Action add -Directory "$INSTDIR\bin"'
  Pop $0
  StrCmp $0 0 +2
    DetailPrint "Kassiber CLI PATH integration returned exit code $0"
SectionEnd

Section "Uninstall"
  ; Tauri: NSIS_HOOK_PREUNINSTALL.
  nsExec::ExecToLog 'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\windows\update-path.ps1" -Action remove -Directory "$INSTDIR\bin"'
  Pop $0
  Delete "$SMPROGRAMS\Kassiber.lnk"
  DeleteRegKey HKCU "Software\Classes\kassiber"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Bitcoin Austria\Kassiber"
  RMDir /r "$INSTDIR\binaries"
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\windows"
  Delete "$INSTDIR\kassiber-ui.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
