//! Milestones, the area hierarchy's tiers and exclusions, and the built-in properties a project defines.

use super::*;

/// One user-defined milestone: a named date with data snapshots explicitly
/// pinned to it (`attachments`: model id → snapshot `taken_at`). The *name*
/// is the milestone's identity — unique per project, and what
/// `/rooms?milestone=` matches on; the date is display/ordering metadata.
/// A milestone can also pin reference-source snapshots (`reference_snapshots`,
/// source name → `taken_at`), so the milestone view joins each source's
/// reference data as it stood at the milestone rather than the project's
/// current data — dRofus is the first (and today, only) entry.
#[derive(Debug, Clone, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct Milestone {
    /// Identity: unique per project, non-empty (validated at load).
    pub name: String,
    /// Display/order date: `YYYY-MM-DD` or a full RFC3339 date-time
    /// (validated at load).
    pub date: String,
    /// Optional reference-source snapshots pinned to this milestone: source
    /// name → the `taken_at` id of one uploaded CSV in the store, joined onto
    /// this milestone's rooms instead of that source's current data. A source
    /// with no entry (the common case, and every milestone authored before
    /// this field existed) keeps the pre-pinning behaviour — the milestone
    /// view joins that source's *current* data. Like an `attachments` pin,
    /// whether a pinned snapshot still *exists* is a read-time concern (skip +
    /// warn, fall back to current); only its *shape* (a valid RFC3339-UTC
    /// snapshot id) is validated here.
    #[serde(default)]
    pub reference_snapshots: BTreeMap<String, String>,
    /// Explicit pins: model id → snapshot id (`taken_at`). A model with no
    /// entry simply doesn't appear in this milestone's view. Whether a pinned
    /// snapshot still *exists* is a read-time concern (skip + warn), not
    /// validated here — settings can't see storage.
    #[serde(default)]
    pub attachments: BTreeMap<String, String>,
    /// The same, for this milestone's **doors** snapshots: model id → snapshot
    /// id. A separate map rather than a second use of `attachments`, because
    /// rooms and doors are pushed independently and their snapshot ids do not
    /// correspond — pinning a milestone to a rooms snapshot says nothing about
    /// which doors snapshot was current at the time, and guessing "the nearest"
    /// would silently pair data that never coexisted.
    ///
    /// A model with no entry contributes no doors to this milestone's view,
    /// exactly as `attachments` governs its rooms. `default` keeps every
    /// milestone authored before doors existed valid, as one that pins none.
    #[serde(default)]
    pub door_attachments: BTreeMap<String, String>,

    /// The same again, for **windows**. A third map for the reason the second
    /// one gives: the three entities are pushed independently and their snapshot
    /// ids do not correspond, so pinning one says nothing about the others and
    /// "the nearest" would silently pair data that never coexisted.
    ///
    /// `default` keeps every milestone authored before windows existed valid, as
    /// one that pins none -- which is what it is.
    #[serde(default)]
    pub window_attachments: BTreeMap<String, String>,

    /// And again, for **FF&E**. A fourth map, for the reason the second one
    /// gave and the third one paid: the entities are pushed independently and
    /// their snapshot ids do not correspond, so pinning one says nothing about
    /// the others and "the nearest" would silently pair data that never
    /// coexisted.
    ///
    /// That matters more here than for any other entity. A milestone exists to
    /// answer "what did the model hold on date X", and FF&E is the thing that
    /// changes between issues while the rooms stand still -- so an FF&E pin
    /// inferred from a rooms pin would be wrong precisely when someone is
    /// looking.
    ///
    /// `default` keeps every milestone authored before FF&E existed valid, as
    /// one that pins none.
    #[serde(default)]
    pub ffe_attachments: BTreeMap<String, String>,

    /// Per-model spaces-snapshot pins for this milestone, on the same terms as
    /// every other entity's map: pushed independently, so pinning one says
    /// nothing about the others.
    ///
    /// **A spaces pin and a rooms pin are the ones most likely to disagree**,
    /// and that is a reason for the map rather than an argument against it. The
    /// two live in different documents by construction -- rooms in the
    /// architectural models, spaces in the services ones -- so they are issued
    /// on different dates by different people. "What did this milestone hold"
    /// has to be answerable separately for each, or the answer is whichever one
    /// happened to be pushed nearest the date.
    #[serde(default)]
    pub space_attachments: BTreeMap<String, String>,

    /// Per-model ceilings snapshot pins, on the same terms as
    /// `space_attachments`.
    ///
    /// Its own map rather than riding the rooms pin, even though a ceiling and
    /// the rooms it covers are normally in ONE document. A milestone pins what
    /// was pushed, and ceilings are pushed by their own entry point on their own
    /// day; sharing the rooms pin would make "which ceilings did this milestone
    /// hold" answerable only when the two happened to be pushed together.
    #[serde(default)]
    pub ceiling_attachments: BTreeMap<String, String>,

    /// Per-model floors snapshot pins, on the same terms as
    /// `ceiling_attachments`. Floors and ceilings share a record and a read,
    /// but not a push: each has its own entry point, so each is pinned on its
    /// own.
    #[serde(default)]
    pub floor_attachments: BTreeMap<String, String>,
}

impl Milestone {
    /// Startup-loud checks on one milestone's own fields (uniqueness across
    /// milestones is checked in `load_settings`, which can see the siblings).
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("a milestone has an empty name");
        }
        let date_ok = chrono::NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").is_ok()
            || chrono::DateTime::parse_from_rfc3339(&self.date).is_ok();
        if !date_ok {
            anyhow::bail!(
                "milestone '{}' has an invalid date {:?} (expected YYYY-MM-DD or RFC3339)",
                self.name,
                self.date
            );
        }
        // Both pin maps get the same shape check — an id that is not a valid
        // RFC3339-UTC snapshot id can never name a stored snapshot, so it is a
        // startup-loud error rather than a read that silently finds nothing.
        for (label, pins) in [
            ("attachment", &self.attachments),
            ("door attachment", &self.door_attachments),
            ("window attachment", &self.window_attachments),
            ("ffe attachment", &self.ffe_attachments),
            ("space attachment", &self.space_attachments),
            ("ceiling attachment", &self.ceiling_attachments),
            ("floor attachment", &self.floor_attachments),
        ] {
            for (model_id, taken_at) in pins {
                if model_id.trim().is_empty() {
                    anyhow::bail!("milestone '{}' has an {} with an empty model id", self.name, label);
                }
                crate::contract::validate_snapshot_id(taken_at).map_err(|e| {
                    anyhow::anyhow!("milestone '{}', {} for model '{}': {}", self.name, label, model_id, e)
                })?;
            }
        }
        // Same rule as an attachments pin: a valid RFC3339-UTC snapshot id.
        // Existence is not checkable here (settings can't see storage).
        for (source, id) in &self.reference_snapshots {
            crate::contract::validate_snapshot_id(id)
                .map_err(|e| anyhow::anyhow!("milestone '{}', reference_snapshots.{}: {}", self.name, source, e))?;
        }
        Ok(())
    }
}

/// One tier of the classification hierarchy. A tier is keyed by a code and/or a
/// name property — at least one must be present (validated at startup), since a
/// tier naming neither is unkeyable.
#[derive(Debug, Clone, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct HierarchyTier {
    /// Human label for the tier ("Building", "Department").
    pub name: String,
    /// Room property holding this tier's code. Optional per-tier.
    #[serde(default)]
    pub code_property: Option<String>,
    /// Room property holding this tier's display name. Optional per-tier.
    #[serde(default)]
    pub name_property: Option<String>,
}

impl HierarchyTier {
    /// A tier must name at least one property or it can't be keyed. Validated
    /// at startup so a misconfigured tier is a loud error, not a silent
    /// "undefined" for every room.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.code_property.is_none() && self.name_property.is_none() {
            anyhow::bail!("hierarchy tier '{}' names neither code_property nor name_property", self.name);
        }
        Ok(())
    }
}

/// A footprint exclusion for the hierarchy-areas feature (`service::areas`). The
/// match kind implies WHERE in the two-stage pipeline it applies — the handover's
/// "the match kind implies the stage":
///
/// - `group` — Case A, applied at **stage 2**: a resolved group at `tier` whose
///   value matches is computed normally but WITHHELD from its parent's dissolve,
///   so it drops out of that tier and every tier above while its own footprints
///   stay. Still reported, flagged "not counted upward" (outdoor areas: real,
///   with their own plan, but not part of the building footprint).
/// - `rooms` — Case B, applied at **stage 1**: the listed room ids never enter
///   any union, so they vanish from every tier including their own bottom group.
///
/// Matching a group reuses the resolved tier value everything else classifies
/// against — no second matching vocabulary; `value` matches the tier's resolved
/// code OR name. Internally tagged on `match`, the same self-describing shape
/// `ReferenceOrigin`/`ColourMode` use.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "match", rename_all = "lowercase")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum HierarchyExclusion {
    Group { tier: String, value: String },
    Rooms { ids: Vec<String> },
}

/// One canonical property definition: a stable name consumers (dRofus
/// `link_property`, hierarchy tier `code_property`/`name_property`) reference,
/// resolved per-source to whatever raw property name that source actually
/// uses. See `Settings::builtin_properties`.
#[derive(Debug, Clone, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct BuiltinPropertyDef {
    /// The stable name consumers reference (e.g. "Area").
    pub canonical: String,
    /// Source key (e.g. "revit") → that source's raw property name.
    pub by_source: HashMap<String, String>,
}

impl BuiltinPropertyDef {
    /// A definition with no source mappings can never resolve to anything —
    /// fail fast rather than silently never matching at request time.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.by_source.is_empty() {
            anyhow::bail!("builtin property '{}' has no by_source mappings", self.canonical);
        }
        Ok(())
    }
}
