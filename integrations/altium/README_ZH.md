# AD 编辑器连接

此脚本由嘉立创导出工作站生成，仅在当前 Windows 用户的本地目录通信，无需服务器、共享目录、管理员权限或安装 SDK。

1. 在 AD 打开 `LcedaBridge.PrjScr`。
2. 在 AD 的 **File → Run Script**（部分版本在 **DXP → Run Script**）选择 `LCEDA_StartBridge`。
3. 保留小型连接窗口，返回工作站。“同步库”会让 AD 追加器件、保存、重开目标库并读回器件，完成后显示 AD 中的实际名称。
4. 关闭连接窗口即断开；重启 AD 后需再次运行脚本。

同步前请保存目标库中的手工编辑。脚本遇到未保存的目标库会拒绝操作；干净的目标库会先重新打开，以读取磁盘最新内容，避免旧缓存覆盖此前的文件同步。已有同名符号或封装会保留，既有封装、3D 关联及电路图实例不会被更新。新符号名称是“型号_C编号”，封装名称使用 C 编号避免冲突；旧 C 编号名称的符号继续保留原名，仍使用该 C 编号在 AD 名称列表筛选。

每次任务的输入、进度、结果和同步前的库备份在 `jobs/<任务编号>`。失败或超时不会自动退回磁盘写入；若 AD 只完成其中一个库的保存，任务会报错并保留备份，不会宣称整个任务成功。

没有运行 AD 时，未启用连接的工作站仍可使用原来的文件同步。Linux 保持文件同步；AD 编辑器连接仅供 Windows。普通“导出”仍导出独立副本。

实现参考 EasyEDALoader 的编辑器流程（打开库、追加、保存），以及 Altium 官方脚本 API；独立实现，不包含 EasyEDALoader 的 GPL 源码或 DLL。模型复制和定时器运行方式参考了 eda-agent 的公开 API 用法。

- https://github.com/expired6978/EasyEDALoader
- https://github.com/salitronic/eda-agent
- https://www.altium.com/documentation/altium-dxp-developer/pcb-api-system-interfaces-reference

验证边界：工作站端的连接协议、超时、错误处理和保存文件校验可独立测试。脚本需要在实际安装的 AD 中验证；开源项目能运行不等于本脚本已在所有 AD 版本验证。
