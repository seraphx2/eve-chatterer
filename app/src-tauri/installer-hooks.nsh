; Tauri NSIS installer hooks (same pattern as dev-prompt).
;
; Force a Start Menu shortcut on every install, including the updater's silent
; reinstall: the app lives in the tray with no window, so the shortcut is the
; only way back in after quitting it.

!macro NSIS_HOOK_POSTINSTALL
  CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"

  ; Start at login on a fresh install, silently; the user can turn it off on
  ; the General page. Skipped when $UpdateMode is set so an update never
  ; brings back a choice the user turned off. tauri-plugin-autostart reads and
  ; writes this exact value, so the in-app checkbox stays in sync.
  ${If} $UpdateMode = 0
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}" '"$INSTDIR\${MAINBINARYNAME}.exe" --autostart'
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"

  ; Start at login: the Run value and the Task Manager on/off state Windows
  ; keeps beside it. Left behind, they point at a deleted exe.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"

  ; The notification identity the app registers on first run (toast.rs), so
  ; Windows notifications show its name and icon.
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\io.github.seraphx2.evechatterer"
!macroend
