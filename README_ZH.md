**语言 / Language:** [English](README.md) · **中文**

# JLC-Export

[![Stars](https://img.shields.io/github/stars/LZJ-I/JLC-Export?style=social)](https://github.com/LZJ-I/JLC-Export)

用 Rust 编写。按型号或立创编号搜索器件，下载 STEP / OBJ，导出 Altium（`.SchLib` / `.PcbLib`）、KiCad（`.kicad_sym` / `.pretty`）和经典 PADS Logic / Layout ASCII（`.c` / `.d` / `.p`）。界面支持中文 / 英文。

如果这个项目对你有帮助，欢迎点一下右上角的 [⭐ Star](https://github.com/LZJ-I/JLC-Export)。

![搜索 ESP32-S3-WROOM-1-N16R8 并悬停放大预览图](docs/demo.gif)

许可为 [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)。

## 安装

从 [Releases](https://github.com/LZJ-I/JLC-Export/releases) 下载对应系统的压缩包，解压后运行。更新时解压覆盖即可。软件内检查更新会按当前系统下载对应安装包并替换重启。

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

左侧导航切换：**搜索**、**收藏**、**导出列表**、**赞助**、**设置**、**关于**。顶栏是程序自绘（拖动移动，双击最大化）。窗口大小、分栏和主题会记住。

1. 在搜索框输入型号（如 `STM32F103C8T6`）或立创编号（如 `C8734`），回车或点「搜索」。双击一行可加入导出列表。
2. 点选器件。右上是预览图，下半部分可拖动查看三维外形。中间的缝可以拖，用来改栏宽和上下高度。
3. 用右侧按钮下载或导出。点 **导出** 会先弹出勾选，确认后才写文件：
   - **下载 STEP / OBJ**：三维模型
   - **导出 AD 库**：`.SchLib` / `.PcbLib`（同时保留 EasyEDA JSON）
   - **导出 KiCad**：`.kicad_sym` 与 `.pretty`（有模型时写入 `.3dshapes` 并在封装中引用）
   - **导出 PADS**：`.c` 原理图 Decal、`.d` PCB Decal、`.p` Part Type（经典 Logic / Layout；Professional / Xpedition 不能当 Central Library）
   - **数据手册**：PDF。立创给的经常是 HTML 页，程序会从页面里取出真正的 PDF
   - **打开立创页**：国内站商品页（`item.szlcsc.com`）

按钮变灰表示当前器件没有对应资源；再点一下会弹出说明，不会建空文件夹。

**收藏** 按分类管理（右键新建 / 改名 / 导入 / 导出 / 删除）。**导出列表** 可导入、导出或清空。

**设置** 里可改语言、外观、保存目录、导出勾选预勾、3D 附带、封装改名、批量合并，以及 **原理图颜色**（写入 Altium `.SchLib`）：

- **Altium 经典**（默认）：外形和管脚暗红（`COLOR=128`），位号 / 型号蓝色，管脚字黑色——和 AD 自带库、手工建库一致
- **立创 / EasyEDA**：蓝框红脚（以前软件导出的配色）
- **黑白**：适合打印
- **自定义**：逐项选色，旁边有预览

3D 两项默认开启（没有模型则跳过）。封装名默认保持立创原名，需要再勾选改名。

**赞助** 的收款码来自共用仓库 [LZJ-I/sponsor](https://github.com/LZJ-I/sponsor)，GitHub 赞助入口也只挂这一处。

导出 AD / KiCad / PADS 时会同时留下 EasyEDA JSON，供对照，不必单独再导一遍。

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
lceda get C2040 --ad --sch-color easyeda -o ./out
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
- 仓库：[LZJ-I/JLC-Export](https://github.com/LZJ-I/JLC-Export)
- 许可：[CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)

有帮助的话，欢迎 [Star](https://github.com/LZJ-I/JLC-Export)。
