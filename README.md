**Language / 语言:** **English** · [中文](README_ZH.md)

# JLC-Export

搜索立创器件，预览 3D，导出 Altium / KiCad / PADS 库和 STEP。
Search LCSC parts, preview 3D, and export Altium / KiCad / PADS libraries plus STEP.

[![Stars](https://img.shields.io/github/stars/LZJ-I/JLC-Export?style=social)](https://github.com/LZJ-I/JLC-Export)

位号写入 Designator，型号写入 Comment，Description 可勾选规格和编号。
Designator gets the prefix, Comment the MPN, and Description a configurable template.

也可搜：立创EDA 导出 Altium、嘉立创封装库、LCSC 转 KiCad。
Also try: LCSC to Altium, JLCPCB library, EasyEDA SchLib / PcbLib.

如果这个项目对你有帮助，欢迎点一下右上角的 [⭐ Star](https://github.com/LZJ-I/JLC-Export)。
If this project helps you, please [⭐ Star](https://github.com/LZJ-I/JLC-Export) it.

![搜索、3D、位号 Description、导出 / Search, 3D, Designator, export](docs/demo.gif)

许可为 [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)。
Licensed under [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/).

## Install

Download a zip for your OS from [Releases](https://github.com/LZJ-I/JLC-Export/releases), unzip, and run it. To update, unzip over the previous files, or use in-app update (downloads the matching OS zip, replaces, restarts).

- Windows x64: `lceda-v*-x86_64-pc-windows-msvc.zip` → `lceda.exe`
- Linux x64: `lceda-v*-x86_64-unknown-linux-gnu.zip` → `lceda`
- macOS Apple Silicon: `lceda-v*-aarch64-apple-darwin.zip` → `lceda`
- macOS Intel: `lceda-v*-x86_64-apple-darwin.zip` → `lceda`

Or build from source (requires [Rust](https://rustup.rs/)):

```bash
cargo build --release -p lceda
```

## GUI

Open `lceda.exe` (Windows) or `lceda` (Linux / macOS). Files go to `lceda-out` under your Downloads folder by default. Change the path in **Settings**.

Each part gets a subfolder named `MPN_LCSC_Manufacturer`. Files inside use the MPN.

The left sidebar switches pages: **Search**, **Favorites**, **Export list**, **Sponsor**, **Settings**, **About**. The title bar is drawn by the app (drag to move, double-click to maximize). Window size, splitters, and theme are remembered.

1. Type an MPN (e.g. `STM32F103C8T6`) or LCSC id (e.g. `C8734`), then press Enter or **Search**. Double-click a row to add it to the export list.
2. Select a part. Photo on the top right; drag in the lower half to orbit the 3D outline. Drag the splitters to resize the panes.
3. Use the buttons on the right. **Export** always opens a checklist first — nothing is written until you confirm:
   - **Download STEP / OBJ**: 3D models
   - **Export Altium**: `.SchLib` / `.PcbLib` (EasyEDA JSON is kept too)
   - **Export KiCad**: `.kicad_sym` and `.pretty` (writes `.3dshapes` and a footprint model ref when a STEP exists)
   - **Export PADS**: `.c` schematic decal, `.d` PCB decal, `.p` part type (classic Logic / Layout; not PADS Professional / Xpedition Central Library)
   - **Datasheet**: PDF. LCSC often gives an HTML page; the app pulls the real PDF from it
   - **Open LCSC**: Chinese product page (`item.szlcsc.com`)

Dimmed buttons mean this part has no matching asset. Click anyway for a notice; no empty folder is created.

**Favorites** holds categories (right-click to create / rename / import / export / delete). **Export list** is a queue you can import, export, or clear.

**Settings** controls language, appearance, the save folder, which boxes are pre-ticked on Export, 3D attach options, footprint renaming, batch merge, and **schematic colors** for Altium `.SchLib`:

- **Altium classic** (default): maroon body and pins (`COLOR=128`), blue designator / comment, black pin text
- **LCSC official**: store symbol preview — maroon `#880000` body, blue pin text, red power, black ground (input/output share the signal colors)
- **Black & white**: for print / IEEE-style sheets
- **Custom**: pick each color; shown as a live preview

3D options default on (a missing model is skipped). Footprint names stay as LCSC left them unless you turn renaming on.

**Sponsor** loads QR codes from the shared [LZJ-I/sponsor](https://github.com/LZJ-I/sponsor) repo. GitHub’s funding link points there only.

Exporting Altium / KiCad / PADS also keeps EasyEDA JSON for inspection.

### Batch file

One MPN or LCSC id per line. Lines starting with `#` or `//` are comments. In the GUI, **Batch file…** opens a checklist so you can export only some formats (PADS only, Altium only, etc.).

```
# example
C8734
STM32F030C8T6
C2040
```

## CLI

No arguments opens the GUI.

```bash
lceda search C2040
lceda get C2040 --step --ad --kicad --pads -o ./out
lceda get C2040 --kicad --no-kicad-3d -o ./out
lceda get C2040 --ad --no-ad-3d -o ./out
lceda get C2040 --ad --rename-footprint -o ./out
lceda get C2040 --ad --sch-color altium -o ./out
lceda get C2040 --source -o ./out
lceda get C2040 --datasheet -o ./out
lceda batch ids.txt --ad --kicad --pads --step -o ./out
lceda --lang zh gui
```

`batch` uses the same file format; CLI flags (`--ad`, `--kicad`, `--pads`, …) pick the types. `--sch-color` accepts `altium` (default), `easyeda`, or `mono`. The GUI custom palette is used when the CLI omits the flag and Settings is set to Custom.

## Notes

Check exported libraries in Altium / KiCad / PADS before using them. This tool talks to unofficial LCSC / EasyEDA APIs, which may change.

## About

- Author: [LZJ-I](https://github.com/LZJ-I)
- Repository: [LZJ-I/JLC-Export](https://github.com/LZJ-I/JLC-Export)
- License: [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)

If it helps, please [Star](https://github.com/LZJ-I/JLC-Export) the repo.
