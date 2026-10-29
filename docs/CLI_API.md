# CLI and automation API

[English README](../README.md) · [中文 README](../README_ZH.md) · [完整 Agent 示例](AGENT_WORKFLOW.md)

This document is the contract for invoking `lceda` as a subprocess. The BOM-plan, JSON and inspection commands are included in release v0.10.0 and later. There is no HTTP server or remote-control port. The same executable provides the desktop and CLI.

## Invocation and discovery

```bash
lceda --help
lceda --version
lceda bom --help
lceda bom confirm --help
lceda bom export --help
```

```text
lceda [--lang en|zh] [--profile DIRECTORY] [--json] COMMAND
  gui
  search KEYWORD [--limit N]
  get KEYWORD [--index N] [EXPORT OPTIONS] [BINDING OPTIONS]
  batch FILE [EXPORT OPTIONS] [--merge [--name STEM]]
  bom import FILE --plan PLAN [--force]
  bom match PLAN
  bom show PLAN
  bom confirm PLAN [--exact] [--select ROW=C_NUMBER] [--exclude ROWS] [--include ROWS]
  bom export PLAN [EXPORT OPTIONS] [--merge [--name STEM]]
  library inspect FILE
```

Global options may appear before or after the subcommand. Paths are resolved against the caller's current working directory. Pass paths as separate process arguments, including paths containing spaces; do not build an interpolated shell command. CLI operations are synchronous and do not require a desktop display. `gui` and no command open the desktop; they reject `--json`.

| Global option | Default | Contract |
| --- | --- | --- |
| `--json` | Off | One JSON object on stdout; diagnostics/warnings go to stderr |
| `--profile DIRECTORY` | OS application config directory | Use this profile's preferences and local snapshots, including when opening the GUI |
| `--lang en\|zh` | Saved setting, then system | GUI language; JSON keys remain English. Upstream and validation error text may contain Chinese |
| `--help`, `--version` | — | Human-readable text, exit 0, even when `--json` is present |

For predictable settings, use the same isolated profile throughout a job, for example `lceda --profile ./work/profile --json ...`. No normal CLI command updates GUI favorites or creates library bindings. `get` changes a bound file only when its corresponding `--*-library-file` argument is explicitly supplied.

## JSON envelope and exit codes

Every `--json` command response, including argument errors, follows this envelope:

```json
{
  "schema_version": 1,
  "command": "bom.show",
  "ok": true,
  "data": {},
  "error": null
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Response schema, currently `1`; reject unsupported versions |
| `command` | string | `search`, `get`, `batch`, `bom.import`, `bom.match`, `bom.show`, `bom.confirm`, `bom.export`, `library.inspect`; parse failures use `arguments` |
| `ok` | boolean | True exactly when the command returns exit 0 |
| `data` | object or null | Command result; retained on partial completion so an agent can recover |
| `error` | object or null | `{ "code": "incomplete" or "command_failed", "message": "..." }` |

| Exit | Meaning | Agent action |
| --- | --- | --- |
| `0` | Command completed | Inspect command-specific state. `bom import/match/show` can complete while `data.ready` is false |
| `1` | Partial result or outstanding confirmation | Read `data.rows` / `data.results`; do not claim the whole job succeeded |
| `2` | Invalid arguments, input, plan state, or command failure | Read `error.message`; fix the cause before retrying |

A successful search with no results has `ok: true`, `data.total: 0`, `data.items: []`. That is a completed search, not permission to select a substitute. Do not parse human output or branch on translated error-message substrings. Check exit code, `ok`, readiness and structured row fields. External termination or a process-launch failure may produce no JSON at all; distinguish that from an application response.

## Search and single-part selection

```bash
lceda search STM32F103C8T6 --limit 10 --json
lceda get C2913202 --ad --kicad -o ./out/part --json
# Only after reviewing the current search results:
lceda get STM32F103C8T6 --index 2 --ad -o ./out/reviewed --json
```

`search` returns `{ "query": string, "total": integer, "items": Part[] }`. `--limit` defaults to 20 and limits returned items, not the reported total; 0 returns just the count.

Each `Part` has these fields:

```json
{
  "index": 1,
  "lcsc": "C2913202",
  "mpn": "ESP32-S3-WROOM-1-N16R8",
  "manufacturer": "ESPRESSIF(乐鑫)",
  "package": "",
  "description": "supplier description",
  "available": { "symbol": true, "footprint": true, "model3d": true, "datasheet": true }
}
```

`lcsc` is a string or null; other text fields are strings. Availability indicates source metadata, not successful download or EDA verification. An empty package means it was not supplied, not that packages match. Supplier text is not translated by `--lang`.

Selection rules:

- C-number lookup is case-insensitive and must find that exact supplier ID. `--index` cannot override this check.
- Without `--index`, an MPN must have exactly one exact match against the returned display title or title. Multiple variants or no exact match produce exit 2.
- `--index` is a positive, 1-based current search index. Search ordering can change between requests, so agents should normally use the reviewed `lcsc` value instead.
- `batch` applies the same exact selection rules, deduplicates repeated queries/IDs and reports failures alongside successful parts. Ambiguous entries must be replaced by reviewed C-numbers or handled with a BOM plan.

## Export options

Shared by `get`, `batch` and `bom export`:

| Option | Meaning / default |
| --- | --- |
| `--ad` | Altium symbol and footprint libraries |
| `--kicad` | KiCad symbol and footprint libraries |
| `--pads` | PADS Logic/Layout `.c`, `.d`, `.p` per part |
| `--step` | Download STEP explicitly |
| `--obj` | Download OBJ and MTL explicitly |
| `--datasheet` | Download the PDF when available |
| `--source` | Save EasyEDA symbol / footprint JSON; also included with library exports |
| `-o, --output DIRECTORY` | Output directory; defaults to `out` relative to the caller |
| `--no-ad-3d` | Disable STEP embedding in AD footprints; embedding otherwise defaults on |
| `--no-kicad-3d` | Disable STEP attachment in KiCad footprints; attachment otherwise defaults on |
| `--rename-footprint` | Rename the source footprint using the part model; otherwise keep its original name |
| `--pads-grid MILS` | Whole number 1–1000; default is the profile's setting, 10 mil in a fresh profile |
| `--sch-color altium\|easyeda\|mono` | Altium symbol colors; otherwise inherit the profile, including a custom palette |
| `--force` | Refresh cached downloads/source files; this is not an overwrite-protection switch |

When **no format/resource flag** is supplied, the legacy default is `--step --ad --kicad` plus source JSON. Agents should pass the intended flags explicitly. Description fields are inherited from the selected profile; a fresh profile uses the application's default field set.

AD / KiCad implicit 3D attachment is best-effort for independent exports: missing or failed STEP may yield a library without 3D and a stderr diagnostic. Use explicit `--step` if the model must be part of a successful output, and inspect the `step` path in each result. A successful library write is not proof of correctness in an EDA editor.

`batch` and `bom export` also support:

| Option | Meaning |
| --- | --- |
| `--merge` | Generate one combined AD and/or KiCad library at the output root; requires at least one of those formats |
| `--name STEM` | Combined filename stem; requires `--merge`, defaults to `lceda`; no separators or reserved filename characters |

```bash
lceda batch ./parts.txt --ad --kicad --merge --name BoardParts -o ./out/board --json
lceda bom export ./work/board.plan.json --pads --pads-grid 100 -o ./out/pads --json
```

With both formats enabled, merged outputs are `BoardParts.SchLib`, `BoardParts.PcbLib`, `BoardParts.kicad_sym`, `BoardParts.pretty/` and, when available, `BoardParts.3dshapes/`. Model/source/PADS files remain in part directories. Without merging, part directories use `MPN_LCSC_Manufacturer` and internal files use the model name. Use the returned paths rather than reconstructing names.

**Output behavior:** ordinary library exports regenerate their files, even without `--force`; merging rebuilds a library rather than appending to it. Use a fresh job-specific directory for an independent deliverable. Do not direct these exports at a hand-maintained library. Failed batch exports can leave completed part files and a combined library containing only successful parts. Exit 1 and the result list identify an incomplete delivery. Retrying can reuse cached resources, but it is not an atomic rollback of the previous batch.

Only `get` accepts incremental file-binding options:

| Option | Requires | Behavior |
| --- | --- | --- |
| `--ad-library-file FILE.SchLib` | `--ad` | Update that AD file and its same-stem `.PcbLib`; close AD first; experimental |
| `--kicad-library-file FILE.kicad_sym` | `--kicad` | Update that KiCad library and its companion footprint/model directories |

These invoke file synchronization. They do not install/connect the AD plugin, create GUI category bindings or add a library to KiCad's global tables. See the [desktop guide](GUI_GUIDE.md) for editor synchronization and backups.

## Export response

`get`, `batch` and `bom export` return:

```json
{
  "output": "./out/board",
  "merged": true,
  "name": "BoardParts",
  "total": 2,
  "succeeded": 2,
  "failed": 0,
  "results": [
    {
      "query": "C25804",
      "item": { "lcsc": "C25804", "mpn": "...", "index": 1 },
      "status": "ok",
      "error": null,
      "missing": [],
      "paths": { "schlib": "./out/board/BoardParts.SchLib", "pcblib": "./out/board/BoardParts.PcbLib" }
    }
  ]
}
```

The example abbreviates `item`, `paths` and the second result. `item` uses the full `Part` schema, or is null if selection failed. `status` is `ok`, `partial` (some requested outputs are missing) or `error`; `error` is a string or null. `failed` counts both partial and error results. An `error` result has `paths: null`; files may still have been written before the failure, so a null report does not prove the directory is empty.

`paths` contains all these keys, each string-or-null:

| Group | Keys |
| --- | --- |
| Location | `folder` |
| Models / documents | `step`, `obj`, `mtl`, `datasheet` |
| Sources | `symbol_json`, `footprint_json` |
| Altium | `schlib`, `pcblib`, `ad_sync` (sync message, not a path) |
| KiCad | `kicad_sym`, `kicad_mod`, `kicad_pretty` |
| PADS | `pads_c`, `pads_d`, `pads_p` |

Merged rows share the combined library paths. For a merged KiCad library, `kicad_pretty` is the footprint collection and each row's `kicad_mod` identifies that part's footprint file. A merged library path is returned for a part only when that part contributed the corresponding symbol or footprint. Embedded/attached STEP does not necessarily have a separate `step` path unless `--step` was requested.

`bom export` additionally returns `plan` (path) and `bom_rows` (included source rows before deduplication). Quantities and designators remain in the plan; exported libraries contain component definitions, not placed schematic instances.

## BOM workflow

The plan is a local review document, not a GUI favorite category or a bound EDA library. It retains the parsed BOM, raw candidate snapshots and explicit choices between invocations. Export uses those confirmed snapshots rather than rerunning a keyword search. Resource downloads can still need the network.

### 1. Import

```bash
lceda bom import ./bom.xlsx --plan ./work/board.plan.json --json
```

- Supported: CSV, TSV, TXT, XLSX, XLS, XLSB and ODS. CSV/TSV/TXT also recognize comma, tab or semicolon delimiters, UTF-8, UTF-16 and GBK text.
- A recognizable LCSC or MPN header is required within the first 50 rows. Examples: `LCSC Part Number` / `立创编号`, `MPN` / `型号`, `Manufacturer` / `厂商`, `Package` / `封装`, `Quantity` / `数量`, `Designator` / `位号`.
- For workbooks, the first worksheet with recognized BOM headers is used. Limits: 32 MiB source, 10,000 BOM rows; workbook parsing also bounds cells/expanded data.
- Blank rows are ignored. Rows missing both ID and model are skipped with warnings. Invalid IDs or quantities are retained with `validation_error: true` and must be corrected in the source or excluded.
- Missing quantity is inferred from designators (minimum 1); an explicit quantity must be a positive integer. Duplicate parts retain separate source rows and quantities in the plan.
- No network request and no automatic confirmation. Import returns 1 for source-row validation errors but still saves the review plan.
- Existing plans are protected. `--force` explicitly replaces the entire plan and discards prior matches/choices. The source file cannot be the plan destination.

### 2. Match or retry

```bash
lceda bom match ./work/board.plan.json --json
```

Search included, valid, unconfirmed rows. Queries are deduplicated, run with up to four workers and use the same matching logic as the GUI. Exact-ID snapshots may be reused from the selected local profile. Confirmed rows and manual substitutions are retained. Matching never confirms a row.

The updated plan is saved even when individual searches fail. Retry the same command for transient failures; completed confirmations remain unchanged. Source validation errors require a source correction/reimport, not repeated network retries. Exit 1 indicates at least one included row with an error; otherwise 0, even if confirmation is still needed.

### 3. Inspect and confirm

```bash
lceda bom show ./work/board.plan.json --json
lceda bom confirm ./work/board.plan.json --exact --json
lceda bom confirm ./work/board.plan.json --select 4=C25804 --select 5=C14663 --json
lceda bom confirm ./work/board.plan.json --exclude 7,8 --json
lceda bom confirm ./work/board.plan.json --include 7 --json
```

- `ROW` is the source-table row number **including the header**, from `data.rows[].line`; it is not the JSON-array position.
- `--exact` confirms only original nonempty C-numbers with exact candidates. It does not confirm MPN-only rows and does not overwrite a manual substitution.
- `--select ROW=C_NUMBER` requires that ID to be among the row's saved candidates. It explicitly confirms that choice, including a reviewed substitute for the original C-number. A source validation error cannot be overridden by selecting a candidate.
- `--exclude` removes rows from the export requirement; `--include` restores them. Existing choices are retained. Use `--include ROW --select ROW=ID` together when needed.
- At least one action is required. Unknown rows, absent candidates, duplicate selections and conflicting include/exclude/select actions fail before the plan is saved.
- Actions within one command: include/exclude, explicit selections, then exact confirmation. Successful changes are saved even if other included rows still need review; that returns exit 1.
- `show` is read-only and returns 0 when the plan is readable; use `data.ready` for export readiness.

### BOM response fields

All import/match/show/confirm commands return the same `data` shape:

```json
{
  "plan": "./work/board.plan.json",
  "source": { "path": "/absolute/path/bom.csv", "name": "bom.csv" },
  "ready": false,
  "summary": { "total": 2, "included": 2, "excluded": 0, "confirmed": 1, "unresolved": 1, "errors": 0 },
  "warnings": [],
  "rows": [
    {
      "line": 4,
      "input": { "lcsc": "", "mpn": "...", "manufacturer": "...", "package": "...", "quantity": 2, "designators": "R1,R2" },
      "query": "...",
      "included": true,
      "confirmed": false,
      "selected_lcsc": null,
      "error": null,
      "validation_error": false,
      "candidates": []
    }
  ]
}
```

The example abbreviates rows. `candidates` contains full `Part` objects. `selected_lcsc` may be an unconfirmed suggestion: **always inspect `confirmed`**, not just selection presence. `warnings` and row `error` are descriptive strings; `validation_error` distinguishes a source-table issue from a lookup issue. `unresolved` counts included rows that are not confirmed and error-free.

`ready` is true only when at least one row is included and every included row is confirmed, has a valid selected candidate and no error. Repeated selected IDs are deduplicated only at export; the plan retains every row.

Plan files have their own `schema_version: 1`, `source_path` and serialized `bom` fields. They include raw supplier snapshots and are **not the same schema as a response**. Use CLI commands for edits. Keep the plan if you need to resume or audit the job; it contains source paths and BOM data. Unknown schema versions are rejected. Maximum saved plan size is 64 MiB.

Writes use an atomic file replacement and a sibling `PLAN.lock` for cooperating CLI commands. Do not run multiple mutations on one plan concurrently. If a process is killed, verify it has stopped before removing its leftover lock. `show` and `export` read a complete saved snapshot and do not modify the plan. Editing the original BOM later does not alter an existing plan; reimport to adopt source corrections.

### 4. Export

```bash
lceda bom export ./work/board.plan.json --ad --kicad --merge --name BoardParts -o ./out/board --json
```

An unready plan fails before creating export files. Once ready, each distinct selected ID is exported using the existing core writers. Resource download/format failures produce per-part results and exit 1. Export does not alter the plan, source BOM, GUI favorites or configured bindings. It does regenerate files under the chosen output directory as described above.

## Inspect the result

```bash
lceda library inspect ./out/board/BoardParts.SchLib --json
lceda library inspect ./out/board/BoardParts.kicad_sym --json
```

Read-only, offline inspection accepts `.SchLib` and `.kicad_sym` (case-insensitive). Unsupported types, unreadable or empty libraries return exit 2. Response:

```json
{
  "file": "./out/board/BoardParts.SchLib",
  "format": "Ad",
  "total": 2,
  "parts": [
    { "key": "actual storage key", "name": "part name", "description": "...", "properties": {}, "footprints": ["footprint name"], "pins": 2, "units": 1 }
  ]
}
```

`format` is `Ad` or `KiCad`. `properties` maps strings to strings; `footprints` is a string array. This verifies the file can be read by the project's library parser. It does not open Altium/KiCad, check PCB placement, validate a PADS import or prove all downloaded models are correct. PCB-library-only and PADS inspection are not currently exposed by this command.

## Integration boundaries

Use CLI subprocesses for application automation. The Rust `lceda-core` crate additionally exposes `LcedaClient::search`, `export::export_items`, `ExportRequest`, `native_library::inspect` and the format writers for in-process integrations. `export_items` accepts exact `SearchItem` snapshots; it does not perform a keyword selection. BOM parsing/review currently lives in the application crate and is exposed through the CLI plan workflow.

The CLI does not currently expose GUI favorite/category editing, native-library import into favorites, optional native-resource supplementation, AD plugin management or a running-window control API. Use the [desktop guide](GUI_GUIDE.md) for those workflows. A future transport can wrap the documented workflow, but there is no HTTP API to call in this version.
