; Installer hooks (tauri.conf.json -> bundle.windows.nsis.installerHooks). The same file is used by
; I1 Phone (i1-phone-desktop), the I1 Switchboard console and the ImagineOne desktop app
; (imagineone-frame-host).
;
; Moving from the Electron build of the app. Both builds have the same name ("Imagine One IT",
; a customer's name such as "NEFCO", ...). When the Electron one is installed for this user, this installer
;   - closes it (asks first, unless installing silently),
;   - carries over "Start with Windows" (if it had it),
;   - uninstalls it. Its settings in %APPDATA% stay: this app reads the same file,
;     so it stays pointed at the same server.
; When the Electron app's own updater ran us (silently, after it quit), the new app is started
; again at the end.
;
; How the Electron app is found: electron-builder writes an Uninstall key named by a GUID whose
; QuietUninstallString ends in "\Uninstall <product name>.exe" /currentuser /S, and its install
; folder under HKCU\Software\<same GUID>\InstallLocation. Its "Start with Windows" entry is named
; after its app id, which is our bundle id.

Var I1OldDir
Var I1OldUninst
Var I1Moved

!macro NSIS_HOOK_PREINSTALL
  StrCpy $I1Moved 0
  StrCpy $I1OldDir ""
  StrCpy $I1OldUninst ""
  StrCpy $R9 0
  ${Do}
    ClearErrors
    EnumRegKey $R8 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall" $R9
    ${If} ${Errors}
    ${OrIf} $R8 == ""
      ${Break}
    ${EndIf}
    IntOp $R9 $R9 + 1
    ${If} $R8 != "${PRODUCTNAME}"
      ReadRegStr $R7 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R8" "QuietUninstallString"
      ClearErrors
      ${WordFind} "$R7" "\Uninstall ${PRODUCTNAME}.exe$\"" "E+1{" $R6
      ${IfNot} ${Errors}
        ReadRegStr $R5 HKCU "Software\$R8" "InstallLocation"
        ${If} $R5 != ""
        ${AndIf} ${FileExists} "$R5\Uninstall ${PRODUCTNAME}.exe"
          StrCpy $I1OldDir "$R5"
          StrCpy $I1OldUninst "$R5\Uninstall ${PRODUCTNAME}.exe"
          ${Break}
        ${EndIf}
      ${EndIf}
    ${EndIf}
  ${Loop}

  ${If} $I1OldUninst != ""
    DetailPrint "Replacing the previous ${PRODUCTNAME} (from $I1OldDir)"
    !insertmacro CheckIfAppIsRunning "$I1OldDir\${PRODUCTNAME}.exe" "${PRODUCTNAME}"

    ReadRegStr $R5 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${BUNDLEID}"
    ${If} $R5 != ""
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${BUNDLEID}"
      DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${BUNDLEID}"
      ; Same format the app itself writes (tauri-plugin-autostart): "<exe> --hidden"
      WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe --hidden"
    ${EndIf}

    ; _?= runs the uninstaller in place so ExecWait really waits for it.
    ExecWait '"$I1OldUninst" /currentuser /S _?=$I1OldDir' $R4
    DetailPrint "Previous version removed (exit code $R4)"
    Delete "$I1OldUninst"
    RMDir "$I1OldDir"
    StrCpy $I1Moved 1
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $I1Moved = 1
  ${AndIf} ${Silent}
    nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" ""
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Really removed (not replaced by an update): take "Start with Windows" away too.
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"
  ${EndIf}
!macroend
