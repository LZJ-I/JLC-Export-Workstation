**语言：** [English](README.md) · **中文**

# 嘉立创导出工作站

发布版本：**0.9.0**。

| 中文 | English |
| --- | --- |
| 搜索立创器件，预览 3D（可嵌入封装），导出 Altium / KiCad / PADS 库和 STEP。 | Search LCSC parts, preview 3D (embeddable in footprints), and export Altium / KiCad / PADS libraries plus STEP. |
| 位号写入 Designator，型号写入 Comment，Description 可勾选规格和编号。 | Designator gets the prefix, Comment the MPN, and Description is a configurable template. |
| 用 Rust 编写。原生桌面程序，Windows 约 13 MB，内存占用低，没有 Electron，也不用额外运行时。 | Written in Rust. One small native binary (~13 MB on Windows), low memory use, no Electron or extra runtime. |
| 许可为 [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)。 | Licensed under [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/). |

![搜索、3D、位号 Description、导出](docs/demo.gif)

## 安装

从 [Releases](https://github.com/LZJ-I/JLC-Export-Workstation/releases) 下载对应系统的压缩包，解压后运行。更新时解压覆盖即可。软件内检查更新会按当前系统下载对应安装包并替换重启。

- Windows x64：`lceda-v*-x86_64-pc-windows-msvc.zip` → `lceda.exe`
- Linux x64：`lceda-v*-x86_64-unknown-linux-gnu.zip` → `lceda`
- macOS Apple Silicon：`lceda-v*-aarch64-apple-darwin.zip` → `lceda`
- macOS Intel：`lceda-v*-x86_64-apple-darwin.zip` → `lceda`

或从源码编译（需 [Rust](https://rustup.rs/)）：

```bash
cargo build --release -p lceda
```

## 图形界面

打开 `lceda.exe`（Windows）或 `lceda`（Linux / macOS）。默认把文件保存到「下载」文件夹下的 `lceda-out`，路径在 **设置** 里改。

每个器件一个子目录，名字是 `型号_立创编号_厂牌`，目录里的文件用型号命名。

左侧导航切换：**搜索**、**器件库**、**导出列表**、**赞助**、**设置**、**关于**。BOM 从器件库的 **导入 BOM…** 进入。顶栏可拖动移动、双击最大化；窗口大小、分栏和主题会记住。

1. 在搜索框输入型号（如 `STM32F103C8T6`）或立创编号（如 `C8734`），回车或点「搜索」。双击一行可加入导出列表。
2. 打开时默认搜索并展示 10kΩ 电阻（C25804）。右侧顶部显示放大的器件图片、资源状态和操作按钮；下方使用一个完整的大画布，通过下拉框切换 **原理图 / 三维外形 / 封装**。切换项会记住；滚轮缩放、拖动预览，双击恢复视图。原理图直接读取实际 AD 导出记录，显示位号、型号和可见管脚文字，字体、字号及颜色沿用导出设置；封装显示实际焊盘、钻孔和外形。
3. 点 **导出…**，在同一勾选框中选择库文件、三维模型或 PDF，确认后写文件：
   - **下载 STEP / OBJ**：三维模型
   - **导出 AD 库**：`.SchLib` / `.PcbLib`（同时保留 EasyEDA JSON）
   - **导出 KiCad**：`.kicad_sym` 与 `.pretty`（有模型时写入 `.3dshapes` 并在封装中引用）
   - **导出 PADS**：`.c` 原理图 Decal、`.d` PCB Decal、`.p` Part Type（经典 Logic / Layout；Professional / Xpedition 不能当 Central Library）
   - **数据手册**：PDF。立创给的经常是 HTML 页，程序会从页面里取出真正的 PDF
   - **打开立创页**：国内站商品页（`item.szlcsc.com`）

资源不可用时，操作会禁用或提示原因。「下载资源」已合并到 **导出…**，可只选 STEP、OBJ 或 PDF。**详细信息** 和 **打开立创页** 是独立按钮；窄窗口中操作自动换行。

底栏“任务记录”采用居中的按钮并预留固定空间，较长保存路径不会挤挡按钮；点击可查看当前会话的搜索、导出和同步记录。

点 **加入器件库** 后选择目标库；只有一个库时也会显示选择框。**器件库** 保留原收藏分类管理（右键新建 / 改名 / 导入 / 导出 / 删除）。**导出列表** 可导入、导出或清空。

**合并本次器件到同一个库** 只出现在多个可合并器件的批量导出框里。单器件导出和全局设置不显示此项；已绑定的 AD 器件仍同步到各自的库文件。

**设置** 里可改语言、外观、保存目录、导出勾选预勾、3D 附带、封装改名，以及 **原理图颜色**（写入 Altium `.SchLib`）：

- **Altium 经典**（默认）：外形和管脚暗红（`COLOR=128`），位号 / 型号蓝色，管脚字黑色
- **立创官方**：商城符号预览——暗红框 `#880000`，普通脚蓝字，电源红、地黑（输入/输出不再另配色）
- **黑白**：适合打印
- **自定义**：逐项选色，旁边有预览

3D 两项默认开启（没有模型则跳过）。封装名默认保持立创原名，需要再勾选改名。

**设置 → 导出 → Description** 使用左右结构：左侧勾选和拖动排序，右侧查看器件的 Designator / Comment / Description。预览默认加载 10kΩ 电阻（C25804），修改勾选立即更新，也可查询其他器件。

**赞助** 的收款码来自共用仓库 [LZJ-I/sponsor](https://github.com/LZJ-I/sponsor)，GitHub 赞助入口也只挂这一处。

导出 AD / KiCad / PADS 时会同时留下 EasyEDA JSON，供对照，不必单独再导一遍。

### BOM 导入器件库

在 **器件库 → 导入 BOM…** 选择 CSV / TSV / Excel（xlsx、xls、xlsb、ods）和目标器件库，保留数量和位号。先匹配，再核对候选的型号、厂商和封装；「确认编号一致项」只确认与源表 C 编号完全一致的结果，并保留已手动选择的替代器件。仅有型号的行需要手动选择。所有包含的行确认后，点 **加入此器件库**；也可排除不需要的行。匹配支持进度、取消和重试。

器件库沿用原收藏及分类。此前新增的独立本地搜索、备注和资料管理界面已移除；旧收藏、设置及内部预览缓存继续保留。目前在线搜索源仍为立创。

### 每个器件库绑定一个 Altium 库

右键一个器件库，选择 **绑定库文件…**，指定现有或新建的 `.SchLib`。配套封装库使用同目录、同名的 `.PcbLib`。每个器件库绑定一对文件，同一文件不允许绑定多个器件库。加入器件、导入 BOM 或修改 AD 颜色、描述及 3D 等导出设置后，会自动同步；也可右键 **同步库文件** 重试。

同步沿用设置中的颜色、描述字段、封装改名和 3D 嵌入选项。器件以 C 编号命名、去重并更新，型号仍写入 Comment；已有其他器件和附加数据保留。移除收藏或解除绑定不删除外部库文件中的器件。

更新有文件锁、成对替换、中断恢复和上一版备份（同目录隐藏的 `.文件名.jlc-backup`）。现有库的结构不受支持、文件损坏或被其他程序写入时会报错，避免覆盖。首次使用建议选库副本。

已移除全局“导出 AD 时同步到固定库”及固定目录设置。自动同步只使用器件库绑定的文件；未绑定的器件按普通导出流程写入保存目录。普通导出不会先重建绑定库。

在 Altium 的库管理中，将这对文件安装一次。后续导出沿用同一路径，无需每个器件手动重新导入；AD 可能需要刷新或重新打开库。此版本不包含向当前原理图自动放置器件的 AD 插件。

本版先支持 AD 绑定。KiCad 绑定的[可行性评估](docs/KICAD_BINDING_ASSESSMENT_ZH.md)已完成；原有 KiCad 单器件和合并导出继续可用，尚未增加 KiCad 自动同步绑定。

快捷键：`Ctrl+F` 搜索，`Ctrl+O` 导入 BOM，`Ctrl+Enter` 打开导出确认框。

### 批量文件

每行一个型号或立创编号，`#` 或 `//` 开头为注释。界面里点「批量文件…」会先弹出勾选框，可只导出其中几种（例如只要 PADS，或只要 AD）。

```
# 例子
C8734
STM32F030C8T6
C2040
```

## 命令行

无参数打开图形界面。

```bash
lceda search C2040
lceda get C2040 --step --ad --kicad --pads -o ./out
lceda get C2040 --kicad --no-kicad-3d -o ./out
lceda get C2040 --ad --no-ad-3d -o ./out
lceda get C2040 --ad --rename-footprint -o ./out
lceda get C2040 --ad --sch-color altium -o ./out
lceda get C2040 --ad --ad-library-file ./MyParts.SchLib -o ./out
lceda get C2040 --source -o ./out
lceda get C2040 --datasheet -o ./out
lceda batch ids.txt --ad --kicad --pads --step -o ./out
lceda --lang zh gui
```

`batch` 文本文件格式与上面相同；命令行用 `--ad` / `--kicad` / `--pads` 等开关选择类型。`--sch-color` 可选 `altium`（默认）、`easyeda`、`mono`。命令行没写时，若设置里是自定义，会用你保存的那套颜色。

## 说明

导出结果请在 Altium / KiCad / PADS 中核对后再使用。本工具使用立创非官方接口，可能随时变化。

## 关于

- 作者：[LZJ-I](https://github.com/LZJ-I)
- 仓库：[LZJ-I/JLC-Export-Workstation](https://github.com/LZJ-I/JLC-Export-Workstation)
- 许可：[CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)

有帮助的话，欢迎点一下 Star。[![Stars](https://img.shields.io/github/stars/LZJ-I/JLC-Export-Workstation?style=social)](https://github.com/LZJ-I/JLC-Export-Workstation)
