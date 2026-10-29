# 自更新流程与验证

## 参考实现

2026-10-29 查阅了以下上游源码与官方说明，采用相同的文件替换及权限处理原则，未复制上游源码：

- [self-replace 的 Windows 实现](https://github.com/mitsuhiko/self-replace/blob/main/src/windows.rs)：运行中的 Windows EXE 可以重命名；替换与旧文件清理由不同阶段处理。
- [Tauri updater](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/updater/src/updater.rs)：候选文件验证、同卷暂存、替换失败恢复以及独立安装过程。
- [ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)：以系统 `runas` 动词请求管理员授权并检查进程结果。
- [GitHub Release assets](https://docs.github.com/en/rest/releases/assets)：从所选平台附件的 `digest` 读取 SHA-256。
- [Windows 长路径要求](https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation)：声明 `longPathAware`，同时避免不支持长路径的工作目录和未使用的 OLE 拖放注册。

## 更新步骤

1. 按平台选择 ZIP，官方源优先、代理作为备用；内容与附件 SHA-256 不符时尝试另一线路，全部失败则保留当前进程。
2. 限制解压文件大小，将程序写入本次独立暂存目录。启动候选程序的探测入口，核对版本、协议与实际程序路径。
3. 锁住目标的更新操作，检查旧、新文件摘要。完整写入目标所在磁盘，落盘并保存恢复记录，然后将旧文件移入备份、把新文件移到原路径。
4. Windows 原文件只读时解除文件自身的只读属性。遇到访问被拒绝时调用独立安装助手，使用系统管理员授权；不拼接 shell 命令、不自动修改目录 ACL。
5. 由原用户进程启动原路径的新程序。新窗口初始化成功后写入含版本、路径与本次标识的确认；协调进程确认后提交更新，此时旧进程才退出。
6. 新版获得提交确认后清理已核对的旧备份。确认前不能清理备份；受权限或其他进程占用影响无法清理时保留备份。
7. 新版无法启动或未确认时终止本次未就绪的子进程，并核对摘要后恢复旧版。外部修改或备份损坏时保留现场，避免误覆盖。

不结束其他应用或按名称批量杀进程，不绕过用户拒绝的 UAC、安全软件或系统策略。持续占用或被系统拦截时，正确行为是保留可用旧版并说明原因，而不是静默关闭或声称成功。

Windows 桌面程序的权限声明为 `asInvoker`。旧版本已经先退出的兼容入口会在安装失败后尝试重新打开原程序，并显示独立错误提示。详细原因保存在配置目录的 `update-last-error.txt`，错误提示会显示具体路径。

## 自动与原生检查

通用测试覆盖版本比较、附件选择及摘要、损坏文件拒绝、完整备份和回滚、并发更新锁、外部修改保护、清理前核验和 Unix 执行权限。Windows 增加只读恢复与原生命令行参数解析检查。

真实 Windows GUI 更新检查需先构建普通程序及测试程序，再运行：

```powershell
./scripts/test_windows_updater.ps1 -TestBinary <lceda-tests.exe> -AppBinary <lceda.exe> -EvidenceDirectory <临时证据目录>
```

脚本只操作临时程序与隔离配置。它检查运行中的 EXE、只读文件、中文和空格、超过 260 字符的路径、新版启动失败回滚、版本不符、文件共享锁、原生提权助手以及旧入口兼容。成功场景保存实际新窗口截图，失败场景验证原文件摘要与原进程仍在，并输出 JSON 结果。

测试机上的管理员令牌运行可以验证 `runas` 安装子进程；不等同于已替用户点击或验证其电脑上的 UAC 授权策略。用户取消授权、企业策略禁止执行或安全软件强制拦截时不会绕过限制。
