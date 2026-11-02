//! Reference data (dRofus and any other configured source): loaded once at
//! startup, joined onto rooms at response assembly — never merged into the
//! stored snapshot.
//!
//! Two disciplines from STRATEGY.md live here. *Store raw, join late:* the
//! parsed map sits in `AppState` and is attached at `/rooms` assembly, so the
//! Revit snapshot stays untouched and the join is reversible. *Separate
//! sub-object because separate lifecycle:* a reference source will later
//! refresh on its own trigger (a mid-session poll), independent of the Revit
//! push, so it must not be fused into the room's own properties — keeping it
//! separate keeps the seam where the refresh boundary actually is.
//!
//! The loader is byte-source-agnostic (`load_reference_from_reader`, with a
//! bytes wrapper): an uploaded CSV hydrated from the snapshot store
//! (`ReferenceOrigin::Upload`) and a future API response parse through the
//! same function. Which source feeds it is dispatched in
//! `bootstrap::load_project_bundle`, where the store is in scope — not here.
//!
//! The two-header-row CSV shape this parses is dRofus's export format, which
//! is where it came from; nothing else about the module is dRofus-specific,
//! and a source declaring that shape parses here whatever it is called.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde::Serialize;

/// One reference-source row, resolved. `fields` is the source's own
/// field-label → value (row 1 labels as keys). Kept as strings — same raw
/// discipline as custom props.
#[derive(Debug, Clone, Serialize)]
pub struct ReferenceRecord {
    pub fields: BTreeMap<String, String>,
}

/// One reference source's whole dataset, resolved once at startup. `Clone` so
/// a bundle marked `is_default` can be registered both under its own project
/// id and as `AppState`'s fallback without the two copies aliasing.
#[derive(Clone)]
pub struct ReferenceData {
    /// Which room property holds the linking id (CSV row 2, col 0).
    /// Read the room property of THIS name to get its dRofus key.
    pub link_property: String,

    /// dRofus id → record. Direct value match; ids are unique, so a plain map.
    pub by_id: BTreeMap<String, ReferenceRecord>,

    /// dRofus field label (row 1) → the Revit property name row 2 lists for
    /// that same column (columns 1+; column 0 is `link_property` above).
    /// This is the "kept for reconciliation" data the CSV format documents —
    /// row 2's non-link columns were always meant to let a consumer cross-check
    /// a dRofus value against the *correct* Revit property (which may not
    /// share the dRofus field's literal name), not just read once and discard.
    pub reconciliation: BTreeMap<String, String>,

    /// Every dRofus field label from row 1 (columns 1+), regardless of
    /// whether row 2 gave it a Revit property mapping. `reconciliation` only
    /// has the *mapped* subset; the QA coverage report needs the full set too,
    /// so it can show "not currently checked" for a label that has no mapping
    /// rather than silently omitting it.
    pub all_labels: Vec<String>,

    /// Ids that appeared on **more than one data row**, ascending, each named
    /// once however many times it repeated.
    ///
    /// `by_id` is a map, so a repeated id can only keep one row: the last one
    /// wins and the earlier ones are gone. That is a lossy, order-dependent
    /// outcome — the surviving values are whichever happened to sit lowest in
    /// the file — and it used to happen in total silence: the count reported
    /// back was already the deduplicated one, so a 200-row CSV with five
    /// repeats reported 195 records and nothing anywhere said why.
    ///
    /// The load still keeps last-write-wins rather than guessing a better
    /// winner (there isn't one), but it now records what it dropped, so the
    /// arbitrariness is *reported* rather than hidden. See
    /// `service::validation`'s `reference_duplicate_ids`.
    pub duplicate_ids: Vec<String>,

    /// Data rows whose id cell (column 0) was empty, and which are therefore
    /// not in `by_id` at all. A count, not a list, because a row with no id
    /// has nothing to name it by.
    ///
    /// Skipping them is deliberate — one blank line should not fail a whole
    /// upload — but skipping them *silently* meant a CSV whose key column was
    /// mis-selected could load as zero records and look merely empty.
    pub blank_id_rows: usize,
}

/// Read the two-header-row CSV into ReferenceData. Fail fast (startup) on a
/// malformed file — same contract as load_settings.
///
/// CSV shape:
///   row 1: dRofus field labels  (DrofusRoomId, NetArea, Department, …)
///   row 2: Revit param names    (RevitDrofusKey, d_net_area, d_dept, …)
///   row 3+: data rows
/// Row 2, col 0 = the Revit room property whose value is the dRofus id (link).
pub fn load_reference_from_reader<R: std::io::Read>(reader: R) -> anyhow::Result<ReferenceData> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false) // both header rows are data to us; we parse them by hand
        .from_reader(reader);

    let mut records = rdr.records();

    // Row 1: dRofus field labels.
    let labels = records.next().context("reference CSV missing row 1 (field labels)")??;
    // Row 2: Revit param names. Col 0 is the link property name.
    let revit_names = records.next().context("reference CSV missing row 2 (Revit param names)")??;

    let link_property = revit_names
        .get(0)
        .context("reference CSV row 2 col 0 (link property) is empty")?
        .to_string();

    // Row 1/row 2, cols 1+: dRofus field label -> the Revit property name it
    // reconciles against. Blank Revit-name cells are skipped rather than
    // failing the load — reconciliation is a bonus check, not required for
    // the join itself to work.
    let mut reconciliation = BTreeMap::new();
    let mut all_labels = Vec::new();
    for col in 1..labels.len() {
        if let Some(label) = labels.get(col) {
            all_labels.push(label.to_string());
            if let Some(revit_name) = revit_names.get(col)
                && !revit_name.is_empty()
            {
                reconciliation.insert(label.to_string(), revit_name.to_string());
            }
        }
    }

    // Data rows: col 0 is the dRofus id (the key), cols 1+ are values keyed by
    // the row-1 label at the same column index.
    // Both counters below exist because this loop is *lossy* in two ways, and
    // both used to be invisible: a blank id skips the row, and a repeated id
    // overwrites the row before it. `by_id.len()` reports the survivors, so
    // without these the loss could not be seen from the outside at all.
    let mut by_id = BTreeMap::new();
    let mut duplicate_ids = BTreeSet::new();
    let mut blank_id_rows = 0usize;
    for row in records {
        let row = row?;
        let id = match row.get(0) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                // Skipped, not fatal — one blank line must not fail an upload.
                // Counted, so "my 200-row CSV loaded 0 records" has an answer.
                blank_id_rows += 1;
                continue;
            }
        };
        let mut fields = BTreeMap::new();
        for col in 1..labels.len() {
            if let (Some(label), Some(val)) = (labels.get(col), row.get(col)) {
                fields.insert(label.to_string(), val.to_string());
            }
        }
        // A `BTreeSet` so an id repeated five times is named once, and the
        // result is ascending for a stable report.
        if by_id.insert(id.clone(), ReferenceRecord { fields }).is_some() {
            duplicate_ids.insert(id);
        }
    }

    // Warn as well as report: the loader also runs at boot, where nobody is
    // looking at an HTTP response.
    if !duplicate_ids.is_empty() {
        tracing::warn!(
            "reference CSV has {} repeated id(s) — only the last row of each was kept: {}",
            duplicate_ids.len(),
            duplicate_ids.iter().take(10).cloned().collect::<Vec<_>>().join(", ")
        );
    }
    if blank_id_rows > 0 {
        tracing::warn!("reference CSV has {blank_id_rows} row(s) with an empty id column — skipped");
    }
    tracing::info!("loaded {} reference record(s); link property = {}", by_id.len(), link_property);

    Ok(ReferenceData {
        link_property,
        by_id,
        reconciliation,
        all_labels,
        duplicate_ids: duplicate_ids.into_iter().collect(),
        blank_id_rows,
    })
}

/// Load a dRofus CSV from raw bytes (an upload body, or a stored upload
/// hydrated at boot). Strips a leading UTF-8 BOM first: Excel CSV exports
/// routinely carry one and the csv crate does not strip it. The BOM lands in
/// row 1 col 0 — unused today, but a quoted first cell parses wrong with a
/// BOM in front, and "col 0 is never read" is not a contract worth leaning on.
pub fn load_reference_from_bytes(bytes: &[u8]) -> anyhow::Result<ReferenceData> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    load_reference_from_reader(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The silent data loss this reports.** `by_id` keeps one row per id, so
    /// three rows sharing an id collapse to one — last write wins, meaning the
    /// surviving values are whichever sat lowest in the file. That is
    /// arbitrary, and it used to happen with nothing said: `record_count` was
    /// already the deduplicated number.
    #[test]
    fn test_repeated_ids_are_reported_and_last_row_wins() {
        let data = load_reference_from_bytes(
            b"DrofusRoomId,NetArea
Number,Area
1,first
1,second
1,third
2,only
",
        )
        .unwrap();

        assert_eq!(data.by_id.len(), 2, "four rows, two surviving records");
        assert_eq!(data.duplicate_ids, vec!["1".to_string()], "named once, however many times it repeated");
        assert_eq!(
            data.by_id["1"].fields.get("NetArea"),
            Some(&"third".to_string()),
            "last row wins -- arbitrary, which is exactly why it is reported"
        );
        assert_eq!(data.blank_id_rows, 0);
    }

    /// Rows with an empty id cell are skipped — one trailing blank line must
    /// not fail an upload — but counted, so "my CSV loaded zero records" has
    /// an answer instead of looking merely empty.
    #[test]
    fn test_blank_id_rows_are_skipped_but_counted() {
        let data = load_reference_from_bytes(
            b"DrofusRoomId,NetArea
Number,Area
1,ok
,orphan
,another
",
        )
        .unwrap();

        assert_eq!(data.by_id.len(), 1);
        assert_eq!(data.blank_id_rows, 2);
        assert!(data.duplicate_ids.is_empty(), "a missing id is not a repeated one");
    }

    /// A clean CSV reports neither — the diagnostics must not cry wolf.
    #[test]
    fn test_a_clean_csv_reports_no_loss() {
        let data = load_reference_from_bytes(
            b"DrofusRoomId,NetArea
Number,Area
1,10
2,20
",
        )
        .unwrap();

        assert_eq!(data.by_id.len(), 2);
        assert!(data.duplicate_ids.is_empty());
        assert_eq!(data.blank_id_rows, 0);
    }

    /// Row 2's non-link columns populate `reconciliation` (label -> Revit
    /// property name); a blank Revit-name cell is skipped, not fatal.
    #[test]
    fn test_load_drofus_populates_reconciliation() {
        let data = load_reference_from_bytes(
            b"DrofusRoomId,NetArea,Department,Notes\nNumber,Area,Department,\n1,25.5,Cardiology,ignored\n",
        )
        .unwrap();

        assert_eq!(data.link_property, "Number");
        assert_eq!(data.reconciliation.get("NetArea"), Some(&"Area".to_string()));
        assert_eq!(data.reconciliation.get("Department"), Some(&"Department".to_string()));
        // "Notes" has a blank Revit-name cell in row 2 -- skipped, not present.
        assert_eq!(data.reconciliation.get("Notes"), None);
        assert_eq!(data.by_id["1"].fields.get("NetArea"), Some(&"25.5".to_string()));

        // `all_labels` carries every row-1 label regardless of mapping --
        // "Notes" belongs here even though it's absent from `reconciliation`,
        // so the coverage report can show it as "not checked" rather than
        // silently omitting it.
        assert_eq!(
            data.all_labels,
            vec!["NetArea".to_string(), "Department".to_string(), "Notes".to_string()]
        );
    }

    /// The bytes loader parses an upload body directly, and strips a leading
    /// UTF-8 BOM (Excel exports carry one; the csv crate does not strip it).
    #[test]
    fn test_load_reference_from_bytes_strips_bom() {
        let csv = "DrofusRoomId,NetArea\nNumber,Area\n1,25.5\n";

        let plain = load_reference_from_bytes(csv.as_bytes()).unwrap();
        assert_eq!(plain.link_property, "Number");
        assert_eq!(plain.by_id["1"].fields.get("NetArea"), Some(&"25.5".to_string()));

        let mut bom_prefixed = b"\xEF\xBB\xBF".to_vec();
        bom_prefixed.extend_from_slice(csv.as_bytes());
        let bom = load_reference_from_bytes(&bom_prefixed).unwrap();
        assert_eq!(bom.link_property, "Number");
        assert_eq!(bom.all_labels, vec!["NetArea".to_string()]);
    }
}
