; Hooks for the NSIS package, wired in through `bundle.windows.nsis.installerHooks` in
; tauri.conf.json. This package feeds the silent auto-updater; people install with the custom
; installer in installer-app/ instead. It keeps the stock MUI2 wizard, because MUI2 can only theme
; its welcome and finish pages and a dark frame around light inner pages looks broken.

!macro NSIS_HOOK_PREINSTALL
  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2
  ${If} $0 > 1024
    CopyFiles /SILENT "$INSTDIR\uninstall.exe" "$TEMP\fern-uninstall.keep"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} ${FileExists} "$TEMP\fern-uninstall.keep"
    CopyFiles /SILENT "$TEMP\fern-uninstall.keep" "$INSTDIR\uninstall.exe"
    Delete "$TEMP\fern-uninstall.keep"
    WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\" --uninstall"
  ${Else}
    Delete "$INSTDIR\uninstall.exe"
    WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --uninstall"
  ${EndIf}
  FileOpen $0 "$INSTDIR\.install-scope" w
  ${If} $MultiUser.InstallMode == "AllUsers"
    FileWrite $0 "all-users"
  ${Else}
    FileWrite $0 "user"
  ${EndIf}
  FileClose $0
!macroend
