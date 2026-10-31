; @tauri-apps/cli 2.12.3 includes hooks after utils.nsh. Replace its running-app
; macro: the upstream silent/passive path can call RmShutdown(RmForceShutdown).
; Never force-close an SSH terminal or an editor/log workflow during maintenance.
!ifmacrondef CheckIfAppIsRunning
  !error "YuuTerm installer requires review: Tauri's running-app macro changed"
!endif
!macroundef CheckIfAppIsRunning

Var YuuTermRestartManagerHandle
Var YuuTermRestartManagerResult

!macro CheckIfAppIsRunning executablePath productName
  StrCpy $YuuTermRestartManagerResult 0
  ${If} ${FileExists} "${executablePath}"
    Push $0
    Push $1
    Push $2
    Push $3
    !insertmacro RestartManager_StartSession $YuuTermRestartManagerHandle
    ${If} $YuuTermRestartManagerHandle == ""
      StrCpy $YuuTermRestartManagerResult 1
    ${Else}
      !insertmacro RestartManager_RegisterFile $YuuTermRestartManagerHandle "${executablePath}"
      ${If} $0 = 0
        StrCpy $1 0
        StrCpy $2 0
        StrCpy $3 0
        System::Call 'RSTRTMGR::RmGetList(i $YuuTermRestartManagerHandle, *i .r1, *i .r2, p 0, *i .r3) i .r0'
      ${EndIf}
      StrCpy $YuuTermRestartManagerResult $0
      !insertmacro RestartManager_EndSession $YuuTermRestartManagerHandle
    ${EndIf}
    Pop $3
    Pop $2
    Pop $1
    Pop $0
  ${EndIf}
  ${If} $YuuTermRestartManagerResult != 0
    DetailPrint "Close ${productName} before installing or uninstalling. No running process was stopped."
    IfSilent +2 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "Close ${productName} before installing or uninstalling, then try again. Your running connections have been left open."
    SetErrorLevel 10
    Abort
  ${EndIf}
!macroend
