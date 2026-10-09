**Language / 语言:** **English** · [中文](README_ZH.md)

# JLC Export Workstation

Release version: **0.9.0**.

| 中文 | English |
| --- | --- |
| 搜索立创器件，预览 3D（可嵌入封装），导出 Altium / KiCad / PADS 库和 STEP。 | Search LCSC parts, preview 3D (embeddable in footprints), and export Altium / KiCad / PADS libraries plus STEP. |
| 位号写入 Designator，型号写入 Comment，Description 可勾选规格和编号。 | Designator gets the prefix, Comment the MPN, and Description is a configurable template. |
| 用 Rust 编写。原生桌面程序，Windows 约 13 MB，内存占用低，没有 Electron，也不用额外运行时。 | Written in Rust. One small native binary (~13 MB on Windows), low memory use, no Electron or extra runtime. |
| 许可为 [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)。 | Licensed under [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/). |

![Search, 3D, Designator, export](docs/demo.gif)

## Install

Download a zip for your OS from [Releases](https://github.com/LZJ-I/JLC-Export-Workstation/releases), unzip, and run it. To update, unzip over the previous files, or use in-app update (downloads the matching OS zip, replaces, restarts).

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

The sidebar switches between **Search**, **Library**, **Export list**, **Sponsor**, **Settings**, and **About**. BOM import is inside Library. Drag the title bar to move, or double-click to maximize. Window size, splitters, and theme are remembered.

1. Type an MPN (e.g. `STM32F103C8T6`) or LCSC id (e.g. `C8734`), then press Enter or **Search**. Double-click a row to add it to the export list.
2. Startup searches for a 10kΩ resistor (C25804). The header shows a larger part photo, resource status and actions. One full-size canvas has a remembered dropdown for **Schematic / 3D / Footprint**. Scroll to zoom, drag, and double-click to reset. The schematic reads actual AD export records, including designator, MPN and visible pin text, with the exported fonts, sizes and colors. The footprint shows actual pads, holes and outlines.
3. **Export…** opens one checklist for libraries, models and PDF. Confirm your selections to write files:
   - **Download STEP / OBJ**: 3D models
   - **Export Altium**: `.SchLib` / `.PcbLib` (EasyEDA JSON is kept too)
   - **Export KiCad**: `.kicad_sym` and `.pretty` (writes `.3dshapes` and a footprint model ref when a STEP exists)
   - **Export PADS**: `.c` schematic decal, `.d` PCB decal, `.p` part type (classic Logic / Layout; not PADS Professional / Xpedition Central Library)
   - **Datasheet**: PDF. LCSC often gives an HTML page; the app pulls the real PDF from it
   - **Open LCSC**: Chinese product page (`item.szlcsc.com`)

Unavailable assets are disabled or explained. Download asset has been folded into **Export…**; selecting only STEP, OBJ or PDF downloads those resources. **Details** and **Open LCSC** are separate buttons. Actions wrap in narrower windows.

The centered **Activity** button reserves its space in the status bar, so long output paths cannot obscure it. Open it to review search, export and sync activity for this session.

**Add to library** always asks for the destination, including when only one library exists. **Library** retains favorite categories (right-click to create / rename / import / export / delete). **Export list** is a queue you can import, export, or clear.

**Combine these parts into one library** appears only when a batch contains multiple parts that can be combined. It is absent from single-part export and Settings. Bound AD parts still sync to their own library files.

**Settings** controls language, appearance, the save folder, which boxes are pre-ticked on Export, 3D attach options, footprint renaming, and **schematic colors** for Altium `.SchLib`:

- **Altium classic** (default): maroon body and pins (`COLOR=128`), blue designator / comment, black pin text
- **LCSC official**: store symbol preview — maroon `#880000` body, blue pin text, red power, black ground (input/output share the signal colors)
- **Black & white**: for print / IEEE-style sheets
- **Custom**: pick each color; shown as a live preview

3D options default on (a missing model is skipped). Footprint names stay as LCSC left them unless you turn renaming on.

**Settings → Export → Description** places field selection and drag ordering on the left, and the Designator / Comment / Description preview on the right. The preview defaults to a 10kΩ resistor (C25804), updates immediately when fields change, and can look up another part.

**Sponsor** loads QR codes from the shared [LZJ-I/sponsor](https://github.com/LZJ-I/sponsor) repo. GitHub’s funding link points there only.

Exporting Altium / KiCad / PADS also keeps EasyEDA JSON for inspection.

### Import BOM into a library

Use **Library → Import BOM…** for CSV / TSV / Excel (xlsx, xls, xlsb, ods). Select the destination library, match, and review manufacturer and package. Exact-ID confirmation preserves manual substitutions and only confirms candidates matching the original C-number. MPN-only rows require manual selection. All included rows must be confirmed and valid before **Add to this library**; unwanted rows can be excluded. Quantities and references are retained. Matching supports progress, cancellation and retry.

Library retains existing favorites and categories. The additional local search, notes and asset-management UI has been removed; existing favorites, settings and internal preview caches remain. LCSC remains the online search provider.

### Bind each library to Altium files

Right-click a library and choose **Bind library file…**. Select an existing or new `.SchLib`; the companion `.PcbLib` uses the same folder and stem. Each library has one binding, and duplicate bindings are rejected. Adding parts, importing a BOM or changing AD export settings schedules synchronization. Use **Sync library files** to retry manually.

Synchronization inherits colors, description fields, footprint renaming and 3D embedding settings. Stable C-number names prevent collisions; re-exporting updates the same component. Comment retains the MPN. Other existing components and extra streams are preserved. Removing favorites or unbinding does not remove external components.

Updates use file locking, paired replacement, interruption recovery and previous-generation backups named `.filename.jlc-backup`. Unsupported, corrupt or concurrently modified files are rejected. Use a library copy for your first trial.

The global persistent-folder option has been removed. Automatic synchronization uses only per-library bindings. Unbound parts export to the ordinary output folder. Ordinary exports never rebuild a bound library first.

Install the two file libraries once in Altium library management. Later exports update the same paths; Altium may need a refresh or library reopen. This version does not include an AD plugin that places parts into the active schematic.

AD binding is implemented in this release. [KiCad binding was assessed](docs/KICAD_BINDING_ASSESSMENT_ZH.md) but is not implemented; existing individual and merged KiCad exports remain available.

Shortcuts: `Ctrl+F` search, `Ctrl+O` import BOM, `Ctrl+Enter` export checklist.

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
lceda get C2040 --ad --ad-library-file ./MyParts.SchLib -o ./out
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
- Repository: [LZJ-I/JLC-Export-Workstation](https://github.com/LZJ-I/JLC-Export-Workstation)
- License: [CC BY-NC-4.0](https://creativecommons.org/licenses/by-nc/4.0/)

If it helps, please [Star](https://github.com/LZJ-I/JLC-Export-Workstation) the repo.
