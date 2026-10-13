# 嘉立创导出工作站 · AD 原生插件（0.9.0）

在工作站“器件库 → 同步库”选择“安装插件并连接 AD…”，进入连接窗口后点击“安装原生插件”；也可在“设置 → 常规 → AD 原生插件 → 管理插件…”打开该窗口。安装前关闭 AD，安装后启动 AD。插件随 AD 加载，后台连接当前 Windows 用户的工作站，不需要运行 `LcedaBridge.PrjScr`，没有常驻连接窗口。

安装器使用该电脑上 AD 配套的 `Altium.SDK.dll`、`Altium.SDK.Interfaces.dll` 编译原生 C# 模块。**安装 AD 不一定同时安装 SDK**；缺少时需在对应 AD 版本安装 Altium Developer SDK。没有这些接口文件时安装会停止，工作站不会将脚本伪装成已安装的插件。本发布包不分发 Altium 商业接口文件。目标为带旧版托管扩展接口的 64 位 AD；新版本接口不同会明确报错，未经实际 AD 验证的版本不宣称兼容。

自动发现仅适用于一个 AD 安装和一个扩展注册目录。多版本时在连接窗口选择安装目录（含 `X2.exe`）以及对应 ProgramData 下含 `ExtensionsRegistry.xml` 的 `Extensions` 目录，避免把插件装到错误版本。

也可以在 Windows PowerShell 5.1 运行：

```powershell
./Install.ps1 -AdDirectory 'D:\Altium\AD24' -ExtensionsDirectory 'C:\ProgramData\Altium\Altium Designer {实际安装GUID}\Extensions'
```

没有目录写权限时，使用管理员身份运行工作站或上述安装命令。更新、卸载同样要求 AD 关闭。注册表 XML 与旧插件会留备份；安装失败会恢复。在“设置 → 常规 → AD 原生插件”点击“卸载 AD 插件”，或在连接窗口点击“卸载插件”；命令行也可加 `-Action Uninstall`，不会删除器件库、绑定关系和绑定库文件。设置页显示处理状态与结果，未安装或正在处理时不会重复卸载。

同步规则：仅向当前器件库明确绑定的 `.SchLib` / 同名 `.PcbLib` 追加缺少的器件。已有器件、封装、模型与关联保持原样；新符号才设置本次封装关联，新封装才嵌入本次 STEP。用户尚未保存的库会拒绝同步。AD 主线程串行编辑，执行前备份，保存后关闭、重新打开并查找器件；工作站还检查磁盘中的管脚、封装和新增 STEP。失败或超时保留现场与备份，不自动重写、不替换旧器件、不更新已放入设计的实例。

工作站关闭或 AD 退出不会凭旧 IPC 请求再次写入。工作站排队记录只保存待同步的器件库范围，重新连接时按当前绑定和器件列表生成新任务；执行失败的任务需要用户查看结果后手动重试。

Windows 默认顺序为“连接 AD → 绑定库文件 → 同步器件”。未连接时绑定会先引导连接，连接成功后继续选库，仍需确认绑定。已有绑定的同步范围持久化等待；缺少、停用或断开插件都不会自动切换到磁盘写入，KiCad 可独立继续。

可在工作站连接窗口明确选择“改用文件同步（需关闭 AD）”，关闭 AD 后离线绑定和同步；选择保存在本机。重新启用连接或安装插件会退出文件模式，安装失败保留原模式。Linux 继续使用本地文件同步，普通导出副本不依赖 AD 连接。解除绑定会取消对应等待范围，保留外部库和器件。

架构参考：[EasyEDALoader](https://github.com/expired6978/EasyEDALoader) 的 `CSharpPlugin.PluginFactory`、`ServerModule` 和 INS/RCS 加载流程；此处实现是独立的任务协议与保留旧库的同步逻辑，没有引入其下载、界面和器件生成代码。启动加载依据 [Altium SDK FAQ](https://www.altium.com/documentation/altium-dxp-developer/delphi-sdk-how-to-faq?version=18) 的 `SystemExtension=True`。机器人消息值依据官方 [原理图常量](https://www.altium.com/documentation/altium-dxp-developer/schematic-api-constants-reference) 与 [PCB 常量](https://www.altium.com/documentation/altium-dxp-developer/pcb-api-constants-reference)。

验证范围：Windows 编译器可编译运行时，协议、线程限制和安装事务可独立测试。SDK 替身仅用于检查原生工厂的语法与安装事务，不能证明 AD 实机加载或库编辑成功；实际 AD 的编译、自动加载、保存与读回仍需在安装目标上验证。
