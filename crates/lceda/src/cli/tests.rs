use super::*;

fn item(id: &str, model: &str, index: usize) -> SearchItem {
    SearchItem {
        index,
        display_title: model.into(),
        title: model.into(),
        manufacturer: "Maker".into(),
        model_uuid: None,
        raw: json!({"product_code": id}),
    }
}

#[test]
fn command_tree_and_shared_options_are_valid() {
    use clap::CommandFactory;
    Cli::command().debug_assert();
    for command in ["get", "batch"] {
        for mils in ["1", "7", "25", "50", "100", "1000"] {
            let cli =
                Cli::try_parse_from(["lceda", command, "input", "--pads", "--pads-grid", mils])
                    .unwrap();
            let export = match cli.cmd.unwrap() {
                Cmd::Get { export, .. } | Cmd::Batch { export, .. } => export,
                _ => unreachable!(),
            };
            assert_eq!(export.pads_grid.unwrap().mils().to_string(), mils);
        }
        for invalid in ["0", "-10", "1.5", "NaN", "1001", "4294967306"] {
            assert!(
                Cli::try_parse_from(["lceda", command, "input", "--pads-grid", invalid]).is_err()
            );
        }
    }
    assert!(Cli::try_parse_from([
        "lceda",
        "bom",
        "export",
        "plan.json",
        "--json",
        "--ad",
        "--merge",
        "--name",
        "Board"
    ])
    .is_ok());
    assert!(Cli::try_parse_from(["lceda", "get", "C1001", "--sch-color", "typo"]).is_err());
    assert!(Cli::try_parse_from(["lceda", "batch", "parts.txt", "--name", "Board"]).is_err());
}

#[test]
fn exact_ids_never_fall_back_and_ambiguous_models_require_explicit_selection() {
    assert!(select("C1001", vec![item("C1002", "Part", 1)], None).is_err());
    assert_eq!(
        select(
            "c1001",
            vec![item("C1002", "Part", 1), item("C1001", "Part", 2)],
            None
        )
        .unwrap()
        .lcsc_id()
        .as_deref(),
        Some("C1001")
    );
    let candidates = vec![item("C1001", "Part", 1), item("C1002", "Part", 2)];
    assert!(select("Part", candidates.clone(), None).is_err());
    assert_eq!(
        select("Part", candidates, Some(2))
            .unwrap()
            .lcsc_id()
            .as_deref(),
        Some("C1002")
    );
    assert!(select("C1001", vec![item("C1002", "Part", 1)], Some(1)).is_err());
    assert!(select("partial", vec![item("C1001", "Different", 1)], None).is_err());
}

#[test]
fn missing_requested_outputs_are_reported_as_partial() {
    let req = lceda_core::export::ExportRequest {
        datasheet: true,
        ad: true,
        ..Default::default()
    };
    let paths = lceda_core::models::DownloadPaths {
        schlib: Some("out/Part.SchLib".into()),
        ..Default::default()
    };
    let row = output::export_row("C1001", Some(&item("C1001", "Part", 1)), Ok(paths), &req);
    assert_eq!(row["status"], "partial");
    assert_eq!(row["missing"], json!(["datasheet"]));
    assert_eq!(output::exports(vec![row], &req).code, 1);
}
