//! Persistent BOM review plans. The desktop and CLI use the same row validation and matching.
use super::{
    options::{ExportOptions, MergeOptions},
    output::{self, Outcome},
};
use crate::{bom::BomDocument, library::Library};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use lceda_core::{client::LcedaClient, export, models::SearchItem};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const PLAN_VERSION: u32 = 1;
const MAX_PLAN_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Subcommand)]
pub(super) enum Command {
    /// Parse CSV / TSV / XLSX / XLS / XLSB / ODS locally and create a review plan.
    Import {
        file: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        /// Replace an existing plan, discarding its previous selections.
        #[arg(long)]
        force: bool,
    },
    /// Search candidates for included, unconfirmed rows; preserve prior confirmations.
    Match { plan: PathBuf },
    /// Inspect source rows, candidates, confirmations and export readiness.
    Show { plan: PathBuf },
    /// Confirm exact IDs, select candidates by source row, or include/exclude rows.
    Confirm {
        plan: PathBuf,
        /// Confirm remaining original C-number matches only; never confirm MPN-only rows.
        #[arg(long)]
        exact: bool,
        /// Explicit candidate: SOURCE_ROW=C_NUMBER. Repeat for multiple rows.
        #[arg(long, value_name = "ROW=C_NUMBER")]
        select: Vec<Selection>,
        /// Exclude source rows from export (comma separated or repeated).
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<usize>,
        /// Include previously excluded source rows (comma separated or repeated).
        #[arg(long, value_delimiter = ',')]
        include: Vec<usize>,
    },
    /// Export all confirmed included parts; deduplicate IDs and retain BOM metadata in the plan.
    Export {
        plan: PathBuf,
        #[command(flatten)]
        export: ExportOptions,
        #[command(flatten)]
        merge: MergeOptions,
    },
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Import { .. } => "bom.import",
            Self::Match { .. } => "bom.match",
            Self::Show { .. } => "bom.show",
            Self::Confirm { .. } => "bom.confirm",
            Self::Export { .. } => "bom.export",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Selection {
    line: usize,
    lcsc: String,
}

impl std::str::FromStr for Selection {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        let (line, lcsc) = value
            .split_once('=')
            .ok_or("Use SOURCE_ROW=C_NUMBER, for example 4=C25804")?;
        let line: usize = line
            .parse()
            .map_err(|_| "Source row must be a positive integer")?;
        let lcsc = lcsc.to_ascii_uppercase();
        if line == 0 || !lceda_core::models::looks_like_lcsc_token(&lcsc) {
            return Err(
                "Use a positive source row and exact C-number, for example 4=C25804".into(),
            );
        }
        Ok(Self { line, lcsc })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    source_path: PathBuf,
    bom: BomDocument,
}

impl Plan {
    fn read(path: &Path) -> Result<Self> {
        let file = fs::File::open(path).with_context(|| format!("open plan {}", path.display()))?;
        if file.metadata()?.len() > MAX_PLAN_BYTES {
            bail!("BOM plan exceeds 64 MiB");
        }
        let plan: Self = serde_json::from_reader(file).context("invalid BOM plan JSON")?;
        if plan.schema_version != PLAN_VERSION {
            bail!(
                "Unsupported BOM plan version {}; expected {PLAN_VERSION}",
                plan.schema_version
            );
        }
        if plan.bom.rows.is_empty() || plan.bom.rows.len() > 10_000 {
            bail!("Invalid BOM plan row count");
        }
        let mut lines = HashSet::new();
        for row in &plan.bom.rows {
            if row.line == 0 || !lines.insert(row.line) {
                bail!("Invalid or duplicate source row {}", row.line);
            }
            if row
                .selected
                .is_some_and(|index| index >= row.candidates.len())
            {
                bail!("Invalid candidate selection on source row {}", row.line);
            }
            if row.confirmed
                && (row.chosen().and_then(SearchItem::lcsc_id).is_none()
                    || row.validation_error
                    || row.quantity == 0)
            {
                bail!("Invalid confirmation on source row {}", row.line);
            }
        }
        Ok(plan)
    }

    fn save(&self, path: &Path, replace: bool) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() as u64 > MAX_PLAN_BYTES {
            bail!("BOM plan exceeds 64 MiB; split the source BOM");
        }
        let parent = parent(path);
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        if replace {
            temporary.persist(path)?;
        } else {
            temporary.persist_noclobber(path)?;
        }
        Ok(())
    }

    fn selected_items(&self) -> Result<Vec<SearchItem>> {
        self.bom.confirmed_parts().map_err(anyhow::Error::msg)?;
        let mut seen = HashSet::new();
        Ok(self
            .bom
            .rows
            .iter()
            .filter(|row| row.included)
            .filter_map(|row| row.chosen())
            .filter(|item| seen.insert(item.lcsc_id()))
            .cloned()
            .collect())
    }

    fn outcome(&self, path: &Path, code: i32) -> Outcome {
        let rows = &self.bom.rows;
        let included = rows.iter().filter(|row| row.included).count();
        let confirmed = rows
            .iter()
            .filter(|row| row.included && row.confirmed && row.error.is_none())
            .count();
        let errors = rows
            .iter()
            .filter(|row| row.included && row.error.is_some())
            .count();
        let ready = self.selected_items().is_ok();
        let mut text = format!("Plan: {}\n{} rows, {included} included, {confirmed} confirmed, {errors} errors. Ready: {ready}", path.display(), rows.len());
        for row in rows {
            let status = if !row.included {
                "excluded"
            } else if row.error.is_some() {
                "error"
            } else if row.confirmed {
                "confirmed"
            } else {
                "needs review"
            };
            let selected = row
                .chosen()
                .and_then(SearchItem::lcsc_id)
                .unwrap_or_default();
            text.push_str(&format!(
                "\n  row {}: {} -> {} [{status}]",
                row.line,
                row.query(),
                selected
            ));
            if let Some(error) = &row.error {
                text.push_str(&format!(" {error}"));
            }
            if !row.confirmed && row.included {
                for candidate in &row.candidates {
                    text.push_str(&format!(
                        "\n    {} | {} | {} | {}",
                        candidate.lcsc_id().unwrap_or_default(),
                        candidate.name(),
                        candidate.manufacturer_label(),
                        crate::library::package_from_item(candidate)
                    ));
                }
            }
        }
        Outcome {
            code,
            text,
            data: json!({
                "plan": path, "source": {"path": self.source_path, "name": self.bom.source}, "ready": ready,
                "summary": {"total": rows.len(), "included": included, "excluded": rows.len() - included,
                    "confirmed": confirmed, "unresolved": included - confirmed, "errors": errors},
                "warnings": self.bom.warnings,
                "rows": rows.iter().map(|row| json!({
                    "line": row.line, "input": {"lcsc": row.lcsc, "mpn": row.model, "manufacturer": row.manufacturer,
                        "package": row.package, "quantity": row.quantity, "designators": row.designators},
                    "query": row.query(), "included": row.included, "confirmed": row.confirmed,
                    "selected_lcsc": row.chosen().and_then(SearchItem::lcsc_id),
                    "error": row.error, "validation_error": row.validation_error,
                    "candidates": row.candidates.iter().map(output::item).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }),
        }
    }
}

// Serialize CLI read/modify/write operations without locking a user's desktop library.
// An interrupted process leaves a visible lock; recovery is explicit, never a guessed timeout.
struct PlanLock(PathBuf);
impl PlanLock {
    fn acquire(path: &Path) -> Result<Self> {
        fs::create_dir_all(parent(path))?;
        let mut name = path.as_os_str().to_os_string();
        name.push(".lock");
        let path = PathBuf::from(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "Cannot lock {}; another command may be using this plan",
                    path.display()
                )
            })?;
        let guard = Self(path);
        writeln!(file, "pid={}", std::process::id())?;
        Ok(guard)
    }
}
impl Drop for PlanLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub(super) fn execute(command: Command) -> Result<Outcome> {
    match command {
        Command::Import { file, plan, force } => {
            let source_path = fs::canonicalize(&file).context("locate source BOM")?;
            let plan = std::path::absolute(plan)?;
            if source_path == fs::canonicalize(&plan).unwrap_or_else(|_| plan.clone()) {
                bail!("Plan path must differ from the source BOM");
            }
            let _lock = PlanLock::acquire(&plan)?;
            if plan.exists() && !force {
                bail!("Plan already exists; use bom show or --force to replace it");
            }
            let bom = BomDocument::load(&file).map_err(anyhow::Error::msg)?;
            let state = Plan {
                schema_version: PLAN_VERSION,
                source_path,
                bom,
            };
            state.save(&plan, force)?;
            let code = i32::from(state.bom.rows.iter().any(|row| row.validation_error));
            Ok(state.outcome(&plan, code))
        }
        Command::Show { plan } => Ok(Plan::read(&plan)?.outcome(&plan, 0)),
        Command::Match { plan } => {
            let _lock = PlanLock::acquire(&plan)?;
            let mut state = Plan::read(&plan)?;
            state.bom.resolve_with_control(
                &LcedaClient::new(),
                &Library::load(),
                &AtomicBool::new(false),
                |_, _| {},
            );
            state.save(&plan, true)?;
            let code = i32::from(
                state
                    .bom
                    .rows
                    .iter()
                    .any(|row| row.included && row.error.is_some()),
            );
            Ok(state.outcome(&plan, code))
        }
        Command::Confirm {
            plan,
            exact,
            select,
            exclude,
            include,
        } => {
            let _lock = PlanLock::acquire(&plan)?;
            let mut state = Plan::read(&plan)?;
            confirm(&mut state.bom, exact, &select, &exclude, &include)?;
            state.save(&plan, true)?;
            let code = i32::from(state.selected_items().is_err());
            Ok(state.outcome(&plan, code))
        }
        Command::Export {
            plan,
            export: options,
            merge,
        } => {
            let state = Plan::read(&plan)?;
            let items = state
                .selected_items()
                .context("BOM is not ready; use bom show / confirm before exporting")?;
            let mut req = options.request();
            merge.apply(&mut req)?;
            let rows = export::export_items(&LcedaClient::new(), &items, &req)
                .into_iter()
                .map(|(item, result)| {
                    output::export_row(
                        &item.lcsc_id().unwrap_or_default(),
                        Some(&item),
                        result.map_err(|e| e.to_string()),
                        &req,
                    )
                })
                .collect();
            let mut outcome = output::exports(rows, &req);
            outcome.data["plan"] = json!(plan);
            outcome.data["bom_rows"] =
                json!(state.bom.rows.iter().filter(|row| row.included).count());
            Ok(outcome)
        }
    }
}

fn confirm(
    bom: &mut BomDocument,
    exact: bool,
    select: &[Selection],
    exclude: &[usize],
    include: &[usize],
) -> Result<()> {
    if !exact && select.is_empty() && exclude.is_empty() && include.is_empty() {
        bail!("Specify --exact, --select ROW=C_NUMBER, --exclude ROW, or --include ROW");
    }
    let mut selections = HashSet::new();
    for choice in select {
        if !selections.insert(choice.line) {
            bail!("Duplicate selection for row {}", choice.line);
        }
        if exclude.contains(&choice.line) {
            bail!(
                "Cannot select and exclude row {} in the same command",
                choice.line
            );
        }
    }
    for line in include {
        if exclude.contains(line) {
            bail!("Cannot include and exclude row {line} in the same command");
        }
    }
    for (lines, included) in [(exclude, false), (include, true)] {
        for line in lines {
            let row = bom
                .rows
                .iter_mut()
                .find(|row| row.line == *line)
                .with_context(|| format!("Unknown source row {line}"))?;
            row.included = included;
        }
    }
    for choice in select {
        let row = bom
            .rows
            .iter_mut()
            .find(|row| row.line == choice.line)
            .with_context(|| format!("Unknown source row {}", choice.line))?;
        if !row.included {
            bail!(
                "Row {} is excluded; --include it before selecting",
                choice.line
            );
        }
        let index = row
            .candidates
            .iter()
            .position(|item| item.lcsc_id().as_deref() == Some(&choice.lcsc))
            .with_context(|| {
                format!(
                    "{} is not a candidate on row {}; run bom match / show",
                    choice.lcsc, choice.line
                )
            })?;
        if !row.choose(index) {
            bail!(
                "Row {} has a source validation error; correct and reimport the BOM",
                choice.line
            );
        }
    }
    if exact {
        bom.confirm_exact_ids();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bom::BomRow;

    fn candidate(id: &str) -> SearchItem {
        SearchItem {
            index: 1,
            display_title: "Part".into(),
            title: "Part".into(),
            manufacturer: "Maker".into(),
            model_uuid: None,
            raw: json!({"product_code": id}),
        }
    }
    fn sample() -> Plan {
        Plan {
            schema_version: 1,
            source_path: "board.csv".into(),
            bom: BomDocument {
                source: "board.csv".into(),
                rows: vec![
                    BomRow {
                        line: 2,
                        lcsc: "C1001".into(),
                        quantity: 2,
                        designators: "R1,R2".into(),
                        included: true,
                        candidates: vec![candidate("C1001"), candidate("C1002")],
                        ..Default::default()
                    },
                    BomRow {
                        line: 3,
                        model: "Part".into(),
                        quantity: 1,
                        included: true,
                        candidates: vec![candidate("C1002")],
                        ..Default::default()
                    },
                ],
                warnings: vec![],
            },
        }
    }
    #[test]
    fn review_roundtrip_preserves_metadata_and_requires_every_included_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        let mut plan = sample();
        confirm(&mut plan.bom, true, &[], &[], &[]).unwrap();
        assert!(plan.selected_items().is_err());
        plan.save(&path, false).unwrap();
        let mut plan = Plan::read(&path).unwrap();
        confirm(
            &mut plan.bom,
            false,
            &["3=C1002".parse().unwrap()],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(plan.selected_items().unwrap().len(), 2);
        assert_eq!(plan.bom.rows[0].quantity, 2);
        assert_eq!(plan.bom.rows[0].designators, "R1,R2");
        confirm(
            &mut plan.bom,
            false,
            &["2=C1002".parse().unwrap()],
            &[],
            &[],
        )
        .unwrap();
        confirm(&mut plan.bom, true, &[], &[], &[]).unwrap();
        assert_eq!(
            plan.selected_items().unwrap().len(),
            1,
            "deduplicate confirmed choices and retain manual substitution"
        );
    }
    #[test]
    fn exclusion_unknown_rows_and_source_errors_are_not_silently_confirmed() {
        let mut plan = sample();
        confirm(&mut plan.bom, true, &[], &[3], &[]).unwrap();
        assert!(plan.selected_items().is_ok());
        confirm(&mut plan.bom, false, &[], &[], &[3]).unwrap();
        assert!(plan.selected_items().is_err());
        assert!(confirm(&mut plan.bom, false, &[], &[99], &[]).is_err());
        plan.bom.rows[1].validation_error = true;
        assert!(confirm(
            &mut plan.bom,
            false,
            &["3=C1002".parse().unwrap()],
            &[],
            &[]
        )
        .is_err());
    }
    #[test]
    fn plans_reject_bad_versions_invalid_selections_and_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        let mut plan = sample();
        plan.save(&path, false).unwrap();
        assert!(plan.save(&path, false).is_err());
        let lock = PlanLock::acquire(&path).unwrap();
        assert!(PlanLock::acquire(&path).is_err());
        drop(lock);
        assert!(PlanLock::acquire(&path).is_ok());
        plan.schema_version = 99;
        plan.save(&path, true).unwrap();
        assert!(Plan::read(&path).is_err());
        plan.schema_version = 1;
        plan.bom.rows[0].selected = Some(9);
        plan.save(&path, true).unwrap();
        assert!(Plan::read(&path).is_err());
    }
    #[test]
    fn invalid_confirmation_leaves_saved_plan_unchanged_and_unready_export_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let plan = dir.path().join("plan.json");
        sample().save(&plan, false).unwrap();
        let before = fs::read(&plan).unwrap();
        assert!(execute(Command::Confirm {
            plan: plan.clone(),
            exact: false,
            select: vec!["3=C9999".parse().unwrap()],
            exclude: vec![2],
            include: vec![]
        })
        .is_err());
        assert_eq!(fs::read(&plan).unwrap(), before);
        let output = dir.path().join("export");
        assert!(execute(Command::Export {
            plan,
            export: ExportOptions {
                ad: true,
                output: output.clone(),
                ..Default::default()
            },
            merge: MergeOptions::default()
        })
        .is_err());
        assert!(!output.exists());
    }
}
