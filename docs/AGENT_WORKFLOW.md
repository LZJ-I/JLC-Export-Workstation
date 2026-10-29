# 用 Agent 将 BOM 导出成一个库

[返回中文 README](../README_ZH.md) · [English README](../README.md) · [完整 CLI / JSON 契约](CLI_API.md)

此流程使用 v0.10.0 起提供的 CLI，无需打开 GUI。下载并解压发布包即可运行；ZIP 中附带接口文档和示例 BOM。也可从源码编译。

## 一句话提示词

> 请使用 `lceda` CLI 处理 `./bom.csv`，核对器件编号、型号、厂商和封装，只批量确认原始立创 C 编号精确匹配的项，其余候选先列出让我确认，保留 BOM 数量和位号，将全部已确认器件导出为 `./out/board` 下名为 `BoardParts` 的一个 Altium 库，最后读回库文件并报告文件路径、器件数量和失败项，不修改我已有的器件库。

如果目标是 KiCad，把提示词里的 Altium 改成 KiCad。两种格式可以同时请求；“一个 Altium 库”会生成同名 `.SchLib` / `.PcbLib` 这一套文件，“一个 KiCad 库”会生成 `.kicad_sym` / `.pretty/`，有模型时还包含 `.3dshapes/`。

## 准备程序与示例

从 [v0.10.0 发布页](https://github.com/LZJ-I/JLC-Export-Workstation/releases/tag/v0.10.0) 下载对应平台的 ZIP 并解压。命令中的 `lceda` 使用解压后的程序路径；Linux/macOS 可用 `./lceda`，Windows 可用 `./lceda.exe`。如果选择源码编译：

```bash
cargo build -p lceda
```

以下以 `lceda` 已加入 PATH 为例；也可把它替换为 `./target/debug/lceda`。Windows 使用 `lceda.exe`。Agent 应先执行 `lceda --help` 和 `lceda bom --help` 确认命令可用。

仓库提供可运行的 [bom.csv](examples/bom.csv)：

```csv
LCSC Part Number,MPN,Manufacturer,Package,Quantity,Designator
C25804,,,,2,"R1,R2"
C14663,,,,1,C1
```

样例展示两个供应商编号、三个元件实例。型号、厂商和封装在联网匹配后核对；示例不预先填写可能变化的供应商资料。自己的 BOM 可以补充这些列，Agent 应把源表数据与候选数据对照。

## 从导入到导出

### 1. 导入并检查源表

```bash
lceda --profile ./work/profile bom import ./docs/examples/bom.csv --plan ./work/board.plan.json --json
```

这一步只解析文件。检查 `data.warnings` 和 `data.rows[].validation_error`。计划文件保留数量、位号和源表行号，不会添加到 GUI 收藏库。源表错误应修正后重新导入，或者明确排除不需要的行。

计划已存在时不会直接覆盖。继续任务用 `bom show`；确实要从修改后的 BOM 重新开始，再给 `bom import` 加 `--force`，旧匹配和确认将被替换。

### 2. 搜索并核对候选

```bash
lceda --profile ./work/profile bom match ./work/board.plan.json --json
lceda --profile ./work/profile bom show ./work/board.plan.json --json
```

对每个包含的行检查：

| 字段 | Agent 如何使用 |
| --- | --- |
| `line` | 源表行号，后续 `--select` / `--exclude` 使用它 |
| `input` | 原始编号、型号、厂商、封装、数量和位号 |
| `candidates` | 候选的 `lcsc`、`mpn`、`manufacturer`、`package` 和资源可用情况 |
| `selected_lcsc` | 当前建议或选择；不能单靠它判断已经确认 |
| `confirmed` | 是否经过明确确认 |
| `error` / `validation_error` | 区分搜索失败与源表错误 |

网络失败时可再次执行 `bom match`。已确认的行会跳过，其他搜索按去重后的查询并发执行，不会因为重试丢掉已确认的替代器件。

### 3. 明确确认

```bash
lceda --profile ./work/profile bom confirm ./work/board.plan.json --exact --json
```

这条命令对应提示词授权的“确认原始 C 编号一致项”，不会自动确认仅有 MPN 的行。若仍有待确认项，退出码为 1，同时保留刚完成的确认。

对于核对过的其他候选，按实际响应填写行号和编号：

```bash
# 以下行号和候选只是操作示例，不能直接套用到两行样例 BOM。
lceda bom confirm ./work/board.plan.json --select 4=C25804 --json
lceda bom confirm ./work/board.plan.json --exclude 7,8 --json
lceda bom confirm ./work/board.plan.json --include 7 --select 7=C14663 --json
```

只能选择该行匹配结果中已存在的 C 编号。存在歧义时先让用户确认；不要按搜索排名猜测，也不要悄悄排除失败行。

### 4. 导出合并库

确认 `data.ready` 为 `true` 后执行：

```bash
lceda --profile ./work/profile bom export ./work/board.plan.json --ad --merge --name BoardParts -o ./out/board --json
```

改成 `--ad --kicad` 可同时生成两种格式。若本次不需要附带 3D，可显式添加 `--no-ad-3d --no-kicad-3d`；必须下载模型时显式添加 `--step` 并检查结果。

导出目录应是本次任务的交付目录。普通导出会重新生成其中的库文件，`--force` 只控制资源刷新，不是覆盖开关。保留 `work/board.plan.json` 作为匹配记录；同一个已选 C 编号导出一次，数量不表示在库里复制多份定义。

即使文件已经出现，仍要检查进程退出码和 `data.results[].status`。失败时可能保留部分文件，不能把部分合并库当作完整交付。

### 5. 读回并交付

```bash
lceda library inspect ./out/board/BoardParts.SchLib --json
# 同时导出 KiCad 时：
lceda library inspect ./out/board/BoardParts.kicad_sym --json
```

核对实际器件数量、名称、属性、引脚数和封装关联。数量应与“所有包含行中已确认的不同 C 编号数量”一致，而非 BOM 的总用量。此检查是文件解析验证，EDA 软件中的最终检查仍由使用者完成。

Agent 最终报告应包含：输出库路径、成功 / 失败器件数、被明确排除的行、尚未解决的匹配问题，以及实际做过的验证。不要把文件生成或读回称为 Altium / KiCad 实机验证。

## Python subprocess 调用示例

这个示例使用发布包或仓库里的精确编号样例。如果换成自己的 BOM，遇到未确认项时会停下来报告，不会自行挑选或跳过。

```python
import json
from pathlib import Path
import subprocess
import sys

exe = str(Path("lceda.exe" if sys.platform == "win32" else "lceda").resolve())
# 从源码编译时改为 target/debug/lceda（Windows: target/debug/lceda.exe）。
profile = "./work/agent-profile"
plan = "./work/agent-board.plan.json"
output = "./out/agent-board"


def call(*args, allow_partial=False):
    process = subprocess.run(
        [exe, "--profile", profile, "--json", *map(str, args)],
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=600,
        check=False,
    )
    if process.stderr:
        print(process.stderr, file=sys.stderr, end="")
    # Launch failures / timeouts raise; a killed process may have no JSON.
    try:
        response = json.loads(process.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"No valid JSON response (exit {process.returncode})") from error
    if response.get("schema_version") != 1:
        raise RuntimeError("Unsupported CLI response schema")
    if process.returncode != 0 and not (allow_partial and process.returncode == 1):
        raise RuntimeError(json.dumps(response, ensure_ascii=False))
    return response["data"]


# To resume an existing plan, omit import; import deliberately refuses to clobber it.
call("bom", "import", "./docs/examples/bom.csv", "--plan", plan)
call("bom", "match", plan, allow_partial=True)
review = call("bom", "confirm", plan, "--exact", allow_partial=True)
if not review["ready"]:
    pending = [row for row in review["rows"]
               if row["included"] and (not row["confirmed"] or row["error"])]
    raise RuntimeError("Review these source rows before export:\n" + json.dumps(pending, ensure_ascii=False, indent=2))

expected = {row["selected_lcsc"] for row in review["rows"] if row["included"]}
result = call("bom", "export", plan, "--ad", "--kicad", "--merge", "--name", "BoardParts",
              "--no-ad-3d", "--no-kicad-3d", "-o", output)
for extension in ("SchLib", "kicad_sym"):
    library = call("library", "inspect", str(Path(output) / f"BoardParts.{extension}"))
    if library["total"] != len(expected):
        raise RuntimeError(f"Readback count mismatch: {library['total']} vs {len(expected)}")
print(json.dumps(result, ensure_ascii=False, indent=2))
```

Agent 应按任务规模设置超时，并且等待当前进程结束再重试。`bom match` / `confirm` 使用计划锁；中断后如果留下 `.lock`，先确认对应进程已结束，再清理锁。不要用循环快速重试正在写入的同一个计划或输出目录。

更多参数、字段类型、失败处理和接口范围见 [CLI_API.md](CLI_API.md)。
