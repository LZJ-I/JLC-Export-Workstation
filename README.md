**English** · [中文](README_ZH.md)

# JLC Export Workstation

Search LCSC parts, inspect symbols, footprints and 3D models, and export libraries for **Altium, KiCad and PADS**. A native Rust desktop app with a CLI for repeatable BOM workflows.

[Download v0.10.0](https://github.com/LZJ-I/JLC-Export-Workstation/releases/tag/v0.10.0) · [Desktop guide](docs/GUI_GUIDE.md) · [CLI reference](docs/CLI_API.md) · [Agent automation](#agent-automation)

![Search, 3D preview and library export](docs/demo.gif)

| Task | What you get |
| --- | --- |
| Find and inspect a part | LCSC/MPN search, schematic, footprint and interactive 3D preview |
| Build an EDA library | Altium `.SchLib` / `.PcbLib`, KiCad symbols / footprints, PADS `.c` / `.d` / `.p` |
| Process a BOM | CSV / TSV / Excel import, candidate review, exact-ID confirmation and combined AD / KiCad export |
| Keep resources together | STEP / OBJ, optional footprint 3D attachment, datasheets and EasyEDA source JSON |

## Install

Download an archive from [Releases](https://github.com/LZJ-I/JLC-Export-Workstation/releases), unzip it and run the executable. No Electron or additional desktop runtime is required.

| Platform | Release archive | Executable |
| --- | --- | --- |
| Windows x64 | `lceda-v*-x86_64-pc-windows-msvc.zip` | `lceda.exe` |
| Linux x64 | `lceda-v*-x86_64-unknown-linux-gnu.zip` | `lceda` |
| macOS Apple Silicon | `lceda-v*-aarch64-apple-darwin.zip` | `lceda` |
| macOS Intel | `lceda-v*-x86_64-apple-darwin.zip` | `lceda` |

Or build the current source with [Rust](https://rustup.rs/):

```bash
cargo build --release -p lceda
# executable: target/release/lceda (Windows: lceda.exe)
```

The in-app updater verifies the package and retains a backup. Replacing the extracted release files also updates the app.

## Desktop quick start

1. Search by LCSC ID or MPN, then select a result. Switch the preview between **Schematic**, **Footprint** and **3D**.
2. Click **Export…**, select formats and resources, and confirm. The default output is `Downloads/lceda-out`; change it in **Settings**.
3. For multiple parts, use **Library → Import → Import BOM…**, review the matches and export the library. You can also import existing AD / KiCad libraries.

**Settings → Export** controls Description fields, schematic colors, 3D attachment and the PADS connection grid. **Activity** shows search, export and synchronization results. Shortcuts: `Ctrl+F` search, `Ctrl+O` import BOM, `Ctrl+Enter` export.

The [desktop guide](docs/GUI_GUIDE.md) covers library management, native imports, optional resources and binding synchronization. **AD synchronization is experimental**; use a library copy when evaluating it. Ordinary exports create independent library copies.

## CLI

No arguments opens the GUI. Commands run without opening a window.

**Available in v0.10.0 and later.** The release executable includes these CLI commands; the ZIP also contains the interface documentation and sample BOM.

```bash
lceda search C2913202 --json
lceda get C2913202 --ad --kicad --pads --step -o ./out/part --json
lceda batch parts.txt --ad --kicad --merge --name BoardParts -o ./out/board --json
lceda library inspect ./out/board/BoardParts.SchLib --json
```

`parts.txt` contains one LCSC ID or MPN per line; `#` and `//` introduce comment lines. Exact C-numbers never fall back to a different part. Ambiguous MPNs require an explicit choice. `--merge` combines AD / KiCad libraries; PADS remains per-part.

Use `lceda --help` or `lceda bom export --help`. The [CLI contract](docs/CLI_API.md) documents every command, flag, JSON field, exit code, retry behavior and write destination.

## Agent automation

Give your agent this prompt, adjusting the file, output directory and format:

> Use the `lceda` CLI to import `./bom.csv`, check the part numbers, manufacturers and packages, and export the confirmed parts into one Altium library named `BoardParts` under `./out/board`; automatically confirm only original exact LCSC IDs, ask me to review other candidates, retain quantities and designators in the BOM plan, then read back the exported library and report its files, part count and any failures without modifying my existing libraries.

The agent can carry out the workflow through these commands:

```bash
# 1. Parse the BOM locally; retain source rows and quantities in a review plan.
lceda bom import ./bom.csv --plan ./work/board.plan.json --json

# 2. Search candidates, then explicitly confirm original exact C-number matches.
lceda bom match ./work/board.plan.json --json
lceda bom confirm ./work/board.plan.json --exact --json

# 3. Inspect unresolved rows; confirm a reviewed candidate by source row and ID.
lceda bom show ./work/board.plan.json --json
# Example only: lceda bom confirm ./work/board.plan.json --select 4=C25804 --json

# 4. Once data.ready is true, export one library and read it back.
lceda bom export ./work/board.plan.json --ad --merge --name BoardParts -o ./out/board --json
lceda library inspect ./out/board/BoardParts.SchLib --json
```

For KiCad, use `--kicad` and inspect `BoardParts.kicad_sym`. You can request both formats together. The plan is separate from the desktop's saved libraries. Repeated parts export once; original BOM quantities and designators remain in the plan.

`--json` writes one response to stdout; diagnostics use stderr. Exit **0** means the command completed, **1** means partial results or outstanding review, and **2** means the command failed. An export requires every included row to be confirmed. Check `data.ready` before exporting and each export result afterward.

[Full agent walkthrough and Python subprocess example](docs/AGENT_WORKFLOW.md) · [JSON/API reference](docs/CLI_API.md)

## Export formats

| Format | Files | Notes |
| --- | --- | --- |
| Altium | `.SchLib`, `.PcbLib` | Optional embedded STEP; Designator = prefix, Comment = MPN |
| KiCad | `.kicad_sym`, `.pretty/`, optional `.3dshapes/` | KiCad 9 format; add the exported libraries in KiCad |
| PADS | `.c`, `.d`, `.p` | Classic Logic / Layout, not Professional / Xpedition Central Library |
| Models / documents | `.step`, `.obj`, `.mtl`, `.pdf` | Depends on the selected part's available assets |
| Source | EasyEDA JSON | Conversion input retained for inspection and regeneration |

Check exported libraries in your EDA editor before using them. LCSC / EasyEDA data comes from unofficial interfaces and can change. [Verification scope](docs/VERIFICATION_0.9.0_ZH.md) · [PADS grid](docs/PADS_GRID_ZH.md)

## Development

```bash
cargo test --workspace
```

## Project

By [LZJ-I](https://github.com/LZJ-I) · [CC BY-NC 4.0](LICENSE) · [Sponsor](https://github.com/LZJ-I/sponsor)

If this tool helps, a [Star](https://github.com/LZJ-I/JLC-Export-Workstation) is appreciated.
