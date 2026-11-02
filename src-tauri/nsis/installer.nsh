; 安装完成后无条件创建桌面快捷方式。
;
; Tauri 模板把这个动作挂在「完成页」的复选框上（MUI_FINISHPAGE_SHOWREADME），
; 默认不勾选，用户装完桌面上就什么都没有。这里在安装结束后直接调用它，
; 不等用户勾。
;
; 函数名与行为来自 tauri-bundler 的 installer.nsi：
;   - 新建快捷方式：CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" ...
;   - 已存在且指向旧主程序：更新目标路径
;   - 卸载时由模板自带逻辑清理 $DESKTOP\${PRODUCTNAME}.lnk
!macro NSIS_HOOK_POSTINSTALL
  Call CreateOrUpdateDesktopShortcut
!macroend
