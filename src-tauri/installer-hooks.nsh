; 只更新现有快捷方式的图标引用，保留启动参数、应用标识及用户的其他快捷方式设置。
; 32512 为当前 EXE 中已核对的 RT_GROUP_ICON 资源 ID，负值表示按资源 ID 引用。
!macro QingjianRefreshShortcutIcon shortcut
  ${If} ${FileExists} "${shortcut}"
    !insertmacro IsShortcutTarget "${shortcut}" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $3
    ${If} $3 = 1
      !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
      ${If} $0 P<> 0
        ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
        ${If} $1 P<> 0
          ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE}).r2'
          ${If} $2 = 0
            ${IShellLink::SetIconLocation} $0 '(w "$INSTDIR\${MAINBINARYNAME}.exe", i -32512).r2'
            ${If} $2 = 0
              ${IPersistFile::Save} $1 '("${shortcut}",1)'
            ${EndIf}
          ${EndIf}
          ${IUnknown::Release} $1 ""
        ${EndIf}
        ${IUnknown::Release} $0 ""
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  Push $1
  Push $2
  Push $3
  !insertmacro QingjianRefreshShortcutIcon "$DESKTOP\${PRODUCTNAME}.lnk"
  !insertmacro QingjianRefreshShortcutIcon "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  !if "${STARTMENUFOLDER}" != ""
    !insertmacro QingjianRefreshShortcutIcon "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  !endif
  ; 卸载列表也显式使用相同图标；更新原注册项，不创建第二个应用入口。
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\",-32512'
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend
