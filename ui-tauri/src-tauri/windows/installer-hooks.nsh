!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog 'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\windows\update-path.ps1" -Action add -Directory "$INSTDIR\bin"'
  Pop $0
  ${If} $0 != 0
    DetailPrint "Kassiber CLI PATH integration returned exit code $0"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\windows\update-path.ps1" -Action remove -Directory "$INSTDIR\bin"'
  Pop $0
  ${If} $0 != 0
    DetailPrint "Kassiber CLI PATH cleanup returned exit code $0"
  ${EndIf}
!macroend
