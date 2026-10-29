[English](README.md) · **中文**

# 嘉立创导出工作站

搜索立创器件，查看原理图、封装和 3D 模型，导出 **Altium、KiCad、PADS** 器件库。使用 Rust 编写，提供原生桌面界面，也可通过 CLI 自动处理 BOM。

[下载 v0.9.3](https://github.com/LZJ-I/JLC-Export-Workstation/releases) · [图形界面指南](docs/GUI_GUIDE_ZH.md) · [CLI 接口文档](docs/CLI_API.md) · [Agent 自动化](#agent-自动化)

![搜索、3D 预览与器件库导出](docs/demo.gif)

| 用途 | 功能 |
| --- | --- |
| 搜索与预览 | 按立创编号或型号搜索，预览原理图、封装与可旋转的 3D 模型 |
| 生成器件库 | Altium `.SchLib` / `.PcbLib`、KiCad 符号 / 封装、PADS `.c` / `.d` / `.p` |
| 批量处理 BOM | 导入 CSV / TSV / Excel，核对候选、确认编号，将 AD / KiCad 合并成库 |
| 保存配套资源 | STEP / OBJ、可选封装附带 3D、数据手册与 EasyEDA 源 JSON |

## 安装

从 [Releases](https://github.com/LZJ-I/JLC-Export-Workstation/releases) 下载对应平台的压缩包，解压运行，无需 Electron 或额外桌面运行时。

| 平台 | 压缩包 | 程序 |
| --- | --- | --- |
| Windows x64 | `lceda-v*-x86_64-pc-windows-msvc.zip` | `lceda.exe` |
| Linux x64 | `lceda-v*-x86_64-unknown-linux-gnu.zip` | `lceda` |
| macOS Apple Silicon | `lceda-v*-aarch64-apple-darwin.zip` | `lceda` |
| macOS Intel | `lceda-v*-x86_64-apple-darwin.zip` | `lceda` |

也可安装 [Rust](https://rustup.rs/) 后从当前源码编译：

```bash
cargo build --release -p lceda
# 程序位于 target/release/lceda，Windows 为 lceda.exe
```

软件内更新会校验安装包并保留旧版备份，也可以解压新版覆盖更新。

## 图形界面快速开始

1. 输入立创编号或型号，选中结果，在 **原理图 / 封装 / 3D** 之间切换预览。
2. 点击 **导出…**，勾选格式和资源后确认。默认保存到「下载」文件夹下的 `lceda-out`，可在 **设置** 中修改。
3. 批量器件使用 **器件库 → 导入 → 导入 BOM…**，核对匹配后导出整库；也支持导入已有 AD / KiCad 库。

**设置 → 导出** 提供 Description 字段、原理图颜色、3D 附带和 PADS 连接栅格。**任务记录** 可查看搜索、导出和同步结果。快捷键：`Ctrl+F` 搜索、`Ctrl+O` 导入 BOM、`Ctrl+Enter` 导出。

分类管理、原生库导入、资源补全和绑定同步详见[图形界面指南](docs/GUI_GUIDE_ZH.md)。**AD 同步属于实验性功能**，请先用库副本评估；普通导出生成独立副本。

## CLI

无参数时打开 GUI，带子命令时无需打开窗口。

**下文的 JSON、BOM 计划和库检查接口属于当前源码新增功能，尚未包含在 v0.9.3 发布包中；使用时请先从源码编译。**

```bash
lceda search C2913202 --json
lceda get C2913202 --ad --kicad --pads --step -o ./out/part --json
lceda batch parts.txt --ad --kicad --merge --name BoardParts -o ./out/board --json
lceda library inspect ./out/board/BoardParts.SchLib --json
```

`parts.txt` 每行一个立创编号或型号，以 `#`、`//` 开头的行视为注释。C 编号必须精确匹配，不会回退到其他器件；型号有歧义时需明确选择。`--merge` 合并 AD / KiCad，PADS 仍按器件输出。

运行 `lceda --help` 或 `lceda bom export --help` 查看命令。[CLI 接口文档](docs/CLI_API.md) 逐项列出参数、默认值、JSON 字段、退出码、重试方式和文件写入位置。

## Agent 自动化

可以把下面这一句话交给 Agent，按需替换 BOM 路径、输出目录和目标格式：

> 请使用 `lceda` CLI 处理 `./bom.csv`，核对器件编号、型号、厂商和封装，只批量确认原始立创 C 编号精确匹配的项，其余候选先列出让我确认，保留 BOM 数量和位号，将全部已确认器件导出为 `./out/board` 下名为 `BoardParts` 的一个 Altium 库，最后读回库文件并报告文件路径、器件数量和失败项，不修改我已有的器件库。

Agent 可通过以下命令完成流程：

```bash
# 1. 本地解析 BOM，保存源表行号、数量和位号。
lceda bom import ./bom.csv --plan ./work/board.plan.json --json

# 2. 搜索候选，再明确确认原始 C 编号精确匹配项。
lceda bom match ./work/board.plan.json --json
lceda bom confirm ./work/board.plan.json --exact --json

# 3. 检查待确认行；用源表行号和 C 编号选择已核对的候选。
lceda bom show ./work/board.plan.json --json
# 仅举例：lceda bom confirm ./work/board.plan.json --select 4=C25804 --json

# 4. data.ready 为 true 后，导出合并库并读回检查。
lceda bom export ./work/board.plan.json --ad --merge --name BoardParts -o ./out/board --json
lceda library inspect ./out/board/BoardParts.SchLib --json
```

导出 KiCad 时改用 `--kicad`，并检查 `BoardParts.kicad_sym`；也可以同时选择两种格式。计划文件独立于 GUI 收藏库，同一个器件只导出一次，原始数量和位号保留在计划中。

`--json` 将一个完整响应写入 stdout，诊断写入 stderr。退出码 **0** 表示命令完成，**1** 表示部分失败或仍待确认，**2** 表示命令失败。导出前必须确认所有包含的行；先检查 `data.ready`，导出后再检查逐项结果。

[完整示例、提示词与 Python 调用方式](docs/AGENT_WORKFLOW.md) · [JSON / API 契约](docs/CLI_API.md)

## 导出格式

| 格式 | 文件 | 说明 |
| --- | --- | --- |
| Altium | `.SchLib`、`.PcbLib` | 可内嵌 STEP；Designator 写位号前缀，Comment 写型号 |
| KiCad | `.kicad_sym`、`.pretty/`、可选 `.3dshapes/` | KiCad 9 格式，需在 KiCad 中添加导出库 |
| PADS | `.c`、`.d`、`.p` | 经典 Logic / Layout，不适用于 Professional / Xpedition Central Library |
| 模型 / 文档 | `.step`、`.obj`、`.mtl`、`.pdf` | 取决于器件可用资源 |
| 源数据 | EasyEDA JSON | 保留转换输入，便于核对和重新生成 |

使用前请在 EDA 软件中核对结果。立创 / EasyEDA 使用非官方接口，数据和接口可能变化。[验证范围](docs/VERIFICATION_0.9.0_ZH.md) · [PADS 栅格说明](docs/PADS_GRID_ZH.md)

## 开发

```bash
cargo test --workspace
```

## 项目

作者 [LZJ-I](https://github.com/LZJ-I) · [CC BY-NC 4.0](LICENSE) · [赞助](https://github.com/LZJ-I/sponsor)

如果这个工具帮到了你，欢迎点一个 [Star](https://github.com/LZJ-I/JLC-Export-Workstation)。
