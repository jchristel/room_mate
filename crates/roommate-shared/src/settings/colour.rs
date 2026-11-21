//! Colour plans: a property painted onto rooms by value, by comparison or by band.

use super::*;

/// One named, persisted colouring configuration. `active` marks the plan the
/// viewer's colour picker defaults to — the picker also offers "None (flat)",
/// which always overrides, so `active` is a default selection, not a forced
/// application. `name` is user-facing only (the picker label). At most one plan
/// may be `active` (validated — see `validate_colour_plans`).
///
/// Scalar fields (`name`, `active`) are declared before `mode` (a sub-table)
/// so the TOML serializer emits them ahead of the `[colour_plans.mode]` table
/// — the same footgun `Milestone`'s field order (scalars before
/// `reference_snapshots`/`attachments`) guards against.
#[derive(Debug, Clone, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct ColourPlan {
    /// User-facing label shown in the viewer's colour picker.
    pub name: String,
    /// Whether this plan is the viewer picker's default selection. `false`
    /// (the default) means the plan is in the library but not the default —
    /// the picker starts on "None (flat)" unless some plan sets this.
    #[serde(default)]
    pub active: bool,
    /// The colouring strategy. Internally tagged on `kind` so the wire shape is
    /// self-describing and the browser switches on one field — the same tagged
    /// representation `ReferenceOrigin` uses.
    pub mode: ColourMode,
}

/// The colouring strategies. Internally tagged on `kind` (like `ReferenceOrigin`'s
/// `type`), so the browser branches on `mode.kind`. Every variant is a *struct*
/// variant, not a newtype/tuple: internally-tagged serde enums can't carry a
/// newtype variant that wraps a sequence, and struct variants keep the JSON/TOML
/// shape flat and self-describing.
///
/// All three modes are wired end-to-end in the viewer (`colourForRoom` in
/// `index.html` — see STRATEGY-BROWSER.md); an authored plan whose values don't
/// resolve (an unparseable property, a ratio-by-zero, a value between bands, an
/// undefined tier) degrades to a "no data" grey rather than erroring.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum ColourMode {
    /// Categorical hue per parent hierarchy tier, tint/shade per child tier.
    /// `tiers` names which hierarchy tiers participate, parent first (the
    /// browser reads each room's server-resolved `classification` path and
    /// matches by tier name); `scheme` names a bundled qualitative palette for
    /// the parent hues (child tint/shade is derived by lightening, no second
    /// scheme). One tier → hue only; a room whose parent tier is `undefined`
    /// renders "no data" grey.
    Hierarchy { tiers: Vec<String>, scheme: String },

    /// Colour by proximity of a date-typed `property` to `near_date`: nearest
    /// green, furthest red, a date after `near_date` blue. `property` is a
    /// canonical/room property name resolved browser-side the same way labels
    /// are; `scheme` names a bundled diverging palette. `format` is the
    /// strftime pattern the room's date strings are in — the *same* pattern the
    /// dRofus date column uses (Revit room dates originate from dRofus), so an
    /// author reuses that reference field's `format` rather than inventing one;
    /// omitted means the browser falls back to native ISO-8601 parsing, and an
    /// unparseable value just renders "no data" grey. Validated as a real
    /// strftime pattern at load when present (see `validate_colour_plans`).
    DateRange {
        property: String,
        near_date: String,
        scheme: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<String>,
    },

    /// Compare two room properties. `op` derives one number per room
    /// (difference or ratio of A and B); `colouring` maps that number to a
    /// colour. The two steps are kept deliberately separate: the number
    /// derivation is what a *future* `MilestoneCompare` mode (same property
    /// across two snapshots — current vs a `/rooms?milestone=`-pinned one)
    /// would swap out, reusing `Colouring` untouched. Property names that don't
    /// resolve on a given room aren't an error — that room just renders "no
    /// data" grey (the `room_label` "absence is fine" discipline).
    PropertyCompare {
        property_a: String,
        property_b: String,
        op: CompareOp,
        colouring: Colouring,
    },
}

/// How `PropertyCompare` reduces two property values to one number.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum CompareOp {
    /// `A − B`. The natural choice for match (`|A−B| ≤ tol`) and a
    /// zero-centred diverging ramp.
    Diff,
    /// `A / B`. For proportional comparisons; the browser guards division by
    /// zero (→ "no data" grey), so no server-side check is needed.
    Ratio,
}

/// The number→colour step, factored *out* of `PropertyCompare` on purpose: it's
/// the reusable half, so a future mode that derives a per-room number
/// differently (e.g. `MilestoneCompare`) reuses these three styles without
/// change. Internally tagged on `style`; struct variants only (see
/// `ColourMode` for why).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "style", rename_all = "lowercase")]
#[derive(ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub enum Colouring {
    /// Two colours: within `tolerance` of zero (a match) vs. not. This is the
    /// dRofus-vs-Revit QA case. Reuses the *philosophy* of `CompareMode`'s
    /// numeric-adaptive comparison (both sides parse as numbers → numeric
    /// compare with tolerance, else exact) rather than inventing a second one —
    /// the actual comparison runs browser-side against string property values.
    Match { tolerance: f64 },
    /// Map the number onto a diverging palette centred on zero, auto-scaled to
    /// the level's data extent (computed per-render in the browser). `scheme`
    /// names a bundled diverging palette.
    Diverging { scheme: String },
    /// Map the number through user-defined cutoff→colour `bands`. A struct
    /// variant (`{ bands }`), not a newtype `Bands(Vec<Band>)`, because
    /// internally-tagged enums can't wrap a sequence in a newtype variant.
    Bands { bands: Vec<Band> },
}

/// One band of a `Bands` colouring: the half-open interval `[lo, hi)` gets
/// `colour`. `lo`/`hi` are `Option` so the first/last band can be open-ended
/// (`None` = −∞ / +∞). Bands are validated at load to be sorted and disjoint
/// (see `validate_colour_plans`), which is what lets the browser do a simple
/// ordered first-match scan with no overlap-resolution logic. A value that
/// falls in a *gap* between bands (allowed) renders as "no data" grey — a
/// deliberate gap, not a bug.
///
/// `colour` is a CSS colour string (e.g. `"#b4541f"`); the server never parses
/// it — validating colour syntax is a browser concern, and an unparseable one
/// just renders as the browser's fallback.
#[derive(Debug, Clone, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../src-js/settings/generated/")]
pub struct Band {
    /// Inclusive lower bound; `None` = open (−∞).
    #[serde(default)]
    pub lo: Option<f64>,
    /// Exclusive upper bound; `None` = open (+∞).
    #[serde(default)]
    pub hi: Option<f64>,
    /// CSS colour string applied to rooms whose value lands in `[lo, hi)`.
    pub colour: String,
}
