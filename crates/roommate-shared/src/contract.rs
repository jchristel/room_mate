//! The two pieces of the ingest contract that settings validation needs too.
//!
//! **Not the contract.** `roommate::contract` stays in the server, with the
//! payload types and the ingest rules. These two moved because `settings`
//! reaches for them — `AreaPolicy::boundary_location` is a `RoomBoundary`, and a
//! milestone pin must name a valid snapshot id — and this crate cannot depend
//! back on the server. `roommate::contract`
//! re-exports both, so everywhere in the server they still read as contract
//! items, which is what they are.

use serde::{Deserialize, Serialize};

/// Whether a (non-blank) snapshot id is acceptable: it must parse as RFC3339
/// AND be expressed in UTC (`Z` or `+00:00`). One rule covers everything the
/// id must guarantee: it's a real date-time (the contract's definition of a
/// snapshot id), it keeps the store's lexical-max-is-newest ordering sound (a
/// non-UTC offset would sort wrongly against UTC neighbours), and it can't
/// smuggle a path escape (no RFC3339 string contains `/`, `\`, or `..`) —
/// which is why ingest needs no separate filename-safety check for it.
pub fn validate_snapshot_id(taken_at: &str) -> Result<(), String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(taken_at)
        .map_err(|e| format!("snapshot taken_at {taken_at:?} is not an RFC3339 date-time: {e}"))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(format!(
            "snapshot taken_at {taken_at:?} must be expressed in UTC (\"Z\" or \"+00:00\"), not a local offset"
        ));
    }
    Ok(())
}

/// Where a model's room boundaries sit relative to their walls — Revit's
/// `SpatialElementBoundaryLocation`, forwarded verbatim.
///
/// This is a **model fact, not a project policy**: Revit already knows it, and
/// asking a human to re-assert it in TOML duplicates an authoritative value and
/// invites getting it wrong. It rides the envelope per *model* rather than per
/// project because a project legitimately mixes both — each linked model
/// carries its own document setting.
///
/// It exists because `service::areas` otherwise has to *guess* which regime it
/// is looking at, and sizes its morphological close for the worst case. Every
/// footprint artifact chased so far — bevelled corners, 45° chamfers, the
/// million-foot spike, sibling overlaps — is downstream of that guess. Declaring
/// the regime does not merely improve the tolerance: on a centreline model the
/// close radius collapses to zero and the entire artifact class cannot arise.
/// The two regimes and what each implies are in `service::areas`' module
/// header; what the resulting number may be *called* is
/// STRATEGY-AREA-CALCULATION.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings-preview/generated/")]
pub enum RoomBoundary {
    /// Neighbouring rooms tile edge-to-edge: their shared boundaries are
    /// coincident and the gap between them is zero up to float noise. Nothing
    /// needs bridging, and the walls are already inside the room polygons.
    Centreline,
    /// Rooms float inside their walls, so neighbours across a partition are
    /// separated by roughly its thickness. The gap is real and positive, and
    /// bridging it needs a declared thickness ceiling (`[areas]`
    /// `max_wall_thickness`).
    FinishFace,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot id rule: RFC3339, expressed in UTC. Non-dates (including
    /// anything path-shaped) and non-UTC offsets are rejected; "Z" and
    /// "+00:00" both count as UTC.
    #[test]
    fn test_validate_snapshot_id() {
        assert!(validate_snapshot_id("2026-01-01T00:00:00Z").is_ok());
        assert!(validate_snapshot_id("2026-01-01T00:00:00.123456Z").is_ok());
        assert!(validate_snapshot_id("2026-01-01T00:00:00+00:00").is_ok());

        assert!(validate_snapshot_id("2026-01-01T00:00:00+10:00").is_err());
        assert!(validate_snapshot_id("not-a-date").is_err());
        assert!(validate_snapshot_id("2026/01/01").is_err());
        assert!(validate_snapshot_id("..\\..\\evil").is_err());
        assert!(validate_snapshot_id("").is_err());
    }
}
