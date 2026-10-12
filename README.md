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

The sidebar switches between **Search**, **Library**, **Export list**, **Sponsor**, **Settings**, and **About**. Library's **Import** dropdown offers **Import BOM… / Import AD library… / Import KiCad library…**. Drag the title bar to move, or double-click to maximize. Window size, splitters, and theme are remembered.

1. Type an MPN (e.g. `STM32F103C8T6`) or LCSC id (e.g. `C8734`), then press Enter or **Search**. Double-click a row to add it to the export list.
2. The search field and part preview start empty. Search for an MPN or LCSC code, then select a part. The header shows a larger part photo, resource status and actions. One full-size canvas has a remembered dropdown for **Schematic / 3D / Footprint**. Scroll to zoom, drag, and double-click to reset. The schematic reads actual AD export records, including designator, MPN and visible pin text, with the exported fonts, sizes and colors. The footprint shows actual pads, holes and outlines.
3. **Export…** opens one checklist for libraries, models and PDF. Confirm your selections to write files:
   - **Download STEP / OBJ**: 3D models
   - **Export Altium**: `.SchLib` / `.PcbLib` (EasyEDA JSON is kept too)
   - **Export KiCad**: `.kicad_sym` and `.pretty` (writes `.3dshapes` and a footprint model ref when a STEP exists)
   - **Export PADS**: `.c` schematic decal, `.d` PCB decal, `.p` part type (classic Logic / Layout; not PADS Professional / Xpedition Central Library)
   - **Datasheet**: PDF. LCSC often gives an HTML page; the app pulls the real PDF from it
   - **Open LCSC**: Chinese product page (`item.szlcsc.com`)

Unavailable assets are disabled or explained. Download asset has been folded into **Export…**; selecting only STEP, OBJ or PDF downloads those resources. **Details** and **Open LCSC** are separate buttons. Actions wrap in narrower windows.

The **Activity** button reserves its space in the status bar, so long output paths cannot obscure it. Open it to review search, export and sync activity for this session.

**Add to library** always asks for the destination, including when only one library exists. **Library** retains favorite categories (right-click to create / rename / import / export / delete). **Export list** is a queue you can import, export, or clear.

**Combine these parts into one library** appears only when a batch contains multiple parts that can be combined. It is absent from single-part export and Settings. Exports produce independent copies in the output folder; automatic sync and the Sync library menu update bindings.

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

Use **Library → Import → Import BOM…** for CSV / TSV / Excel (xlsx, xls, xlsb, ods). Select the destination library, match, and review manufacturer and package. Exact-ID confirmation preserves manual substitutions and only confirms candidates matching the original C-number. MPN-only rows require manual selection. All included rows must be confirmed and valid before **Add to this library**; unwanted rows can be excluded. Quantities and references are retained. Matching uses up to four workers, deduplicates queries, retains table order, and supports progress, cancellation and retry. Cancellation waits for in-flight requests and keeps completed results.

Library retains existing favorites and categories. The additional local search, notes and asset-management UI has been removed; existing favorites, settings and internal preview caches remain. LCSC remains the online search provider.

### Import existing AD / KiCad libraries

Choose `.SchLib` or `.kicad_sym` from **Library → Import**. The review dialog has a searchable checkbox list on the left and the original symbol preview on the right. Confirm to create a category, named after the source file or your chosen name. Cancel creates no records or resource copies.

Names, descriptions, properties, footprint links and all units are retained. Imported parts support unit selection, details and filtering. Internal snapshots leave source files unchanged; importing the same source path again updates existing records. Native AD Chinese names, mixed encodings and storage mappings are supported. KiCad inheritance dependencies are preserved.

Import does not search, download, or change any original symbols, footprints or 3D links. **Optional resources after import → Footprint / 3D model** are unchecked by default. Choose all imported parts or the previewed part. Opting in opens the resource dialog after import; candidates must still be searched and confirmed before downloading.

**Export native format…** and the library's selected-part / current-library exports retain original formats and available same-stem `.PcbLib`, `.pretty` and `.3dshapes` companions. This version does not convert imported formats, edit native symbols or preview companion footprint / 3D geometry. Imported parts use separate local identities, so a symbol named C2040 is never mistaken for an online part. Binding synchronization updates separately added LCSC parts in the category; reimport the source to refresh native parts.

### Optional footprint and 3D resources

Use **Resources → Supplement current part / Supplement entire library**, the part card, or its context menu. Select all or individual parts, choose footprints, 3D, or both, then search by complete model / C-number and review source, manufacturer, package and preview. **Confirm unique exact matches** requires an explicit action; searches never confirm or download automatically, and batch retries retain other confirmed choices.

Resources are saved separately from original `.PcbLib` / `.pretty` files. Original footprint alternatives, current choices, pin maps, model transforms and existing projects remain unchanged. New footprints have unique names. Model-only downloads save STEP and available OBJ previews without creating a footprint. Missing STEP downloads report failure while retaining existing resources. Searches and downloads use up to four workers with separate output directories. Native part previews show supplementary resources and their sources.

Native export offers **Include supplementary resources (retain original links)**. A `Resources/` directory contains independent footprints, models and source manifests. Downloading both resources embeds STEP in the new AD footprint and links the new KiCad footprint to the accompanying STEP; original symbols retain their original assignments. Replacing assignments remains an explicit EDA operation after checking compatibility. Resource search uses LCSC parts, rather than a general multi-provider footprint / 3D search.

### Library exports and binding synchronization

**Library → Export…** offers **Export selected part… / Export current library…** without first using the export queue. Switching libraries selects a member of the current category and clears the preview for empty libraries. Export confirmation freezes the selection and shows its identifiers, count and actual destination. Each current-library export contains only that category in its own `Library-name-id` folder, with optional merging for multiple parts, keeping earlier exports separate.

The **Sync library** dropdown updates all bindings in the current library, AD only, KiCad only, or every library. The category context menu provides the same controls. **Bind library file… → AD / KiCad** selects an existing or new target. Each category can bind one target per format; duplicate file bindings are rejected.

On Windows, **Sync library → Connect to AD… → Install native plugin** builds against this machine's matching AD SDK. Close AD to install, then start AD for an automatic background connection without a script window. Missing or incompatible SDK interfaces stop installation or synchronization explicitly. The connection dialog provides uninstall, file mode and script compatibility controls. See [native plugin setup and verification limits](integrations/altium/plugin/README_ZH.md).

AD appends, saves, reopens and reads back components; the workstation also verifies saved symbols, footprints and embedded STEP. Existing components, models and links are retained, and unsaved target libraries stop synchronization. New names use `model_Cnumber`. While AD is closed, pending library scopes wait for connection and survive a workstation restart; bindings are revalidated before use. Failed or uncertain writes require manual retry and are never replayed automatically or replaced by file writes. File mode requires AD to be closed. Linux and KiCad retain their file workflow. [Script compatibility instructions](integrations/altium/README_ZH.md).

- **AD:** choose `.SchLib` with its same-stem `.PcbLib`. File sync uses stable C-number names for updates while Comment retains the MPN; the editor connection appends and preserves existing components as described above. Unrelated components, fonts, models and extra streams are retained.
- **KiCad:** choose `.kicad_sym` with same-stem `.pretty/` and `.3dshapes/` folders. Stable C-number identifiers connect symbols, footprints and models; Value retains the MPN. Uses **KiCad 9 format** and upgrades older symbol-library versions while preserving untouched symbols and unknown fields.

Adding parts, importing BOMs and changing export settings automatically schedule synchronization. Removing parts or unbinding preserves external components. Ordinary exports check for path collisions with bound files.

Binding validation runs in the background. **Bind and sync** and manual synchronization immediately open **Library sync progress**, showing the current part, target files, completed count and individual errors. **Run in background** keeps the interface available; reopen progress from the status bar or Sync library menu. **Stop remaining parts** lets the current part finish before stopping, preserving completed writes. Failed or stopped tasks can be retried from the progress window.

Both formats use file locking, recoverable transactions and `.filename.jlc-backup` previous-generation backups. Windows local folders and WSL shared paths have been tested for synchronization and retries. Lock contention is reported separately from path or permission errors; failed replacement preserves the original file. Keep KiCad's `.filename.jlc-meta.json` ownership record with the library. Unowned name conflicts and manually edited managed components are rejected without overwriting them; use a new target after backing up if needed. Failures are shown and can be retried from the menu.

Install AD's two libraries once. In KiCad, add the symbol file and `.pretty` library once using the file stem as the nickname; model references use the binding's absolute path. Refresh or reopen libraries after synchronization as needed. The app does not edit KiCad global/project library tables or place parts in an active schematic.

See [KiCad binding implementation and verification](docs/KICAD_BINDING_ASSESSMENT_ZH.md). The legacy global fixed AD-library option has been removed.

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
lceda get C2040 --kicad --kicad-library-file ./MyParts.kicad_sym -o ./out
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
