//! Reference reconciliation QA: does each room's Revit data agree with every
//! reference source the project configures.
//!
//! **One report per source, not one report.** Each source declares its own
//! link property, so "which rooms resolved no link value" and "what is this
//! room's link value" are different questions for each of them and cannot
//! share a list. `compute_validation` reconciles one source and knows nothing
//! about the others; `compute_project_validation` runs it once per loaded
//! source and sums the tallies for the panel's collapsed header. A source
//! configured but not yet uploaded is skipped, not failed — "declared,
//! nothing uploaded yet" is a normal state, the same "signal, not error"
//! policy the unmatched-key checks themselves follow.
//!
//! **Unmatched is reported in both directions**, and only one of them is
//! obvious. Every other check here walks the rooms and asks the source a
//! question, so `rooms_unmatched` ("this room has no record") falls out
//! naturally. `reference_unmatched` ("this record has no room") requires
//! walking the source instead, and its absence used to fail silently in the
//! worst way: a source of 200 rows joined against 50 rooms reported zero
//! unmatched and read as clean. A reconciliation report exists to notice the
//! two sides disagree, and that was half the disagreement.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::contract::{
    date_match, lookup_property, numeric_match, property_presence, DoorPayload, PropertyPresence, Room, RoomPayload,
};
use crate::reference::ReferenceData;
use crate::settings::{
    BuiltinPropertyDef, CompareMode, FieldType, OpeningPolicy, ReferenceFieldConfig, RoomAttribution, RoomResolution,
};
use crate::state::{AppState, ModelKey};

use super::ServiceError;

/// Resolved link value → every `(room, source)` that resolved to it. A value
/// with more than one entry is an ambiguous (duplicate) link value, excluded
/// from the unmatched/mismatch checks. Borrows the rooms out of the stored
/// payloads, hence the lifetime.
type LinkValueIndex<'a> = BTreeMap<String, Vec<(&'a Room, &'a str)>>;

/// One link-property value shared by more than one room — ambiguous, so it's
/// excluded from the unmatched/mismatch checks below rather than guessing
/// which room a dRofus record actually describes.
#[derive(Serialize)]
pub struct DuplicateLinkValue {
    pub value: String,
    pub room_ids: Vec<String>,
}

/// One property where a uniquely-matched room and its dRofus record disagree.
#[derive(Serialize)]
pub struct PropertyMismatch {
    pub room_id: String,
    pub reference_id: String,
    /// The dRofus field label (row 1) — the same key `reconciliation` and
    /// `ReferenceRecord.fields` use.
    pub field: String,
    pub room_value: String,
    pub reference_value: String,
}

/// One reconciled field where dRofus has a real value but the matched room's
/// corresponding Revit property doesn't (see `PropertyPresence`). Kept as two
/// separate response lists rather than one, because the two cases mean
/// different things: landing here via `Absent` means the property was never
/// extracted from Revit for this room at all -- a mapping typo or a
/// parameter the extractor never wired up, worth flagging loudly; via
/// `Empty` it just means nobody has filled the value in yet, an ordinary
/// per-room gap.
#[derive(Serialize)]
pub struct MissingInRevit {
    pub room_id: String,
    pub reference_id: String,
    pub field: String,
}

/// Whether one dRofus CSV field (row 1) is actually checked by this QA pass,
/// and if so, which Revit property it's checked against. A field overridden
/// `Ignore` in settings is left out of this list entirely -- that's a
/// deliberate exclusion (e.g. a sync timestamp that will legitimately always
/// differ), not a coverage gap someone needs to notice and fix.
#[derive(Serialize)]
pub struct FieldCoverage {
    pub label: String,
    pub checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revit_property: Option<String>,
}

/// Per-room detail for a room that appears in some discrepancy list — the
/// human-friendly fields the CSV export shows beyond the bare `room_id`. Keyed
/// by `room_id` in `ValidationResponse::error_rooms`. Every field defaults to
/// `""` when the underlying property doesn't resolve (an absent room Number, or
/// a room that resolved no link value at all), so a consumer never has to
/// distinguish "absent" from "empty" here — the discrepancy lists already carry
/// that distinction where it matters.
#[derive(Serialize)]
pub struct ErrorRoomInfo {
    /// The room's Revit "Number" parameter value (resolved via `lookup_property`).
    pub number: String,
    /// The room's Revit "Name" parameter value (resolved via `lookup_property`).
    pub name: String,
    /// The room's dRofus link value — its value for the link property. `""` when
    /// the room resolved none (i.e. it's in `rooms_missing_link_value`).
    pub link_value: String,
}

/// Discrepancy tallies so a consumer (MCP `get_validation`, the browser panel)
/// can answer "how many discrepancies?" without re-summing the seven lists.
/// Category counts are the **list lengths** — `duplicate_link_values` counts
/// duplicate-value *groups*, not the rooms in them — matching the panel's
/// existing issue count. `total` is their sum.
#[derive(Serialize, Default)]
pub struct DiscrepancyCounts {
    pub total: usize,
    pub rooms_missing_link_value: usize,
    pub duplicate_link_values: usize,
    pub rooms_unmatched: usize,
    pub reference_unmatched: usize,
    pub reference_duplicate_ids: usize,
    pub reference_blank_id_rows: usize,
    pub property_mismatches: usize,
    pub fields_absent_in_revit: usize,
    pub fields_empty_in_revit: usize,
}

impl DiscrepancyCounts {
    /// Accumulate another source's tallies into this one — how the response's
    /// cross-source total is built, so the QA header can show one number
    /// without the client re-summing every section.
    fn add(&mut self, other: &DiscrepancyCounts) {
        self.total += other.total;
        self.rooms_missing_link_value += other.rooms_missing_link_value;
        self.duplicate_link_values += other.duplicate_link_values;
        self.rooms_unmatched += other.rooms_unmatched;
        self.reference_unmatched += other.reference_unmatched;
        self.reference_duplicate_ids += other.reference_duplicate_ids;
        self.reference_blank_id_rows += other.reference_blank_id_rows;
        self.property_mismatches += other.property_mismatches;
        self.fields_absent_in_revit += other.fields_absent_in_revit;
        self.fields_empty_in_revit += other.fields_empty_in_revit;
    }
}

/// One reference source's reconciliation report. Every list here is scoped to
/// that source: each source declares its own link property, so "which rooms
/// failed to resolve a link value" and "what is this room's link value" are
/// different questions per source and cannot share a list.
#[derive(Serialize)]
pub struct SourceValidation {
    pub link_property: String,
    /// The link property named **nothing on any room in the project** — not
    /// blank somewhere, absent everywhere. A configuration fault, reported
    /// apart from the data-quality lists because it is not one of them: every
    /// other finding here says something about the data, and this one says the
    /// join was never wired up.
    ///
    /// It has to be called out separately because its symptoms are
    /// indistinguishable from a catastrophic data problem. `resolve_raw_name`
    /// falls back to using a canonical name **verbatim** when no
    /// `builtin_properties` entry maps it (deliberately — that is what lets a
    /// config name a raw Revit property directly), so a link property with a
    /// typo, or one needing an alias that nobody declared, silently looks up a
    /// property no room has. Every room then lands in
    /// `rooms_missing_link_value` and every row in `reference_unmatched`, and
    /// the source's own record count still reports fine.
    ///
    /// That cost a real afternoon on a production machine: an `arch` CSV whose
    /// row 2 declares `Room Number` against rooms whose Revit property is
    /// `Number`, on a settings file missing the alias the dev machine had.
    /// "3045 rooms and 1824 rows, none of them linked" reads as broken data
    /// until somebody thinks to doubt the property name.
    ///
    /// **Absent everywhere, not empty everywhere**, and the difference is the
    /// whole point — see `PropertyPresence`. A property that exists and is
    /// blank on every room is an ordinary (if extreme) data gap and is NOT
    /// flagged; one that no room carries at all cannot be anything but
    /// misconfiguration. False on a project with no rooms, where there is
    /// nothing to conclude.
    pub link_property_absent_everywhere: bool,
    pub rooms_missing_link_value: Vec<String>,
    pub duplicate_link_values: Vec<DuplicateLinkValue>,
    /// Rooms whose link value finds no record in this source.
    pub rooms_unmatched: Vec<String>,
    /// **The other direction**: link values this source carries that no room
    /// resolves to, ascending. `rooms_unmatched` answers "which rooms have no
    /// record?"; without this, "which records have no room?" was never asked,
    /// and a source of 200 rows joined against 50 rooms reported zero
    /// unmatched and read as clean.
    ///
    /// A value shared by several rooms is **matched**, not listed here: the
    /// records do have rooms pointing at them, and the ambiguity is already
    /// reported once as `duplicate_link_values`. Listing it again in the
    /// opposite direction would double-count one problem.
    ///
    /// Bare values, with no per-record detail. `error_rooms` cannot serve this
    /// list — its entries are keyed by room id and there is no room here —
    /// and the link value is the identity a reader needs to go find the row.
    pub reference_unmatched: Vec<String>,
    /// Ids the source repeated across rows. **Data the loader threw away**:
    /// `by_id` keeps one row per id, so every earlier row for a repeated id is
    /// gone and the surviving values are whichever sat lowest in the file.
    /// Nothing downstream can detect that — the record still matches a room,
    /// so it is neither unmatched nor a mismatch — which is why it has to be
    /// reported from the load itself (`ReferenceData::duplicate_ids`).
    pub reference_duplicate_ids: Vec<String>,
    /// Rows the source left without an id, and which the loader therefore
    /// skipped. A count: a row with no id has nothing to name it by. Mostly
    /// harmless (a trailing blank line) but the signal that catches the
    /// expensive case — a CSV whose key column was mis-selected loads as zero
    /// records and would otherwise look merely empty.
    pub reference_blank_id_rows: usize,
    pub property_mismatches: Vec<PropertyMismatch>,
    pub fields_absent_in_revit: Vec<MissingInRevit>,
    pub fields_empty_in_revit: Vec<MissingInRevit>,
    pub field_coverage: Vec<FieldCoverage>,
    /// Discrepancy tallies (total + per-category) — see `DiscrepancyCounts`.
    pub discrepancies: DiscrepancyCounts,
    /// `room_id` → its `ErrorRoomInfo`, populated only for rooms that appear in
    /// some discrepancy list above. What the CSV export reads to fill its
    /// room_number/room_name/link-value columns. Per source because
    /// `link_value` is resolved through *this* source's link property.
    pub error_rooms: BTreeMap<String, ErrorRoomInfo>,
}

/// Data-quality report for one project's rooms against every reference source
/// it configures, for the header's validation panel. An on-demand aggregate
/// over the whole snapshot, not a per-room render concern — see
/// STRATEGY-SOURCES.md.
#[derive(Serialize)]
pub struct ValidationResponse {
    /// One report per configured, loaded reference source, keyed by source
    /// name. **Empty is the normal "nothing to reconcile" answer**, not an
    /// error: a project may configure no reference source, have one declared
    /// but not yet uploaded, or have no registered settings at all. The old
    /// `drofus_configured: false` said exactly this for the single-source
    /// world; an empty map says it for N.
    pub sources: BTreeMap<String, SourceValidation>,
    /// Rooms examined. A project-level fact, identical for every source, so it
    /// sits here rather than being repeated per section.
    pub total_rooms: usize,
    /// Tallies summed across every source — what the collapsed QA header
    /// counts. Each source's own breakdown is on its `SourceValidation`.
    pub discrepancies: DiscrepancyCounts,
    /// Whether this project's models agree on which Revit phase they were
    /// filtered to (see `PhaseReport`).
    ///
    /// A top-level field rather than an entry under `sources`, because a phase
    /// disagreement has no reference source: `sources` is keyed by source name
    /// and every other finding here is a room-versus-source reconciliation.
    /// This one is a room-versus-room problem.
    pub phases: PhaseReport,
    /// Whether this project's doors link to rooms that actually exist (see
    /// `DoorReport`).
    ///
    /// Top-level for the same reason `phases` is: `sources` is keyed by
    /// reference-source name, and this is a door-versus-room problem with no
    /// source in it.
    pub doors: DoorReport,
}

/// Which phase each of a project's models is on, and whether they agree.
///
/// **Why this is a finding at all.** `/rooms` merges every model's latest
/// snapshot, and nothing forces the models of one project onto the same phase —
/// enforcing that would deadlock, since moving a project from phase A to B
/// needs some model to go first and it would be refused for disagreeing with
/// the rest. So a project can legitimately serve a plan whose rooms come from
/// two different phases, looking complete and being quietly wrong. Under the
/// immutability rule that state is also *permanent* until someone activates a
/// re-phase, which is what makes it worth surfacing rather than waiting out.
///
/// Reported, never rejected — "signal, not error", the same stance an unmatched
/// reference key gets.
#[derive(Debug, Default, Serialize)]
pub struct PhaseReport {
    /// Model id → the phase its rooms were filtered to. `null` is a model whose
    /// snapshot predates phasing: its rooms were never filtered at all, which
    /// is a distinct (and worse) problem from disagreeing about which phase.
    pub by_model: BTreeMap<String, Option<String>>,
    /// True when the models do not all report the same phase — counting an
    /// unphased model as its own distinct value, since mixing filtered and
    /// unfiltered rooms is exactly the same class of problem as mixing two
    /// phases. False for a project with one model, or none.
    pub disagree: bool,
}

/// Which side of a door a room reference sits on. A typed field rather than a
/// bare string so a consumer grouping by side cannot be defeated by a spelling,
/// and so adding a third side (there isn't one) would be a compile error rather
/// than a silently-unhandled value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorSide {
    FromRoom,
    ToRoom,
}

/// One door that references no room on either side.
#[derive(Debug, Serialize)]
pub struct DoorWithoutRoom {
    /// **Carried on every door finding**, because a door id — like a room id —
    /// is unique only within one model. A bare door id would be ambiguous the
    /// moment a project has two models, and unresolvable back to an element.
    pub model_id: String,
    pub door_id: String,
}

/// One door room reference that names a room this model does not have.
#[derive(Debug, Serialize)]
pub struct UnresolvedRoomReference {
    pub model_id: String,
    pub door_id: String,
    pub side: DoorSide,
    /// The room id the door names, which nothing in this model's rooms matches.
    pub room_id: String,
}

/// One door whose **authored** room reference disagrees with the room the
/// project's `room_attribution` policy actually attributes it to.
///
/// Reported, never corrected: the two are different claims — the geometry says
/// where the door is, the authored value says which room the modeller considers
/// it to serve — and there is no general rule for which is right. On the House A
/// sample the disagreements are mostly doors where the geometry picks an
/// exterior or circulation space over the served room.
#[derive(Debug, Serialize)]
pub struct RoomReferenceMismatch {
    pub model_id: String,
    pub door_id: String,
    /// The value of the door property named by `[doors] room_reference_property`.
    pub authored: String,
    /// The room the policy attributed the door to, and its resolved `Number` —
    /// which is what `authored` is compared against.
    pub attributed_room_id: String,
    pub attributed_room_number: String,
}

/// Door discrepancy tallies, so a consumer needn't re-sum the lists.
#[derive(Debug, Default, Serialize)]
pub struct DoorDiscrepancyCounts {
    pub total: usize,
    pub doors_without_room_reference: usize,
    pub doors_unresolved_room: usize,
    pub doors_unattributed: usize,
    pub room_reference_mismatches: usize,
    /// Models, not doors — see `DoorReport::doors_phase_drift`.
    pub doors_phase_drift: usize,
    pub room_geometry_mismatches: usize,
}

/// Which of two snapshots was read first, and therefore which one has not been
/// checked against the other since.
///
/// **It names the *unverified* side, not the wrong one**, and the distinction
/// matters when acting on it. If the doors were read later than the rooms and
/// the two disagree, it may be because a door moved — in which case the rooms
/// data is perfectly current and re-pushing it changes nothing. Re-pushing the
/// older side is still the right first move, because it either fixes the
/// disagreement or proves it is not drift; the finding just must not claim more
/// than it knows, which is why both timestamps are reported rather than only a
/// verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleSide {
    /// The doors snapshot was read first: it has not seen the current rooms.
    Doors,
    /// The rooms snapshot was read first: it has not seen the current doors.
    Rooms,
    /// **Both were read in one run and still disagree**, so this is not drift at
    /// all — nobody needs to re-push, and the model itself is wrong. A
    /// multi-model push gives every model in a run one shared `taken_at`, which
    /// is what makes this distinguishable rather than a coincidence.
    Neither,
}

/// One door whose authored room reference resolves, and disagrees with where the
/// door physically is.
///
/// **The finding nothing else could see.** A dangling reference names a room the
/// model does not have and is already reported; this one names a room that
/// exists, so every id check passes while the door is attributed to a room it is
/// nowhere near. It is what rooms-moved-and-doors-did-not looks like from the
/// server: the ids still line up and the geometry no longer does.
///
/// **Not a swing check.** A door is attributed to the room it *serves*, which
/// the modeller decides and which is not always the room it opens into — a
/// cupboard off a corridor swings into the corridor and belongs to the cupboard.
/// `CLAUDE.md` forbids reconciling those two and this does not: the cupboard
/// door's insertion point is still on the cupboard/corridor boundary, so it
/// passes. What fails here is a room that has *moved away* from its door.
#[derive(Debug, Serialize)]
pub struct RoomGeometryMismatch {
    pub model_id: String,
    pub door_id: String,
    pub side: DoorSide,
    /// The room the model states this side opens onto.
    pub authored_room_id: String,
    /// The room the geometry puts on that side, `None` when it found none.
    /// Absent with the door still present means the door is no longer beside
    /// *any* room on that side.
    pub geometric_room: Option<crate::service::room_locator::RoomRef>,
    /// When each snapshot was read — the export's own timestamp, not the
    /// server's receipt time, which is what makes comparing them meaningful.
    pub doors_taken_at: String,
    pub rooms_taken_at: String,
    pub stale: StaleSide,
}

/// How many sides the geometry answered, and how many it could not.
///
/// **Without this the resolver is unauditable.** A project that turns resolution
/// on and sees nothing change cannot otherwise tell whether every door was
/// already answered, or whether every probe failed for the same reason.
#[derive(Debug, Default, Serialize)]
pub struct RoomResolutionCounts {
    /// Sides the model left absent and the geometry filled.
    pub derived: usize,
    /// Sides the model left absent and the geometry could not fill, by reason.
    /// `no_candidate` here is usually an **external door** and not a problem.
    pub unresolved: BTreeMap<String, usize>,
}

/// One model whose stored doors were filtered to a different phase than its
/// stored rooms.
///
/// **Stale data, not absent data** — which is why it is a finding rather than a
/// pending state. Both snapshots exist and both look complete; they simply
/// describe different phases of the building, so every reference between them is
/// answering a question about a model that no longer exists in that form.
///
/// It became reachable in two ways at once. A rooms push whose phase disagrees
/// is quarantined and *promotable*, so activating it moves the lineage while the
/// stored doors stay on the old phase. And doors may now be pushed before rooms
/// and phase a lineage themselves, so the rooms-first ordering that used to make
/// this rare is gone.
///
/// Nothing else notices: `PhaseReport` reads rooms snapshots only, and the door
/// reference checks resolve ids without asking what phase either side is on.
#[derive(Debug, Serialize)]
pub struct DoorPhaseDrift {
    pub model_id: String,
    /// The phase this model's stored **doors** were filtered to.
    pub doors_phase: Option<String>,
    /// The phase this model's stored **rooms** were filtered to. `None` is a
    /// snapshot from before phasing existed — its rooms were never filtered at
    /// all, which is a different and worse problem than disagreeing.
    pub rooms_phase: Option<String>,
}

/// A door whose model has no rooms snapshot yet, so its references cannot be
/// resolved either way.
///
/// **Not a finding, and the distinction is the whole point of this type.** A
/// doors push no longer requires its model's rooms to be on the server -- ingest
/// used to refuse one, and that gate is gone because the question it asked
/// ("can these references resolve *now*?") has a legitimate answer of "not
/// yet": rooms may arrive in a later push. Counting these as dangling would
/// report an entire doors-first push as broken, which is the loudest possible
/// way to be wrong about a normal state.
///
/// What separates this from a dangling reference is *which* thing is missing.
/// Here the model has no rooms at all, so nothing was checked. A dangling
/// reference names a room that its model demonstrably does not have, which
/// stays a finding.
#[derive(Debug, Serialize)]
pub struct PendingRoomReference {
    pub model_id: String,
    pub door_id: String,
}

/// Whether this project's doors link to rooms that actually exist.
///
/// **Two findings, and one deliberate non-finding.**
///
/// A door with **neither** side set references no room at all — it cannot be
/// attributed, scheduled, or used as a connectivity edge, so it is a finding.
///
/// A door naming a room this model does not have is a finding for a different
/// reason: the reference is *there* but dangling. It is the state a re-push
/// produces when rooms and doors drift apart — doors pushed against one rooms
/// snapshot, rooms then re-pushed without them — and nothing else would notice,
/// because the door itself is perfectly well-formed.
///
/// A door in a model with **no rooms snapshot at all** is the deliberate
/// non-finding that used to be impossible: ingest refused such a push, so this
/// report could treat "no rooms for this model" as "every reference dangles".
/// That gate is gone (see `handlers::check_doors_ingest`), so the state is now
/// ordinary — doors may simply have arrived first — and it is reported as
/// **pending** instead. This report is where the gate's question moved, and the
/// gain is that it is re-answered every time the data changes rather than once,
/// at the push, on the least information anyone will ever have.
///
/// A door with exactly **one** side set is **not** a finding. That is an
/// external door, and it is a normal state the contract carries deliberately
/// (`Opening::from_room`); 6 of the 26 doors in the House A sample are one-sided,
/// so flagging them would put a 23% false-positive rate on the first real data
/// this ever saw. It is counted, so a reader can see the shape of the model,
/// and left out of `discrepancies`.
///
/// **References resolve within one model, never across the project.** Room ids
/// are unique only within a model, so a door in model A naming room `2621156`
/// is asking about *A's* room `2621156` — a same-numbered room in model B is a
/// different room, and matching against it would turn a real dangling reference
/// into a false clean bill.
///
/// Reported, never rejected: "signal, not error", the same stance an unmatched
/// reference key gets.
#[derive(Debug, Default, Serialize)]
pub struct DoorReport {
    /// Doors examined across every model in this project. `0` covers both "no
    /// doors pushed" and "doors pushed but empty" — neither is an error, and a
    /// project that has never pushed doors simply has an empty report.
    pub total_doors: usize,
    /// Doors referencing no room on either side.
    pub doors_without_room_reference: Vec<DoorWithoutRoom>,
    /// Room references naming a room the door's own model does not have. One
    /// entry per dangling *side*, so a door dangling on both appears twice —
    /// they are two broken references, and a reader fixing them fixes two.
    ///
    /// Only ever populated for a model that **has** a rooms snapshot; a model
    /// still waiting for one contributes to `doors_pending_rooms` instead.
    pub doors_unresolved_room: Vec<UnresolvedRoomReference>,
    /// Doors whose model has no rooms snapshot yet — see `PendingRoomReference`.
    /// **Informational, not a discrepancy**, and one entry per door rather than
    /// per side: nothing about this door was checked, so there is no per-side
    /// answer to give.
    pub doors_pending_rooms: Vec<PendingRoomReference>,
    /// Models whose stored doors and stored rooms describe different phases —
    /// see `DoorPhaseDrift`. One entry per model, not per door: it is a fact
    /// about the two snapshots, and repeating it per door would bury every other
    /// finding under it.
    pub doors_phase_drift: Vec<DoorPhaseDrift>,
    /// Authored references that resolve but disagree with the geometry — see
    /// `RoomGeometryMismatch`. Empty when `[doors] room_resolution` is off,
    /// which is **not the same as clean**; the setting is echoed below so a
    /// reader can tell the two apart.
    pub room_geometry_mismatches: Vec<RoomGeometryMismatch>,
    /// What the geometry answered across this project — see
    /// `RoomResolutionCounts`.
    pub room_resolution_counts: RoomResolutionCounts,
    /// The resolution mode in force, echoed for the same reason
    /// `room_reference_property` is: off is a real state, and a reader must be
    /// able to tell "the check found nothing" from "the check did not run".
    pub room_resolution: RoomResolution,
    /// Doors with a room on exactly one side — external doors. **Informational,
    /// not a discrepancy**; see the type doc for why.
    pub doors_external: usize,
    /// The attribution policy in force, echoed so a reader can tell whether
    /// `doors_unattributed` below is a data problem or a policy consequence.
    pub room_attribution: RoomAttribution,
    /// Doors that **name a room but the policy declines to use it** — a
    /// `to_room`-only policy against a door that opens *from* somewhere and into
    /// nowhere.
    ///
    /// Deliberately excludes doors already in `doors_without_room_reference`, so
    /// the two lists never double-count the same door. Under the default chain
    /// this is therefore **always empty**: the chain uses whatever reference
    /// exists, so the only unattributed doors are the ones with no reference at
    /// all. A non-empty list means the configured policy is narrower than the
    /// data, which is a legitimate choice worth being able to see.
    pub doors_unattributed: Vec<DoorWithoutRoom>,
    /// Doors whose authored room reference disagrees with the attributed room.
    /// Empty when `[doors] room_reference_property` is unset — the check is
    /// off, which is not the same as clean, and the setting is echoed on the
    /// response so a reader can tell.
    pub room_reference_mismatches: Vec<RoomReferenceMismatch>,
    /// The door property the reconciliation read, or `None` when the check is
    /// disabled.
    pub room_reference_property: Option<String>,
    /// Tallies for the two findings above. Deliberately **not** added into
    /// `ValidationResponse::discrepancies`, which is documented as the sum
    /// across reference *sources* — a door is not a source, and folding it in
    /// would make that number mean two different things. Same separation
    /// `phases` has.
    pub discrepancies: DoorDiscrepancyCounts,
}

impl ValidationResponse {
    /// The "nothing configured" answer: no sources, no rooms counted, no
    /// discrepancies. Callers return this rather than an error — see
    /// `compute_project_validation`.
    fn nothing_to_reconcile() -> Self {
        Self {
            sources: BTreeMap::new(),
            total_rooms: 0,
            discrepancies: DiscrepancyCounts::default(),
            phases: PhaseReport::default(),
            doors: DoorReport::default(),
        }
    }
}

/// Which phase each of a project's models is on, and whether they agree.
///
/// Reads each model's phase off the **snapshot**, not the lineage's current
/// phase in the manifest — a snapshot pushed before phasing existed reports
/// `None` because its rooms genuinely were not filtered, and that remains true
/// after a later push phases the lineage.
///
/// Agreement folds case and whitespace, matching `contract::phases_agree`, so a
/// model differing only in spelling is not reported as a disagreement. The map
/// keeps the original casing, since that is what a reader recognises.
fn phase_report(project_id: &str, stored: &[(ModelKey, RoomPayload)]) -> PhaseReport {
    let mut by_model: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (key, payload) in stored {
        if payload.project.id != project_id {
            continue;
        }
        by_model.insert(key.model_id.clone(), payload.phase.clone());
    }
    let distinct: std::collections::BTreeSet<Option<String>> =
        by_model.values().map(|p| p.as_ref().map(|s| s.trim().to_lowercase())).collect();
    PhaseReport { by_model, disagree: distinct.len() > 1 }
}

/// Every door's geometric answer, resolved once by the caller so this module
/// does not need the store.
///
/// Keyed on `(model id, door id)` for the reason every door lookup here is: a
/// door id, like a room id, is unique only within one model.
pub type LocatedDoors = BTreeMap<(String, String), crate::service::room_locator::Sides>;

/// Compare each authored reference against where the door physically is, and
/// count what the geometry managed to answer.
///
/// **Runs over doors that stated a reference too**, unlike the read path, which
/// skips the probe when both sides are authored because nothing it found would
/// be used. Here the disagreement *is* the finding, so the probe is exactly what
/// is wanted.
///
/// A side the model left absent is not a mismatch — there is nothing to
/// disagree with — so it lands in `derived`/`unresolved` instead.
fn geometry_pass(
    project_id: &str,
    stored_rooms: &[(ModelKey, RoomPayload)],
    stored_doors: &[(ModelKey, DoorPayload)],
    located: &LocatedDoors,
    report: &mut DoorReport,
) {
    use crate::service::room_locator::Located;

    let rooms_taken_at: BTreeMap<&str, &str> = stored_rooms
        .iter()
        .filter(|(_, p)| p.project.id == project_id)
        .map(|(key, p)| (key.model_id.as_str(), p.snapshot.taken_at.as_str()))
        .collect();

    for (key, payload) in stored_doors.iter().filter(|(_, p)| p.project.id == project_id) {
        for door in &payload.doors {
            let Some(sides) = located.get(&(key.model_id.clone(), door.id.clone())) else {
                continue;
            };
            for (side, authored, derived) in [
                (DoorSide::FromRoom, door.from_room.as_deref(), &sides.from),
                (DoorSide::ToRoom, door.to_room.as_deref(), &sides.to),
            ] {
                let Some(authored) = authored else {
                    // Nothing stated: this is the fill case, not the drift case.
                    match derived {
                        Located::Found(_) => report.room_resolution_counts.derived += 1,
                        Located::Unresolved(why) => {
                            let label = serde_json::to_value(why)
                                .ok()
                                .and_then(|v| v.as_str().map(str::to_string))
                                .unwrap_or_else(|| "unknown".to_string());
                            *report.room_resolution_counts.unresolved.entry(label).or_default() += 1;
                        }
                    }
                    continue;
                };

                // The geometry finding *nothing* on a side the model named is
                // not evidence the model is wrong: an unplaced room, a door
                // with no direction, or a probe that cleared no wall all land
                // here. Only a geometry answer that names a different room is a
                // disagreement.
                let Located::Found(found) = derived else { continue };
                if found.model_id == key.model_id && found.room_id == authored {
                    continue;
                }

                let doors_taken_at = payload.snapshot.taken_at.as_str();
                let rooms_at = rooms_taken_at.get(key.model_id.as_str()).copied().unwrap_or("");
                report.room_geometry_mismatches.push(RoomGeometryMismatch {
                    model_id: key.model_id.clone(),
                    door_id: door.id.clone(),
                    side,
                    authored_room_id: authored.to_string(),
                    geometric_room: Some(found.clone()),
                    doors_taken_at: doors_taken_at.to_string(),
                    rooms_taken_at: rooms_at.to_string(),
                    stale: stale_side(doors_taken_at, rooms_at),
                });
            }
        }
    }
}

/// Which snapshot was read first. RFC3339 UTC ids compare lexically — the same
/// property the store's "lexical max is newest" rule already depends on — so
/// this needs no date parsing and cannot fail on a value the store accepted.
fn stale_side(doors_taken_at: &str, rooms_taken_at: &str) -> StaleSide {
    match doors_taken_at.cmp(rooms_taken_at) {
        std::cmp::Ordering::Less => StaleSide::Doors,
        std::cmp::Ordering::Greater => StaleSide::Rooms,
        std::cmp::Ordering::Equal => StaleSide::Neither,
    }
}

/// Reconcile one project's doors against its rooms — see `DoorReport`.
///
/// Rooms are indexed **per model** rather than into one project-wide set, which
/// is the whole correctness argument here: a project-wide set would silently
/// resolve a door in model A against a same-numbered room in model B and report
/// a dangling reference as clean.
///
/// Both sides are read off the stored *snapshots*, so this reports on the data
/// actually being served: doors pushed against an older rooms snapshot that has
/// since been replaced show up here, which is the drift case nothing else
/// notices.
///
/// Over the 100-line discovery threshold since the pending/dangling split, and
/// kept whole deliberately. The body is one pass over one door deciding between
/// five mutually-exclusive outcomes, and the *order* of those decisions is
/// load-bearing — external before pending, attribution before the room-dependent
/// checks. Splitting it into per-outcome helpers would hide that ordering behind
/// call sites, which is exactly the thing a reader has to see.
#[allow(clippy::too_many_lines)]
fn door_report(
    project_id: &str,
    stored_rooms: &[(ModelKey, RoomPayload)],
    stored_doors: &[(ModelKey, DoorPayload)],
    policy: &OpeningPolicy,
    builtin_defs: &[BuiltinPropertyDef],
    located: Option<&LocatedDoors>,
) -> DoorReport {
    let rooms_by_model: BTreeMap<&str, BTreeSet<&str>> = stored_rooms
        .iter()
        .filter(|(_, payload)| payload.project.id == project_id)
        .map(|(key, payload)| {
            (
                key.model_id.as_str(),
                payload.rooms.iter().map(|r| r.id.as_str()).collect::<BTreeSet<&str>>(),
            )
        })
        .collect();

    // The rooms themselves, for the authored-reference reconciliation: it needs
    // the attributed room's `Number`, which the id set above cannot supply.
    // Keyed on `(model, room)` for the reason every door lookup here is —
    // a room id is unique only within its model.
    let room_by_model: BTreeMap<(&str, &str), (&Room, &str)> = stored_rooms
        .iter()
        .filter(|(_, payload)| payload.project.id == project_id)
        .flat_map(|(key, payload)| {
            payload
                .rooms
                .iter()
                .map(move |r| ((key.model_id.as_str(), r.id.as_str()), (r, payload.model.source.as_str())))
        })
        .collect();

    let mut report = DoorReport { room_attribution: policy.room_attribution, ..DoorReport::default() };
    report.room_reference_property = policy.room_reference_property.clone();
    report.room_resolution = policy.room_resolution;

    // Phase drift is a fact about the two *snapshots*, so it is settled per
    // model before any door is looked at. A model with doors but no rooms is not
    // drift -- it is pending, and reported as such per door below.
    let rooms_phase_by_model: BTreeMap<&str, Option<&str>> = stored_rooms
        .iter()
        .filter(|(_, payload)| payload.project.id == project_id)
        .map(|(key, payload)| (key.model_id.as_str(), payload.phase.as_deref()))
        .collect();
    for (key, payload) in stored_doors.iter().filter(|(_, p)| p.project.id == project_id) {
        let Some(rooms_phase) = rooms_phase_by_model.get(key.model_id.as_str()) else {
            continue; // no rooms yet: pending, not drift
        };
        // Folded exactly as `contract::phases_agree` folds, so a model differing
        // only in spelling is not reported -- the same stance `phase_report`
        // takes, and the same one ingest takes when deciding agreement.
        if !crate::contract::phases_agree(payload.phase.as_deref(), *rooms_phase) {
            report.doors_phase_drift.push(DoorPhaseDrift {
                model_id: key.model_id.clone(),
                doors_phase: payload.phase.clone(),
                rooms_phase: rooms_phase.map(str::to_string),
            });
        }
    }

    for (key, payload) in stored_doors.iter().filter(|(_, p)| p.project.id == project_id) {
        // A model with doors but no rooms snapshot has nothing to resolve
        // against. That used to be unreachable — ingest refused the push — so
        // this treated it as "every reference dangles". It is now an ordinary
        // state (doors may arrive before rooms), and calling it dangling would
        // report a whole correct push as broken. See the `rooms` binding below
        // for what it skips and, more importantly, what it does not.
        let rooms = rooms_by_model.get(key.model_id.as_str());
        for door in &payload.doors {
            report.total_doors += 1;
            let sides = [
                (DoorSide::FromRoom, door.from_room.as_deref()),
                (DoorSide::ToRoom, door.to_room.as_deref()),
            ];

            match (door.from_room.as_deref(), door.to_room.as_deref()) {
                (None, None) => {
                    report
                        .doors_without_room_reference
                        .push(DoorWithoutRoom { model_id: key.model_id.clone(), door_id: door.id.clone() });
                    // Nothing to resolve — a door with no references cannot also
                    // have a dangling one, so it is reported once, not twice.
                    continue;
                }
                // Exactly one side: an external door. Counted, never flagged.
                (Some(_), None) | (None, Some(_)) => report.doors_external += 1,
                (Some(_), Some(_)) => {}
            }

            // Attribution. This door HAS a reference (the no-reference case
            // `continue`d above), so an empty result means the configured policy
            // declined to use it — a policy consequence, reported separately
            // from a data gap so the two are never confused.
            let owners = policy.room_attribution.owners(door.from_room.as_deref(), door.to_room.as_deref());
            if owners.is_empty() {
                report
                    .doors_unattributed
                    .push(DoorWithoutRoom { model_id: key.model_id.clone(), door_id: door.id.clone() });
                continue;
            }

            // Pending — this model's rooms have not arrived, so the two checks
            // below (does the named room exist, does its Number match the
            // authored one) have nothing to run against.
            //
            // **Only those two are skipped, and the ordering above is what makes
            // that true.** Whether a door is external, and whether the
            // attribution policy declines to use its reference, are facts about
            // the door alone — checked before this gate precisely so a
            // doors-first push does not quietly stop reporting them.
            let Some(rooms) = rooms else {
                report
                    .doors_pending_rooms
                    .push(PendingRoomReference { model_id: key.model_id.clone(), door_id: door.id.clone() });
                continue;
            };

            for (side, room_id) in sides {
                let Some(room_id) = room_id else { continue };
                if !rooms.contains(room_id) {
                    report.doors_unresolved_room.push(UnresolvedRoomReference {
                        model_id: key.model_id.clone(),
                        door_id: door.id.clone(),
                        side,
                        room_id: room_id.to_string(),
                    });
                }
            }

            // Reconcile the authored reference against the attributed room's
            // Number, when the project names a property to read it from.
            // Compared against the FIRST owner: under `both` the door is
            // attributed twice and an authored value can only ever name one of
            // them, so agreeing with either is agreement.
            // A blank value, or the literal "None" Revit stringifies an unset
            // parameter as, is "not authored" rather than an authored empty —
            // the same reading `lookup_property` gives a blank property.
            let Some(authored) = policy
                .room_reference_property
                .as_deref()
                .and_then(|p| door.properties.get(p))
                .map(|v| v.value.trim())
                .filter(|v| !v.is_empty() && *v != "None")
            else {
                continue;
            };

            let matched = owners.iter().any(|room_id| {
                room_by_model
                    .get(&(key.model_id.as_str(), *room_id))
                    .and_then(|(room, source)| lookup_property(*room, "Number", source, builtin_defs))
                    .is_some_and(|number| number.trim() == authored)
            });
            if !matched {
                let first = owners[0];
                let number = room_by_model
                    .get(&(key.model_id.as_str(), first))
                    .and_then(|(room, source)| lookup_property(*room, "Number", source, builtin_defs))
                    .unwrap_or_default();
                report.room_reference_mismatches.push(RoomReferenceMismatch {
                    model_id: key.model_id.clone(),
                    door_id: door.id.clone(),
                    authored: authored.to_string(),
                    attributed_room_id: first.to_string(),
                    attributed_room_number: number,
                });
            }
        }
    }

    if let Some(located) = located {
        geometry_pass(project_id, stored_rooms, stored_doors, located, &mut report);
    }

    report.discrepancies = DoorDiscrepancyCounts {
        total: report.doors_without_room_reference.len()
            + report.doors_unresolved_room.len()
            + report.doors_unattributed.len()
            + report.room_reference_mismatches.len()
            + report.doors_phase_drift.len()
            + report.room_geometry_mismatches.len(),
        doors_unattributed: report.doors_unattributed.len(),
        room_reference_mismatches: report.room_reference_mismatches.len(),
        doors_without_room_reference: report.doors_without_room_reference.len(),
        doors_unresolved_room: report.doors_unresolved_room.len(),
        doors_phase_drift: report.doors_phase_drift.len(),
        room_geometry_mismatches: report.room_geometry_mismatches.len(),
    };
    report
}

/// The declaration for one dRofus field label, if the settings carry one.
fn field_config<'a>(drofus_fields: &'a [ReferenceFieldConfig], label: &str) -> Option<&'a ReferenceFieldConfig> {
    drofus_fields.iter().find(|f| f.label == label)
}

/// The configured QA override for one dRofus field label, or `None` when the
/// column has no declaration, or a declaration with no `qa` set (both mean
/// the default: numeric-adaptive if both sides parse as a number, else exact
/// string match).
fn compare_mode(drofus_fields: &[ReferenceFieldConfig], label: &str) -> Option<CompareMode> {
    field_config(drofus_fields, label).and_then(|f| f.qa)
}

/// A copy of `s` with every non-ASCII character replaced by `?`, mirroring
/// duHast's `encode_ascii` step (Python's `str.encode("ascii", "replace")`,
/// see `Objects/base.py`'s `to_json_utf`) that every room value already went
/// through before it reached this service. Used to re-check a string-compare
/// mismatch: if narrowing the dRofus side the same lossy way makes it equal
/// to the room value, the two sides agree and the mismatch was purely an
/// artefact of that export step, not a real disagreement.
fn ascii_narrowed(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii() { c } else { '?' }).collect()
}

/// Phase 1 — resolve every room's link-property value. Returns the room
/// count, the ids of rooms that resolved no value at all
/// (`rooms_missing_link_value`), a map of resolved link value → every
/// `(room, source)` that resolved to it (so the caller can detect a value
/// shared by more than one room), and whether the property was **named by no
/// room at all** (`SourceValidation::link_property_absent_everywhere`).
/// Borrows the rooms out of `stored`.
///
/// Reads through `property_presence` rather than `lookup_property` — the two
/// agree on which rooms resolve a value, but only the former keeps `Absent`
/// and `Empty` apart, and that distinction is the entire difference between
/// "this property does not exist" and "nobody has filled it in".
fn resolve_link_values<'a>(
    project_id: &str,
    stored: &'a [(ModelKey, RoomPayload)],
    drofus: &ReferenceData,
    builtin_defs: &[BuiltinPropertyDef],
) -> (usize, Vec<String>, LinkValueIndex<'a>, bool) {
    let mut total_rooms = 0;
    let mut rooms_missing_link_value = Vec::new();
    let mut by_value: LinkValueIndex = BTreeMap::new();
    // Any room that carried the name at all, whether or not it held a value.
    let mut named_by_some_room = false;

    for (_key, payload) in stored {
        if payload.project.id != project_id {
            continue;
        }
        for room in &payload.rooms {
            total_rooms += 1;
            match property_presence(room, &drofus.link_property, &payload.model.source, builtin_defs) {
                PropertyPresence::Present(value) => {
                    named_by_some_room = true;
                    by_value.entry(value).or_default().push((room, &payload.model.source));
                }
                PropertyPresence::Empty => {
                    named_by_some_room = true;
                    rooms_missing_link_value.push(room.id.clone());
                }
                PropertyPresence::Absent => rooms_missing_link_value.push(room.id.clone()),
            }
        }
    }

    (total_rooms, rooms_missing_link_value, by_value, total_rooms > 0 && !named_by_some_room)
}

/// The typed comparison ladder for one reconciled field, each rung falling
/// through to the next on `None`: a `Date`-declared field is compared as
/// parsed instants first (two renderings of one moment agree); then
/// `numeric_match` when both sides parse as numbers; finally string equality
/// (with the ASCII-narrowing re-check that forgives duHast's lossy
/// `encode_ascii` export step — see `ascii_narrowed`). `Exact` mode skips both
/// typed rungs and forces the string comparison.
fn field_values_agree(reference_value: &str, room_value: &str, field_cfg: Option<&ReferenceFieldConfig>) -> bool {
    let exact_mode = field_cfg.and_then(|f| f.qa) == Some(CompareMode::Exact);
    let date = if exact_mode {
        None
    } else {
        field_cfg.filter(|f| f.field_type == FieldType::Date).and_then(|f| {
            let fmt = f.format.as_deref()?; // always Some on Date (validated at startup)
            let revit_fmt = f.revit_format.as_deref().unwrap_or(fmt);
            date_match(reference_value, room_value, fmt, revit_fmt)
        })
    };
    let numeric = if exact_mode || date.is_some() {
        None
    } else {
        numeric_match(reference_value, room_value)
    };
    match (date, numeric) {
        (Some(date_matches), _) => date_matches,
        (None, Some(numeric_matches)) => numeric_matches,
        (None, None) => {
            reference_value.trim() == room_value.trim() || ascii_narrowed(reference_value.trim()) == room_value.trim()
        }
    }
}

/// Phase 3 — which dRofus fields this pass actually checks: every row-1 label
/// except those overridden `Ignore` (a deliberate exclusion, hidden from this
/// report entirely rather than shown as "not checked"), each flagged with
/// whether row 2 mapped it to a Revit property.
fn compute_field_coverage(drofus: &ReferenceData, drofus_fields: &[ReferenceFieldConfig]) -> Vec<FieldCoverage> {
    let ignored: BTreeSet<&str> = drofus_fields
        .iter()
        .filter(|f| f.qa == Some(CompareMode::Ignore))
        .map(|f| f.label.as_str())
        .collect();
    drofus
        .all_labels
        .iter()
        .filter(|label| !ignored.contains(label.as_str()))
        .map(|label| FieldCoverage {
            label: label.clone(),
            checked: drofus.reconciliation.contains_key(label),
            revit_property: drofus.reconciliation.get(label).cloned(),
        })
        .collect()
}

/// Resolve the human-friendly detail (`ErrorRoomInfo`) for every room whose id
/// is in `error_ids`, in a single pass over the project's rooms. Number, name
/// and link value all go through `lookup_property` the same way
/// `resolve_link_values` resolves the link value — so canonical→raw resolution
/// (and the source dimension) stays consistent with the rest of the pass, and a
/// property that doesn't resolve degrades to `""` (the CSV shows a blank cell).
///
/// Keyed by `room_id`, which is only unique within a model — the same
/// pre-existing caveat the discrepancy lists already carry (a colliding id from
/// a second linked model resolves to whichever room is seen last). This is a
/// detail lookup for display, not an identity the checks depend on.
fn collect_error_rooms(
    project_id: &str,
    stored: &[(ModelKey, RoomPayload)],
    drofus: &ReferenceData,
    builtin_defs: &[BuiltinPropertyDef],
    error_ids: &BTreeSet<String>,
) -> BTreeMap<String, ErrorRoomInfo> {
    let mut error_rooms = BTreeMap::new();
    for (_key, payload) in stored {
        if payload.project.id != project_id {
            continue;
        }
        let source = &payload.model.source;
        for room in &payload.rooms {
            if !error_ids.contains(&room.id) {
                continue;
            }
            error_rooms.insert(
                room.id.clone(),
                ErrorRoomInfo {
                    number: lookup_property(room, "Number", source, builtin_defs).unwrap_or_default(),
                    name: lookup_property(room, "Name", source, builtin_defs).unwrap_or_default(),
                    link_value: lookup_property(room, &drofus.link_property, source, builtin_defs).unwrap_or_default(),
                },
            );
        }
    }
    error_rooms
}

/// Pure computation behind `compute_project_validation` — pulled out so it's
/// testable without a full `AppState`, same shape as `resolve_label_fields`.
///
/// Five checks, in order: (1) does every room resolve a value for the link
/// property (`resolve_link_values`); (2) among those that do, is the value
/// actually unique per room (a shared value is ambiguous — recorded, then
/// excluded from the rest); (3) does each remaining room's value find a
/// record; (4) for rooms that do, does every reconciled, non-`Ignore`d
/// property agree between the two sides (`field_values_agree`); and (5) the
/// reverse of (3) — which of the source's records did no room reach at all.
///
/// (5) is the only one that walks the source rather than the rooms, which is
/// exactly why it was missing: every other question starts from a room.
///
/// Also reports `field_coverage` (`compute_field_coverage`): which of the
/// source's fields this pass actually checks at all, for the panel's "what's
/// being QA'd" reference.
pub fn compute_validation(
    project_id: &str,
    stored: &[(ModelKey, RoomPayload)],
    reference: &ReferenceData,
    builtin_defs: &[BuiltinPropertyDef],
    fields: &[ReferenceFieldConfig],
) -> SourceValidation {
    let (_total_rooms, rooms_missing_link_value, by_value, link_property_absent_everywhere) =
        resolve_link_values(project_id, stored, reference, builtin_defs);

    let mut duplicate_link_values = Vec::new();
    let mut rooms_unmatched = Vec::new();
    let mut property_mismatches = Vec::new();
    let mut fields_absent_in_revit = Vec::new();
    let mut fields_empty_in_revit = Vec::new();

    for (value, rooms) in &by_value {
        if rooms.len() > 1 {
            duplicate_link_values.push(DuplicateLinkValue {
                value: value.clone(),
                room_ids: rooms.iter().map(|(r, _)| r.id.clone()).collect(),
            });
            continue; // ambiguous -- can't uniquely match, so no further checks
        }
        let (room, source) = rooms[0];
        let Some(record) = reference.by_id.get(value) else {
            rooms_unmatched.push(room.id.clone());
            continue;
        };
        for (label, revit_property) in &reference.reconciliation {
            if compare_mode(fields, label) == Some(CompareMode::Ignore) {
                continue;
            }
            // Normalize the dRofus side the same way `lookup_property`
            // already does for the Revit side: a blank cell is "no value
            // here", not a real empty-string value to compare against. A
            // dRofus-side absence isn't tracked further -- only Revit-side
            // absence is (see `MissingInRevit`'s doc comment for why).
            let Some(reference_value) = record.fields.get(label).filter(|s| !s.is_empty()) else {
                continue;
            };
            match property_presence(room, revit_property, source, builtin_defs) {
                PropertyPresence::Absent => fields_absent_in_revit.push(MissingInRevit {
                    room_id: room.id.clone(),
                    reference_id: value.clone(),
                    field: label.clone(),
                }),
                PropertyPresence::Empty => fields_empty_in_revit.push(MissingInRevit {
                    room_id: room.id.clone(),
                    reference_id: value.clone(),
                    field: label.clone(),
                }),
                PropertyPresence::Present(room_value) => {
                    if !field_values_agree(reference_value, &room_value, field_config(fields, label)) {
                        property_mismatches.push(PropertyMismatch {
                            room_id: room.id.clone(),
                            reference_id: value.clone(),
                            field: label.clone(),
                            room_value,
                            reference_value: reference_value.clone(),
                        });
                    }
                }
            }
        }
    }

    // Phase 4 — the reverse direction: records this source carries that no room
    // points at. Every check above walks the ROOMS and asks the source a
    // question; this is the only one that walks the source.
    //
    // `by_value` holds every link value the rooms resolved, whether or not it
    // matched a record — so subtracting it from the source's keys leaves
    // exactly the records nothing reached. That also gives the duplicate case
    // the right answer for free: a value with several rooms is present in
    // `by_value`, so its record counts as matched (ambiguously) and is not
    // re-reported here. `by_id` is a `BTreeMap`, so the result is ascending.
    let reference_unmatched: Vec<String> =
        reference.by_id.keys().filter(|id| !by_value.contains_key(*id)).cloned().collect();

    // Per-category counts (list lengths — duplicate counts as groups, matching
    // the panel's issue count) and their total, so a consumer needn't re-sum.
    let discrepancies = DiscrepancyCounts {
        total: rooms_missing_link_value.len()
            + duplicate_link_values.len()
            + rooms_unmatched.len()
            + reference_unmatched.len()
            + reference.duplicate_ids.len()
            + reference.blank_id_rows
            + property_mismatches.len()
            + fields_absent_in_revit.len()
            + fields_empty_in_revit.len(),
        rooms_missing_link_value: rooms_missing_link_value.len(),
        duplicate_link_values: duplicate_link_values.len(),
        rooms_unmatched: rooms_unmatched.len(),
        reference_unmatched: reference_unmatched.len(),
        reference_duplicate_ids: reference.duplicate_ids.len(),
        reference_blank_id_rows: reference.blank_id_rows,
        property_mismatches: property_mismatches.len(),
        fields_absent_in_revit: fields_absent_in_revit.len(),
        fields_empty_in_revit: fields_empty_in_revit.len(),
    };

    // Every room id that appears in any discrepancy list — the set the CSV
    // export needs number/name/link-value for.
    let mut error_ids: BTreeSet<String> = BTreeSet::new();
    error_ids.extend(rooms_missing_link_value.iter().cloned());
    error_ids.extend(duplicate_link_values.iter().flat_map(|d| d.room_ids.iter().cloned()));
    error_ids.extend(rooms_unmatched.iter().cloned());
    error_ids.extend(property_mismatches.iter().map(|m| m.room_id.clone()));
    error_ids.extend(fields_absent_in_revit.iter().map(|m| m.room_id.clone()));
    error_ids.extend(fields_empty_in_revit.iter().map(|m| m.room_id.clone()));
    let error_rooms = collect_error_rooms(project_id, stored, reference, builtin_defs, &error_ids);

    SourceValidation {
        link_property: reference.link_property.clone(),
        link_property_absent_everywhere,
        rooms_missing_link_value,
        duplicate_link_values,
        rooms_unmatched,
        reference_unmatched,
        reference_duplicate_ids: reference.duplicate_ids.clone(),
        reference_blank_id_rows: reference.blank_id_rows,
        property_mismatches,
        fields_absent_in_revit,
        fields_empty_in_revit,
        field_coverage: compute_field_coverage(reference, fields),
        discrepancies,
        error_rooms,
    }
}

/// Rooms in one project across every stored model. Counted here rather than
/// taken from `compute_validation`, because it is a project-level fact that
/// every source would otherwise report identically — see
/// `ValidationResponse::total_rooms`.
fn count_project_rooms(project_id: &str, stored: &[(ModelKey, RoomPayload)]) -> usize {
    stored
        .iter()
        .filter(|(_, payload)| payload.project.id == project_id)
        .map(|(_, payload)| payload.rooms.len())
        .sum()
}

/// Data-quality report for the header's validation panel — see
/// `ValidationResponse`/`compute_validation`. `drofus_configured: false` is a
/// normal, non-error result — covers both "no dRofus source configured for
/// this project" and "this project has no registered settings at all" (the
/// latter has no separate signal here, same as `list_buildings`) — and is
/// returned as `Ok`; a storage read failure is a real internal error and
/// surfaces as `ServiceError::Internal`, so the HTTP adapter can still map it
/// to 500 exactly as it does today.
pub fn compute_project_validation(state: &AppState, project_id: &str) -> Result<ValidationResponse, ServiceError> {
    let registry = state.settings();
    let Some(bundle) = registry.settings_for(project_id) else {
        return Ok(ValidationResponse::nothing_to_reconcile());
    };

    // Reconcile against every source that has data. A configured-but-not-yet-
    // uploaded source (`data: None`) is skipped rather than reported as a
    // failure — "declared, nothing uploaded yet" is a normal state, and the
    // same "signal, not error" policy the unmatched-key checks below follow.
    let loaded: Vec<(&String, &ReferenceData, &[ReferenceFieldConfig])> = bundle
        .reference
        .iter()
        .filter_map(|(name, src)| src.data.as_ref().map(|data| (name, data, src.fields.as_slice())))
        .collect();
    // One storage read for all of them: `all_snapshots` is the expensive call
    // here, and every source reconciles against the same room set.
    //
    // Read *before* the no-sources bail below, because the phase report is a
    // room-versus-room finding that owes nothing to reference data — a project
    // reconciling against nothing can still be serving two phases at once, and
    // that is exactly the project nobody would otherwise be watching.
    let stored = state.all_snapshots(Some(project_id)).map_err(ServiceError::Internal)?;
    let stored_doors = state.all_door_snapshots(Some(project_id)).map_err(ServiceError::Internal)?;

    let mut response = ValidationResponse::nothing_to_reconcile();
    // Both of these are room-versus-room and door-versus-room findings that owe
    // nothing to reference data, so they are computed *before* the no-sources
    // bail below — a project reconciling against nothing can still be serving
    // two phases at once, or serving doors that link to rooms it no longer has,
    // and that is exactly the project nobody would otherwise be watching.
    response.phases = phase_report(project_id, &stored);
    // Resolved before the report so `door_report` stays a pure function of the
    // data it is handed -- it never touches the store, which is what lets every
    // one of its tests build a scenario as two vectors.
    let located = super::doors::locate_project_doors(state, project_id, bundle.doors.room_resolution, &stored_doors)?;
    response.doors = door_report(
        project_id,
        &stored,
        &stored_doors,
        &bundle.doors,
        &bundle.builtin_properties,
        (!located.is_empty()).then_some(&located),
    );
    if loaded.is_empty() {
        return Ok(response);
    }

    response.total_rooms = count_project_rooms(project_id, &stored);
    for (name, data, fields) in loaded {
        let report = compute_validation(project_id, &stored, data, &bundle.builtin_properties, fields);
        response.discrepancies.add(&report.discrepancies);
        response.sources.insert(name.clone(), report);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CustomValue, Model, Project, Snapshot, SUPPORTED_SCHEMA};
    use crate::reference::ReferenceRecord;

    fn make_room(id: &str, name: &str, props: &[(&str, &str)]) -> Room {
        let mut properties = BTreeMap::new();
        for (k, v) in props {
            properties.insert(k.to_string(), CustomValue { value: v.to_string(), storage_type: None });
        }
        Room {
            id: id.to_string(),
            name: name.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            properties,
        }
    }

    fn make_payload(project_id: &str, rooms: Vec<Room>) -> (ModelKey, RoomPayload) {
        let key = ModelKey { project_id: project_id.to_string(), model_id: "m1".to_string() };
        let payload = RoomPayload {
            schema_version: SUPPORTED_SCHEMA,
            project: Project { id: project_id.to_string(), name: "P".to_string() },
            model: Model { id: "m1".to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-01-01T00:00:00Z".to_string() },
            phase: None,
            model_to_shared: None,
            room_boundary: None,
            levels: vec![],
            rooms,
        };
        (key, payload)
    }

    /// One model on a named phase, for the phase-report tests.
    fn on_phase(project_id: &str, model_id: &str, phase: Option<&str>) -> (ModelKey, RoomPayload) {
        let (_, payload) = make_payload(project_id, vec![]);
        let key = ModelKey { project_id: project_id.to_string(), model_id: model_id.to_string() };
        let payload = RoomPayload {
            model: Model { id: model_id.to_string(), name: "M".to_string(), source: "revit".to_string() },
            phase: phase.map(str::to_string),
            ..payload
        };
        (key, payload)
    }

    /// One model, or several agreeing ones, is not a finding. Agreement folds
    /// case and whitespace exactly as ingest does, so a model differing only in
    /// spelling must not be reported as a disagreement — that would be a false
    /// alarm on data ingest itself considers identical.
    #[test]
    fn test_phase_report_agreement_folds_case_and_whitespace() {
        let stored = vec![
            on_phase("p1", "arch", Some("New Construction")),
            on_phase("p1", "struct", Some("  NEW CONSTRUCTION ")),
        ];
        let report = phase_report("p1", &stored);
        assert!(!report.disagree, "same phase, different spelling, is not a disagreement");
        // The map keeps what was pushed -- a reader recognises the original.
        assert_eq!(report.by_model["struct"].as_deref(), Some("  NEW CONSTRUCTION "));
    }

    /// Two genuinely different phases in one project is the finding: `/rooms`
    /// merges them anyway, so a plan can span two phases and look complete. It
    /// is reported, never rejected — enforcing agreement across models would
    /// deadlock a project trying to move phase.
    #[test]
    fn test_phase_report_flags_two_phases_in_one_project() {
        let stored = vec![
            on_phase("p1", "arch", Some("New Construction")),
            on_phase("p1", "struct", Some("Existing")),
            // Another project's disagreement is not this project's finding.
            on_phase("p2", "other", Some("Demolition")),
        ];
        let report = phase_report("p1", &stored);
        assert!(report.disagree);
        assert_eq!(report.by_model.len(), 2, "scoped to this project's models");
    }

    /// An unphased model counts as its own value: mixing rooms that were never
    /// filtered with rooms that were is the same class of problem as mixing two
    /// phases, and arguably worse, so it must not read as agreement.
    #[test]
    fn test_phase_report_counts_an_unphased_model_as_disagreement() {
        let stored = vec![
            on_phase("p1", "arch", Some("New Construction")),
            on_phase("p1", "legacy", None),
        ];
        let report = phase_report("p1", &stored);
        assert!(report.disagree, "filtered + unfiltered is not agreement");
        assert_eq!(report.by_model["legacy"], None);

        // But a project that is uniformly unphased has nothing to disagree about.
        let all_legacy = vec![on_phase("p1", "a", None), on_phase("p1", "b", None)];
        assert!(!phase_report("p1", &all_legacy).disagree);
    }

    fn make_drofus(
        link_property: &str,
        records: &[(&str, &[(&str, &str)])],
        reconciliation: &[(&str, &str)],
    ) -> ReferenceData {
        let mut by_id = BTreeMap::new();
        // `all_labels` mirrors the real loader's row-1 label set: the union
        // of every reconciled label and every field label that shows up in
        // any record (the real CSV always has a row-1 label for a column
        // regardless of whether row 2 mapped it).
        let mut all_labels: BTreeSet<String> = BTreeSet::new();
        for (id, fields) in records {
            let mut f = BTreeMap::new();
            for (k, v) in *fields {
                f.insert(k.to_string(), v.to_string());
                all_labels.insert(k.to_string());
            }
            by_id.insert(id.to_string(), ReferenceRecord { fields: f });
        }
        let mut reconciliation_map = BTreeMap::new();
        for (k, v) in reconciliation {
            reconciliation_map.insert(k.to_string(), v.to_string());
            all_labels.insert(k.to_string());
        }
        ReferenceData {
            link_property: link_property.to_string(),
            by_id,
            reconciliation: reconciliation_map,
            all_labels: all_labels.into_iter().collect(),
            duplicate_ids: vec![],
            blank_id_rows: 0,
        }
    }

    /// A room with no value for the link property is reported, not silently
    /// dropped.
    #[test]
    fn test_compute_validation_missing_link_value() {
        let room = make_room("1", "Room", &[]); // no "Number" property
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(count_project_rooms("p1", &stored), 1);
        assert_eq!(result.rooms_missing_link_value, vec!["1".to_string()]);
        assert!(result.duplicate_link_values.is_empty());
    }

    /// The production failure this flag exists for: a link property no room
    /// carries. Every room lands in `rooms_missing_link_value` and every row
    /// in `reference_unmatched` — symptoms identical to catastrophically bad
    /// data — so the flag is what says "the join was never wired up".
    ///
    /// `resolve_raw_name` uses an unmapped canonical name verbatim, on
    /// purpose, so this is reachable from a plain settings typo with nothing
    /// else wrong anywhere.
    #[test]
    fn test_compute_validation_flags_a_link_property_no_room_carries() {
        let rooms = vec![
            make_room("1", "Room A", &[("Number", "101")]),
            make_room("2", "Room B", &[("Number", "102")]),
        ];
        let (key, payload) = make_payload("p1", rooms);
        let stored = vec![(key, payload)];
        // The rooms key on `Number`; the source asks for `Room Number`, and no
        // builtin_properties entry maps one to the other.
        let drofus = make_drofus("Room Number", &[("101", &[]), ("102", &[])], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.link_property_absent_everywhere, "named the fault");
        assert_eq!(result.rooms_missing_link_value.len(), 2, "and every room still listed");
        assert_eq!(result.reference_unmatched.len(), 2, "with every row unclaimed");
    }

    /// The distinction the flag turns on: a property that EXISTS and is blank
    /// everywhere is an ordinary data gap, however extreme, and must not be
    /// reported as misconfiguration. Same visible symptom, different cause,
    /// different fix — one is a settings edit, the other is data entry.
    #[test]
    fn test_compute_validation_empty_everywhere_is_not_a_config_fault() {
        let rooms = vec![
            make_room("1", "Room A", &[("Number", "")]),
            make_room("2", "Room B", &[("Number", "")]),
        ];
        let (key, payload) = make_payload("p1", rooms);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("101", &[])], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(!result.link_property_absent_everywhere, "present but blank is data, not config");
        assert_eq!(result.rooms_missing_link_value.len(), 2, "still reported as missing a value");
    }

    /// A project with no rooms concludes nothing — there is no evidence either
    /// way, and flagging it would fire on every project the moment a source is
    /// configured before its first push.
    #[test]
    fn test_compute_validation_no_rooms_is_not_a_config_fault() {
        let (key, payload) = make_payload("p1", vec![]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Room Number", &[("101", &[])], &[]);

        assert!(!compute_validation("p1", &stored, &drofus, &[], &[]).link_property_absent_everywhere);
    }

    /// Two rooms sharing one link value are ambiguous: reported as a
    /// duplicate, and excluded from the unmatched/mismatch checks (neither
    /// can be uniquely said to be the room a dRofus record describes).
    #[test]
    fn test_compute_validation_duplicate_excluded_from_other_checks() {
        let rooms = vec![
            make_room("1", "Room A", &[("Number", "101")]),
            make_room("2", "Room B", &[("Number", "101")]),
        ];
        let (key, payload) = make_payload("p1", rooms);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("101", &[])], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.duplicate_link_values.len(), 1);
        let dup = &result.duplicate_link_values[0];
        assert_eq!(dup.value, "101");
        assert_eq!(dup.room_ids, vec!["1".to_string(), "2".to_string()]);
        assert!(result.rooms_unmatched.is_empty());
        assert!(result.property_mismatches.is_empty());
    }

    /// A room whose (unique) link value isn't in the dRofus map is reported
    /// as unmatched.
    #[test]
    fn test_compute_validation_unmatched_in_drofus() {
        let room = make_room("1", "Room", &[("Number", "999")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[])], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.rooms_unmatched, vec!["1".to_string()]);
    }

    /// A uniquely-matched room: an agreeing reconciled field produces no
    /// mismatch, a disagreeing one does.
    #[test]
    fn test_compute_validation_property_mismatch_and_agreement() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "25.5"), ("Department", "Cardiology")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("NetArea", "30.0"), ("Dept", "Cardiology")])],
            &[("NetArea", "Area"), ("Dept", "Department")],
        );

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.rooms_unmatched.is_empty());
        assert_eq!(result.property_mismatches.len(), 1);
        let mismatch = &result.property_mismatches[0];
        assert_eq!(mismatch.field, "NetArea");
        assert_eq!(mismatch.room_value, "25.5");
        assert_eq!(mismatch.reference_value, "30.0");
    }

    /// A discrepant room carries its number/name/link-value in `error_rooms`
    /// (what the CSV export shows beyond the id), and the discrepancy counts
    /// tally the lists.
    #[test]
    fn test_compute_validation_error_rooms_and_counts() {
        let room = make_room("r1", "Office 101", &[("Number", "101"), ("Name", "Office"), ("Area", "25.5")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("101", &[("NetArea", "30.0")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        // One mismatch (Area 25.5 vs NetArea 30.0), and the counts reflect it.
        assert_eq!(result.property_mismatches.len(), 1);
        assert_eq!(result.discrepancies.property_mismatches, 1);
        assert_eq!(result.discrepancies.total, 1);

        // The mismatched room's detail: Revit Number/Name params + link value.
        let info = result.error_rooms.get("r1").expect("mismatched room has detail");
        assert_eq!(info.number, "101");
        assert_eq!(info.name, "Office");
        assert_eq!(info.link_value, "101");
    }

    /// A room missing its link value appears in `error_rooms` with an empty
    /// `link_value` (there is none to resolve), while its Name still resolves;
    /// the counts tally the missing-link category and the total.
    #[test]
    fn test_compute_validation_error_rooms_missing_link_value_blank() {
        let room = make_room("r1", "Office", &[("Name", "Office")]); // no "Number"
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[], &[]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.rooms_missing_link_value, vec!["r1".to_string()]);
        assert_eq!(result.discrepancies.rooms_missing_link_value, 1);
        assert_eq!(result.discrepancies.total, 1);

        let info = result.error_rooms.get("r1").expect("missing-link room has detail");
        assert_eq!(info.link_value, "", "no link value resolved → blank");
        assert_eq!(info.number, "", "no Number param → blank");
        assert_eq!(info.name, "Office");
    }

    /// The reported bug: the Revit export's ASCII-narrowing step replaces any
    /// non-ASCII character with `?` before the value reaches this service, so
    /// a room value that legitimately started with an en dash arrives as
    /// `?`. That must not be flagged once the dRofus side is narrowed the
    /// same lossy way and the two agree.
    #[test]
    fn test_compute_validation_ascii_narrowing_no_false_mismatch() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Department", "Loading Dock ? Option 2")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("Dept", "Loading Dock \u{2013} Option 2")])],
            &[("Dept", "Department")],
        );

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
    }

    /// A genuine content mismatch that merely happens to contain a literal
    /// `?` on the dRofus side must still be reported -- narrowing only
    /// rescues a mismatch when it's the *sole* difference, not any mismatch
    /// touching a `?` character.
    #[test]
    fn test_compute_validation_ascii_narrowing_does_not_mask_genuine_mismatch() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Department", "MECH")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("Dept", "SM.EX?")])], &[("Dept", "Department")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert_eq!(result.property_mismatches.len(), 1);
    }

    /// The reported bug: a unit-conversion float artifact (Revit's
    /// `"1.49999935417"` vs dRofus's `"1.5"`) must not be flagged once both
    /// are rounded to the lesser stated precision.
    #[test]
    fn test_compute_validation_numeric_tolerance_no_false_mismatch() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "1.49999935417")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "1.5")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
    }

    /// A blank dRofus cell must be treated as "no value here", not compared
    /// against Revit's real value -- previously this produced a false
    /// `""` vs `"25.5"` mismatch.
    #[test]
    fn test_compute_validation_empty_drofus_value_not_flagged() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "25.5")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_absent_in_revit.is_empty());
        assert!(result.fields_empty_in_revit.is_empty());
    }

    /// dRofus has a real value but the room has no such Revit property at
    /// all -- the serious case (mapping/model-setup problem), reported
    /// separately from a merely-blank value.
    #[test]
    fn test_compute_validation_field_absent_in_revit() {
        let room = make_room("1", "Room", &[("Number", "1")]); // no "Area" property at all
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "30.0")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_empty_in_revit.is_empty());
        assert_eq!(result.fields_absent_in_revit.len(), 1);
        assert_eq!(result.fields_absent_in_revit[0].field, "NetArea");
    }

    /// dRofus has a real value, the room's Revit property exists but is
    /// blank -- an ordinary per-room gap, reported separately from `Absent`.
    #[test]
    fn test_compute_validation_field_empty_in_revit() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "30.0")])], &[("NetArea", "Area")]);

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_absent_in_revit.is_empty());
        assert_eq!(result.fields_empty_in_revit.len(), 1);
        assert_eq!(result.fields_empty_in_revit[0].field, "NetArea");
    }

    /// A field overridden `Ignore` is skipped entirely: no mismatch, no
    /// absent/empty entry, and no row in the coverage report.
    #[test]
    fn test_compute_validation_ignore_override_skips_field_entirely() {
        let room = make_room("1", "Room", &[("Number", "1"), ("SyncTime", "2026-07-02")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("LastSync", "2026-06-29")])], &[("LastSync", "SyncTime")]);
        // Also declares the field's type -- proves `qa: Ignore` and `type:
        // Date` coexist: QA still skips it, independent of what a future
        // date-consuming feature would do with the same declaration.
        let drofus_fields = vec![crate::settings::ReferenceFieldConfig {
            label: "LastSync".to_string(),
            field_type: crate::settings::FieldType::Date,
            format: Some("%Y-%m-%d".to_string()),
            revit_format: None,
            qa: Some(CompareMode::Ignore),
        }];

        let result = compute_validation("p1", &stored, &drofus, &[], &drofus_fields);

        assert!(result.property_mismatches.is_empty());
        assert!(result.fields_absent_in_revit.is_empty());
        assert!(result.fields_empty_in_revit.is_empty());
        assert!(result.field_coverage.iter().all(|c| c.label != "LastSync"));
    }

    /// A `Date` field declaration for tests: the shipped dRofus pattern,
    /// optionally a distinct Revit-side pattern, optionally a QA override.
    fn date_field(label: &str, revit_format: Option<&str>, qa: Option<CompareMode>) -> ReferenceFieldConfig {
        ReferenceFieldConfig {
            label: label.to_string(),
            field_type: FieldType::Date,
            format: Some("%-m/%-d/%Y %-I:%M:%S %p %z".to_string()),
            revit_format: revit_format.map(|s| s.to_string()),
            qa,
        }
    }

    const DROFUS_DATE_FMT: &str = "%-m/%-d/%Y %-I:%M:%S %p %z";

    /// `date_match` with the shipped dRofus pattern: two renderings of the
    /// same instant agree, different instants disagree, and an unparseable
    /// side yields `None` (fall back to string comparison).
    #[test]
    fn test_date_match_same_instant_different_rendering() {
        // Same instant: 5:01:01 PM +10:00 == 7:01:01 AM +00:00.
        assert_eq!(
            date_match(
                "6/29/2026 5:01:01 PM +10:00",
                "6/29/2026 7:01:01 AM +00:00",
                DROFUS_DATE_FMT,
                DROFUS_DATE_FMT,
            ),
            Some(true)
        );
        assert_eq!(
            date_match(
                "6/29/2026 5:01:01 PM +10:00",
                "6/29/2026 5:01:02 PM +10:00",
                DROFUS_DATE_FMT,
                DROFUS_DATE_FMT,
            ),
            Some(false)
        );
        assert_eq!(
            date_match("not a date", "6/29/2026 5:01:01 PM +10:00", DROFUS_DATE_FMT, DROFUS_DATE_FMT),
            None
        );
    }

    /// A distinct `revit_format` parses the room side with its own pattern; a
    /// zoned dRofus side against a naive Revit side compares the zoned side's
    /// local wall-clock reading.
    #[test]
    fn test_date_match_revit_format_and_mixed_offset() {
        assert_eq!(
            date_match("6/29/2026 5:01:01 PM +10:00", "2026-06-29 17:01:01", DROFUS_DATE_FMT, "%Y-%m-%d %H:%M:%S",),
            Some(true)
        );
        assert_eq!(
            date_match("6/29/2026 5:01:01 PM +10:00", "2026-06-29 07:01:01", DROFUS_DATE_FMT, "%Y-%m-%d %H:%M:%S",),
            Some(false),
            "a naive side is a wall-clock reading, not a UTC instant"
        );
    }

    /// A `Date`-declared field where the two sides differ textually but
    /// denote the same instant produces no mismatch; `qa = "exact"` on the
    /// same field forces the textual comparison and reports it.
    #[test]
    fn test_compute_validation_date_field_same_instant_not_flagged() {
        let room = make_room("1", "Room", &[("Number", "1"), ("SyncTime", "6/29/2026 7:01:01 AM +00:00")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("LastSync", "6/29/2026 5:01:01 PM +10:00")])],
            &[("LastSync", "SyncTime")],
        );

        let typed = vec![date_field("LastSync", None, None)];
        let result = compute_validation("p1", &stored, &drofus, &[], &typed);
        assert!(result.property_mismatches.is_empty(), "same instant, different rendering: no mismatch");

        let exact = vec![date_field("LastSync", None, Some(CompareMode::Exact))];
        let result = compute_validation("p1", &stored, &drofus, &[], &exact);
        assert_eq!(result.property_mismatches.len(), 1, "exact mode forces the textual comparison");
    }

    /// A `Date` declaration whose values don't actually parse falls back to
    /// the string path -- the declaration is a hint, not truth, so a
    /// free-text value in a date-labeled column still compares as a string.
    #[test]
    fn test_compute_validation_date_field_unparseable_falls_back_to_string() {
        let room = make_room("1", "Room", &[("Number", "1"), ("SyncTime", "pending")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus("Number", &[("1", &[("LastSync", "pending")])], &[("LastSync", "SyncTime")]);

        let typed = vec![date_field("LastSync", None, None)];
        let result = compute_validation("p1", &stored, &drofus, &[], &typed);
        assert!(result.property_mismatches.is_empty(), "equal strings agree on the fallback path");
    }

    /// The coverage report shows every dRofus field: a reconciled one as
    /// checked (with its mapped Revit property), an unmapped one as
    /// unchecked.
    #[test]
    fn test_compute_validation_field_coverage() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Area", "25.5")]);
        let (key, payload) = make_payload("p1", vec![room]);
        let stored = vec![(key, payload)];
        let drofus = make_drofus(
            "Number",
            &[("1", &[("NetArea", "25.5"), ("Notes", "not mapped")])],
            &[("NetArea", "Area")],
        );

        let result = compute_validation("p1", &stored, &drofus, &[], &[]);

        let net_area = result.field_coverage.iter().find(|c| c.label == "NetArea").unwrap();
        assert!(net_area.checked);
        assert_eq!(net_area.revit_property.as_deref(), Some("Area"));

        let notes = result.field_coverage.iter().find(|c| c.label == "Notes").unwrap();
        assert!(!notes.checked);
        assert!(notes.revit_property.is_none());
    }

    /// Register a project whose settings carry `sources`, each a
    /// (name, data, field configs) triple, and store one payload for it.
    fn state_with_sources(rooms: Vec<Room>, sources: Vec<(&str, ReferenceData)>) -> AppState {
        let (_key, payload) = make_payload("p1", rooms);
        let reference = sources
            .into_iter()
            .map(|(name, data)| {
                (
                    name.to_string(),
                    crate::state::ProjectReferenceSource {
                        entity: crate::settings::ReferenceEntity::Rooms,
                        data: Some(data),
                        fields: vec![],
                    },
                )
            })
            .collect();
        let bundle = crate::state::ProjectSettings {
            reference,
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            hierarchy_exclusions: vec![],
        };
        let state = AppState::new(
            Box::new(crate::storage::MemStore::new()),
            std::collections::HashMap::from([("p1".to_string(), bundle)]),
            None,
        );
        state.set_snapshot(payload).unwrap();
        state
    }

    /// **The point of the generalization.** Two configured sources produce two
    /// reports, each reconciled against its OWN link property — not one report
    /// about whichever source happened to be called "drofus". `drofus` keys on
    /// Number and agrees; `ffe` keys on Code and disagrees on Finish, and each
    /// discrepancy lands in its own section.
    #[test]
    fn test_every_configured_source_gets_its_own_report() {
        let room = make_room("1", "Room", &[("Number", "1"), ("Code", "C1"), ("Area", "25.5"), ("Finish", "Vinyl")]);
        let drofus = make_drofus("Number", &[("1", &[("NetArea", "25.5")])], &[("NetArea", "Area")]);
        let ffe = make_drofus("Code", &[("C1", &[("FinishSpec", "Carpet")])], &[("FinishSpec", "Finish")]);
        let state = state_with_sources(vec![room], vec![("drofus", drofus), ("ffe", ffe)]);

        let result = compute_project_validation(&state, "p1").unwrap();

        assert_eq!(result.sources.keys().collect::<Vec<_>>(), vec!["drofus", "ffe"]);
        assert_eq!(result.total_rooms, 1, "counted once for the project, not once per source");

        let d = &result.sources["drofus"];
        assert_eq!(d.link_property, "Number");
        assert!(d.property_mismatches.is_empty(), "drofus agrees");

        let f = &result.sources["ffe"];
        assert_eq!(f.link_property, "Code", "each source reconciles on its own link property");
        assert_eq!(f.property_mismatches.len(), 1);
        assert_eq!(f.property_mismatches[0].reference_value, "Carpet");
        assert_eq!(f.property_mismatches[0].room_value, "Vinyl");

        // The header's one number is the sum, so a second source's problems
        // cannot hide behind a clean first one.
        assert_eq!(result.discrepancies.total, 1);
        assert_eq!(result.discrepancies.property_mismatches, 1);
    }

    /// **The gap this check closes.** A source carrying records no room
    /// reaches used to report nothing at all: every other check starts from a
    /// room, so records the rooms never mention were invisible. Two rooms
    /// against four records must surface the two orphans.
    #[test]
    fn test_records_no_room_reaches_are_reported() {
        let rooms = vec![
            make_room("r1", "One", &[("Number", "101")]),
            make_room("r2", "Two", &[("Number", "102")]),
        ];
        let (key, payload) = make_payload("p1", rooms);
        let stored = vec![(key, payload)];
        let reference = make_drofus(
            "Number",
            &[
                ("101", &[("NetArea", "10")]),
                ("102", &[("NetArea", "20")]),
                ("903", &[("NetArea", "30")]), // no room
                ("904", &[("NetArea", "40")]), // no room
            ],
            &[],
        );

        let result = compute_validation("p1", &stored, &reference, &[], &[]);

        assert_eq!(
            result.reference_unmatched,
            vec!["903".to_string(), "904".to_string()],
            "ascending, and only the orphans"
        );
        assert!(result.rooms_unmatched.is_empty(), "both rooms did find a record");
        assert_eq!(result.discrepancies.reference_unmatched, 2);
        assert_eq!(result.discrepancies.total, 2, "counts toward the header total");
    }

    /// A link value shared by several rooms is **matched**, not an orphan: the
    /// record does have rooms pointing at it, and the ambiguity is already
    /// reported once as a duplicate. Listing it again in the other direction
    /// would report one problem twice.
    #[test]
    fn test_a_duplicated_link_value_is_not_also_an_orphan() {
        let rooms = vec![
            make_room("r1", "One", &[("Number", "101")]),
            make_room("r2", "Two", &[("Number", "101")]), // same value
        ];
        let (key, payload) = make_payload("p1", rooms);
        let stored = vec![(key, payload)];
        let reference = make_drofus("Number", &[("101", &[("NetArea", "10")])], &[]);

        let result = compute_validation("p1", &stored, &reference, &[], &[]);

        assert_eq!(result.duplicate_link_values.len(), 1, "reported as ambiguous");
        assert!(result.reference_unmatched.is_empty(), "record 101 has rooms — not an orphan");
        assert_eq!(result.discrepancies.total, 1, "one problem, counted once");
    }

    /// A room that resolved a link value matching nothing is an orphan on the
    /// ROOM side only. The two directions are independent and both reported.
    #[test]
    fn test_both_directions_are_reported_independently() {
        let rooms = vec![make_room("r1", "One", &[("Number", "101")])];
        let (key, payload) = make_payload("p1", rooms);
        let stored = vec![(key, payload)];
        // The one room points at 101; the source only knows 999.
        let reference = make_drofus("Number", &[("999", &[("NetArea", "10")])], &[]);

        let result = compute_validation("p1", &stored, &reference, &[], &[]);

        assert_eq!(result.rooms_unmatched, vec!["r1".to_string()], "the room found no record");
        assert_eq!(result.reference_unmatched, vec!["999".to_string()], "the record found no room");
        assert_eq!(result.discrepancies.total, 2, "two distinct findings, not one");
    }

    /// A project with no reference source at all answers an empty map, not an
    /// error — the shape that replaced `drofus_configured: false`.
    #[test]
    fn test_no_configured_source_is_an_empty_map_not_an_error() {
        let state = state_with_sources(vec![make_room("1", "Room", &[])], vec![]);
        let result = compute_project_validation(&state, "p1").unwrap();
        assert!(result.sources.is_empty());
        assert_eq!(result.discrepancies.total, 0);

        // Same answer for a project that has no registered settings at all.
        let unknown = compute_project_validation(&state, "ghost").unwrap();
        assert!(unknown.sources.is_empty());
    }

    /// A source declared in settings but never uploaded (`data: None`) is
    /// skipped, not reported as a failure — "declared, nothing uploaded yet"
    /// is a normal state on the `Upload` origin.
    #[test]
    fn test_configured_but_unloaded_source_is_skipped() {
        let (_key, payload) = make_payload("p1", vec![make_room("1", "Room", &[("Number", "1")])]);
        let reference = BTreeMap::from([
            (
                "drofus".to_string(),
                crate::state::ProjectReferenceSource {
                    entity: crate::settings::ReferenceEntity::Rooms,
                    data: Some(make_drofus("Number", &[("1", &[])], &[])),
                    fields: vec![],
                },
            ),
            (
                "pending".to_string(),
                crate::state::ProjectReferenceSource {
                    entity: crate::settings::ReferenceEntity::Rooms,
                    data: None,
                    fields: vec![],
                },
            ),
        ]);
        let bundle = crate::state::ProjectSettings {
            reference,
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            hierarchy_exclusions: vec![],
        };
        let state = AppState::new(
            Box::new(crate::storage::MemStore::new()),
            std::collections::HashMap::from([("p1".to_string(), bundle)]),
            None,
        );
        state.set_snapshot(payload).unwrap();

        let result = compute_project_validation(&state, "p1").unwrap();
        assert_eq!(
            result.sources.keys().collect::<Vec<_>>(),
            vec!["drofus"],
            "the unloaded source contributes nothing"
        );
    }

    // ---------- doors ----------

    fn make_door(id: &str, from_room: Option<&str>, to_room: Option<&str>) -> crate::contract::Opening {
        crate::contract::Opening {
            id: id.to_string(),
            level_id: "1".to_string(),
            loops: vec![],
            from_room: from_room.map(str::to_string),
            to_room: to_room.map(str::to_string),
            // These helpers exercise references and properties, never placement,
            // so `None` here is the honest input rather than a stub: a door with
            // no position and no direction is a state the contract carries.
            insertion_point: None,
            through_wall_normal: None,
            type_id: "t1".to_string(),
            type_name: "Single".to_string(),
            properties: BTreeMap::new(),
            type_properties: BTreeMap::new(),
        }
    }

    fn door_with(
        id: &str,
        from_room: Option<&str>,
        to_room: Option<&str>,
        props: &[(&str, &str)],
    ) -> crate::contract::Opening {
        let mut door = make_door(id, from_room, to_room);
        for (k, v) in props {
            door.properties
                .insert(k.to_string(), CustomValue { value: v.to_string(), storage_type: None });
        }
        door
    }

    fn make_doors(project_id: &str, model_id: &str, doors: Vec<crate::contract::Opening>) -> (ModelKey, DoorPayload) {
        let key = ModelKey { project_id: project_id.to_string(), model_id: model_id.to_string() };
        let payload = DoorPayload {
            schema_version: crate::contract::SUPPORTED_DOOR_SCHEMA,
            project: Project { id: project_id.to_string(), name: "P".to_string() },
            model: Model { id: model_id.to_string(), name: "M".to_string(), source: "revit".to_string() },
            snapshot: Snapshot { taken_at: "2026-02-01T00:00:00Z".to_string() },
            phase: Some("New Construction".to_string()),
            model_to_shared: None,
            levels: vec![],
            doors,
        };
        (key, payload)
    }

    fn rooms_for(project_id: &str, model_id: &str, room_ids: &[&str]) -> (ModelKey, RoomPayload) {
        let (_, mut payload) = make_payload(project_id, room_ids.iter().map(|id| make_room(id, "R", &[])).collect());
        payload.model.id = model_id.to_string();
        (ModelKey { project_id: project_id.to_string(), model_id: model_id.to_string() }, payload)
    }

    /// `rooms_for` with the snapshot id stated, for the drift tests -- which are
    /// entirely about which of two snapshots was read first.
    fn rooms_at(project_id: &str, model_id: &str, room_ids: &[&str], taken_at: &str) -> (ModelKey, RoomPayload) {
        let (key, mut payload) = rooms_for(project_id, model_id, room_ids);
        payload.snapshot.taken_at = taken_at.to_string();
        (key, payload)
    }

    /// `make_doors` with the snapshot id stated.
    fn doors_at(
        project_id: &str,
        model_id: &str,
        doors: Vec<crate::contract::Opening>,
        taken_at: &str,
    ) -> (ModelKey, DoorPayload) {
        let (key, mut payload) = make_doors(project_id, model_id, doors);
        payload.snapshot.taken_at = taken_at.to_string();
        (key, payload)
    }

    /// `rooms_for` with the phase stated, for the phase-drift tests.
    fn phased_rooms(
        project_id: &str,
        model_id: &str,
        room_ids: &[&str],
        phase: Option<&str>,
    ) -> (ModelKey, RoomPayload) {
        let (key, mut payload) = rooms_for(project_id, model_id, room_ids);
        payload.phase = phase.map(str::to_string);
        (key, payload)
    }

    /// `make_doors` with the phase stated.
    fn phased_doors(
        project_id: &str,
        model_id: &str,
        doors: Vec<crate::contract::Opening>,
        phase: Option<&str>,
    ) -> (ModelKey, DoorPayload) {
        let (key, mut payload) = make_doors(project_id, model_id, doors);
        payload.phase = phase.map(str::to_string);
        (key, payload)
    }

    /// The clean case: every reference resolves, nothing is reported, and the
    /// one-sided door is counted as external rather than flagged.
    #[test]
    fn test_door_report_clean_project() {
        let rooms = vec![rooms_for("p1", "m1", &["r1", "r2"])];
        let doors = vec![make_doors(
            "p1",
            "m1",
            vec![
                make_door("d1", Some("r1"), Some("r2")),
                make_door("d2", None, Some("r1")),
            ],
        )];

        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.total_doors, 2);
        assert_eq!(report.doors_external, 1, "the one-sided door is external");
        assert_eq!(report.discrepancies.total, 0, "an external door is not a discrepancy");
    }

    /// **The finding the whole report exists for, half one.** A door with
    /// neither side set references no room at all.
    #[test]
    fn test_door_with_no_room_reference_is_flagged() {
        let rooms = vec![rooms_for("p1", "m1", &["r1"])];
        let doors = vec![make_doors("p1", "m1", vec![make_door("d1", None, None)])];

        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.discrepancies.doors_without_room_reference, 1);
        assert_eq!(report.doors_without_room_reference[0].door_id, "d1");
        assert_eq!(report.doors_without_room_reference[0].model_id, "m1");
        assert_eq!(report.doors_external, 0, "no sides at all is not 'external'");
        assert!(report.doors_unresolved_room.is_empty(), "reported once, not twice");
    }

    /// **Half two.** A reference naming a room this model does not have — the
    /// drift case a re-push produces, which nothing else would notice because
    /// the door itself is well-formed.
    #[test]
    fn test_door_referencing_a_missing_room_is_flagged_per_side() {
        let rooms = vec![rooms_for("p1", "m1", &["r1"])];
        let doors = vec![make_doors(
            "p1",
            "m1",
            vec![make_door("d1", Some("gone"), Some("also-gone"))],
        )];

        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.discrepancies.doors_unresolved_room, 2, "two broken references, two entries");
        let sides: Vec<DoorSide> = report.doors_unresolved_room.iter().map(|u| u.side).collect();
        assert_eq!(sides, vec![DoorSide::FromRoom, DoorSide::ToRoom]);
        assert_eq!(report.doors_unresolved_room[0].room_id, "gone");
    }

    /// **The correctness argument for indexing rooms per model.** A door in
    /// `m1` naming `r1` must not be resolved by a same-numbered room in `m2`:
    /// room ids are unique only within a model, so those are different rooms,
    /// and a project-wide set would report this dangling reference as clean.
    #[test]
    fn test_a_same_numbered_room_in_another_model_does_not_resolve() {
        let rooms = vec![rooms_for("p1", "m1", &["other"]), rooms_for("p1", "m2", &["r1"])];
        let doors = vec![make_doors("p1", "m1", vec![make_door("d1", Some("r1"), None)])];

        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.discrepancies.doors_unresolved_room, 1, "m2's r1 is a different room");
        assert_eq!(report.doors_unresolved_room[0].model_id, "m1");
    }

    /// Another project's doors are not this project's problem — the same
    /// scoping every other check here applies.
    #[test]
    fn test_door_report_is_scoped_to_the_project() {
        let rooms = vec![rooms_for("p1", "m1", &["r1"]), rooms_for("p2", "m1", &["x"])];
        let doors = vec![
            make_doors("p1", "m1", vec![make_door("d1", Some("r1"), None)]),
            make_doors("p2", "m1", vec![make_door("d2", None, None)]),
        ];

        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.total_doors, 1);
        assert_eq!(report.discrepancies.total, 0, "p2's broken door is not counted here");
    }

    /// **The decided default.** A door opening into a room is attributed to it;
    /// one that only opens *from* somewhere falls back to that; one with neither
    /// is homeless and is already reported as having no room reference, not
    /// double-reported as unattributed.
    #[test]
    fn test_default_attribution_is_to_room_then_from_room() {
        let rooms = vec![rooms_for("p1", "m1", &["r1", "r2", "r3"])];
        let doors = vec![make_doors(
            "p1",
            "m1",
            vec![
                make_door("both", Some("r1"), Some("r2")),
                make_door("to-only", None, Some("r2")),
                make_door("from-only", Some("r3"), None),
                make_door("homeless", None, None),
            ],
        )];
        let policy = OpeningPolicy::default();
        let report = door_report("p1", &rooms, &doors, &policy, &[], None);

        let owners = |from: Option<&str>, to: Option<&str>| {
            policy.room_attribution.owners(from, to).iter().map(|s| s.to_string()).collect::<Vec<_>>()
        };
        assert_eq!(owners(Some("r1"), Some("r2")), vec!["r2"], "opens INTO wins over opens from");
        assert_eq!(owners(None, Some("r2")), vec!["r2"]);
        assert_eq!(owners(Some("r3"), None), vec!["r3"], "falls back to opens-from");
        assert!(owners(None, None).is_empty(), "homeless");

        assert_eq!(report.discrepancies.doors_without_room_reference, 1);
        assert!(
            report.doors_unattributed.is_empty(),
            "the chain uses whatever exists, so nothing is unattributed for policy reasons"
        );
    }

    /// A **narrower** policy leaves doors unattributed that the chain would
    /// have attributed — reported separately from a data gap, because it is a
    /// policy consequence rather than a missing reference.
    #[test]
    fn test_a_narrower_policy_reports_unattributed_doors_separately() {
        let rooms = vec![rooms_for("p1", "m1", &["r1"])];
        let doors = vec![make_doors(
            "p1",
            "m1",
            vec![
                make_door("from-only", Some("r1"), None),
                make_door("homeless", None, None),
            ],
        )];
        let policy = OpeningPolicy { room_attribution: RoomAttribution::ToRoom, ..Default::default() };
        let report = door_report("p1", &rooms, &doors, &policy, &[], None);

        assert_eq!(report.doors_unattributed.len(), 1);
        assert_eq!(report.doors_unattributed[0].door_id, "from-only");
        assert_eq!(
            report.discrepancies.doors_without_room_reference, 1,
            "the homeless door is not double-counted"
        );
        assert_eq!(
            report.room_attribution,
            RoomAttribution::ToRoom,
            "the policy is echoed so the count is readable"
        );
    }

    /// `both` attributes a door between two rooms twice — the reason
    /// `owner_rooms` is a list rather than an `Option`.
    #[test]
    fn test_both_policy_attributes_twice() {
        let owners = RoomAttribution::Both.owners(Some("r1"), Some("r2"));
        assert_eq!(owners, vec!["r2", "r1"], "to_room first, then from_room");
        assert!(RoomAttribution::None.owners(Some("r1"), Some("r2")).is_empty(), "none attributes nothing");
    }

    /// **The reconciliation.** An authored room reference that disagrees with
    /// the attributed room is a finding; one that agrees is silent; and the
    /// check is off entirely when no property is named.
    #[test]
    fn test_authored_room_reference_is_reconciled_against_the_attributed_room() {
        let mut rooms = rooms_for("p1", "m1", &[]);
        rooms.1.rooms = vec![
            make_room("r1", "SERVED", &[("Number", "01.07")]),
            make_room("r2", "HALL", &[("Number", "01.12")]),
        ];
        let doors = vec![make_doors(
            "p1",
            "m1",
            vec![
                // Attributed to r2 (opens into), authored says r1 — a mismatch.
                door_with("disagrees", Some("r1"), Some("r2"), &[("Door Room Reference", "01.07")]),
                // Attributed to r2, authored agrees.
                door_with("agrees", Some("r1"), Some("r2"), &[("Door Room Reference", "01.12")]),
                // Revit's unset-parameter spelling is "not authored", not a mismatch.
                door_with("unset", Some("r1"), Some("r2"), &[("Door Room Reference", "None")]),
            ],
        )];

        let off = door_report("p1", &[rooms.clone()], &doors, &OpeningPolicy::default(), &[], None);
        assert!(off.room_reference_mismatches.is_empty(), "no property named — the check is off");
        assert!(off.room_reference_property.is_none());

        let policy = OpeningPolicy {
            room_reference_property: Some("Door Room Reference".to_string()),
            ..Default::default()
        };
        let on = door_report("p1", &[rooms], &doors, &policy, &[], None);
        assert_eq!(on.room_reference_mismatches.len(), 1);
        let m = &on.room_reference_mismatches[0];
        assert_eq!(m.door_id, "disagrees");
        assert_eq!(m.authored, "01.07");
        assert_eq!(m.attributed_room_id, "r2");
        assert_eq!(m.attributed_room_number, "01.12");
        assert_eq!(on.discrepancies.room_reference_mismatches, 1);
    }

    /// A project that has never pushed doors gets an empty report, not an
    /// error and not a missing field — the same "normal state" stance a
    /// configured-but-unuploaded reference source gets.
    #[test]
    fn test_no_doors_is_an_empty_report() {
        let rooms = vec![rooms_for("p1", "m1", &["r1"])];
        let report = door_report("p1", &rooms, &[], &OpeningPolicy::default(), &[], None);
        assert_eq!(report.total_doors, 0);
        assert_eq!(report.discrepancies.total, 0);
    }

    // ---------- phase drift ----------

    /// **Doors and rooms describing different phases is a finding.** Both
    /// snapshots exist and look complete, so nothing else notices: the phase
    /// report reads rooms only, and the reference checks resolve ids without
    /// asking what phase either side is on.
    #[test]
    fn test_doors_on_a_different_phase_than_their_rooms_is_a_finding() {
        let rooms = vec![phased_rooms("p1", "m1", &["r1"], Some("Stage 2"))];
        let doors = vec![phased_doors(
            "p1",
            "m1",
            vec![make_door("d1", Some("r1"), None)],
            Some("Stage 1"),
        )];
        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);

        assert_eq!(report.doors_phase_drift.len(), 1);
        assert_eq!(report.doors_phase_drift[0].model_id, "m1");
        assert_eq!(report.doors_phase_drift[0].doors_phase.as_deref(), Some("Stage 1"));
        assert_eq!(report.doors_phase_drift[0].rooms_phase.as_deref(), Some("Stage 2"));
        assert_eq!(report.discrepancies.doors_phase_drift, 1);
    }

    /// Folded exactly as ingest folds when it decides agreement, so a model
    /// differing only in spelling is not reported.
    #[test]
    fn test_phase_drift_folds_case_and_whitespace() {
        let rooms = vec![phased_rooms("p1", "m1", &["r1"], Some("New Construction"))];
        let doors = vec![phased_doors(
            "p1",
            "m1",
            vec![make_door("d1", Some("r1"), None)],
            Some("  new CONSTRUCTION "),
        )];
        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert!(report.doors_phase_drift.is_empty(), "same phase, different spelling");
    }

    /// One entry per model, not per door — it is a fact about the two
    /// snapshots, and repeating it per door would bury every other finding.
    #[test]
    fn test_phase_drift_is_reported_once_per_model() {
        let rooms = vec![phased_rooms("p1", "m1", &["r1"], Some("Stage 2"))];
        let doors = vec![phased_doors(
            "p1",
            "m1",
            vec![make_door("d1", Some("r1"), None), make_door("d2", Some("r1"), None)],
            Some("Stage 1"),
        )];
        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.doors_phase_drift.len(), 1, "two doors, one model, one finding");
    }

    /// A model with doors and no rooms yet is **pending**, not drift: there is
    /// no rooms phase to disagree with.
    #[test]
    fn test_a_model_awaiting_rooms_is_not_phase_drift() {
        let doors = vec![phased_doors(
            "p1",
            "m1",
            vec![make_door("d1", Some("r1"), None)],
            Some("Stage 1"),
        )];
        let report = door_report("p1", &[], &doors, &OpeningPolicy::default(), &[], None);
        assert!(report.doors_phase_drift.is_empty());
        assert_eq!(report.doors_pending_rooms.len(), 1);
    }

    // ---------- geometric drift ----------

    /// The geometric answer for one door, as `locate_project_doors` would have
    /// produced it. Built by hand so these tests state the scenario rather than
    /// constructing a store and a coordinate system to imply it.
    fn located(model: &str, door: &str, from: Option<&str>, to: Option<&str>) -> LocatedDoors {
        use crate::service::room_locator::{Located, RoomRef, Sides, Unresolved};
        let side = |r: Option<&str>| match r {
            Some(room) => Located::Found(RoomRef { model_id: model.into(), room_id: room.into() }),
            None => Located::Unresolved(Unresolved::NoCandidate),
        };
        BTreeMap::from([((model.to_string(), door.to_string()), Sides { from: side(from), to: side(to) })])
    }

    /// **The finding nothing else could see.** The authored reference resolves —
    /// room `r1` exists — so every id check passes while the door is nowhere
    /// near it. This is what rooms-moved-and-doors-did-not looks like from the
    /// server.
    #[test]
    fn test_an_authored_reference_that_disagrees_with_geometry_is_a_finding() {
        let rooms = vec![rooms_at("p1", "m1", &["r1", "r2"], "2026-02-01T00:00:00Z")];
        let doors = vec![doors_at(
            "p1",
            "m1",
            vec![make_door("d1", None, Some("r1"))],
            "2026-01-01T00:00:00Z",
        )];
        let report = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", None, Some("r2"))),
        );

        assert!(report.doors_unresolved_room.is_empty(), "the reference resolves — that is the point");
        assert_eq!(report.room_geometry_mismatches.len(), 1);
        let finding = &report.room_geometry_mismatches[0];
        assert_eq!(finding.authored_room_id, "r1");
        assert_eq!(finding.geometric_room.as_ref().unwrap().room_id, "r2");
        assert_eq!(finding.side, DoorSide::ToRoom);
        assert_eq!(report.discrepancies.room_geometry_mismatches, 1);
    }

    /// The doors were read first, so they are the side that has not seen the
    /// current rooms.
    #[test]
    fn test_the_older_snapshot_is_named_as_the_unverified_one() {
        let rooms = vec![rooms_at("p1", "m1", &["r1", "r2"], "2026-02-01T00:00:00Z")];
        let doors = vec![doors_at(
            "p1",
            "m1",
            vec![make_door("d1", None, Some("r1"))],
            "2026-01-01T00:00:00Z",
        )];
        let report = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", None, Some("r2"))),
        );
        assert_eq!(report.room_geometry_mismatches[0].stale, StaleSide::Doors);

        // Reversed, the other side is the unverified one.
        let rooms = vec![rooms_at("p1", "m1", &["r1", "r2"], "2026-01-01T00:00:00Z")];
        let doors = vec![doors_at(
            "p1",
            "m1",
            vec![make_door("d1", None, Some("r1"))],
            "2026-02-01T00:00:00Z",
        )];
        let report = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", None, Some("r2"))),
        );
        assert_eq!(report.room_geometry_mismatches[0].stale, StaleSide::Rooms);
    }

    /// **Equal timestamps mean it is not drift at all.** A multi-model push
    /// gives every model in a run one shared `taken_at`, so rooms and doors read
    /// together and still disagreeing is a modelling error — nobody needs to
    /// re-push, and the finding must not tell them to.
    #[test]
    fn test_snapshots_read_together_report_neither_as_stale() {
        let ts = "2026-02-01T00:00:00Z";
        let rooms = vec![rooms_at("p1", "m1", &["r1", "r2"], ts)];
        let doors = vec![doors_at("p1", "m1", vec![make_door("d1", None, Some("r1"))], ts)];
        let report = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", None, Some("r2"))),
        );
        assert_eq!(report.room_geometry_mismatches[0].stale, StaleSide::Neither);
        assert_eq!(report.room_geometry_mismatches[0].doors_taken_at, ts);
        assert_eq!(report.room_geometry_mismatches[0].rooms_taken_at, ts);
    }

    /// Agreement is not a finding, and neither is the geometry finding nothing:
    /// an unplaced room, a door with no direction and a probe that cleared no
    /// wall all land there, and none of them is evidence the model is wrong.
    #[test]
    fn test_agreement_and_an_unresolved_probe_are_both_silent() {
        let rooms = vec![rooms_at("p1", "m1", &["r1"], "2026-01-01T00:00:00Z")];
        let doors = vec![doors_at(
            "p1",
            "m1",
            vec![make_door("d1", None, Some("r1"))],
            "2026-01-01T00:00:00Z",
        )];

        let agrees = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", None, Some("r1"))),
        );
        assert!(agrees.room_geometry_mismatches.is_empty(), "geometry agrees");

        let silent = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", None, None)),
        );
        assert!(silent.room_geometry_mismatches.is_empty(), "the probe found nothing — not a disagreement");
    }

    /// A side the model left **absent** is the fill case, not the drift case:
    /// it is counted, never reported as a mismatch.
    #[test]
    fn test_an_absent_side_is_counted_as_derived_not_as_a_mismatch() {
        let rooms = vec![rooms_at("p1", "m1", &["r1", "r2"], "2026-01-01T00:00:00Z")];
        let doors = vec![doors_at(
            "p1",
            "m1",
            vec![make_door("d1", None, None)],
            "2026-01-01T00:00:00Z",
        )];
        let report = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", Some("r1"), Some("r2"))),
        );

        assert!(report.room_geometry_mismatches.is_empty(), "nothing authored to disagree with");
        assert_eq!(report.room_resolution_counts.derived, 2, "both sides filled");
    }

    /// Unresolved sides are broken down by reason, so a project that turns
    /// resolution on and sees nothing can tell "already answered" from "every
    /// probe failed the same way".
    #[test]
    fn test_unresolved_sides_are_counted_by_reason() {
        let rooms = vec![rooms_at("p1", "m1", &["r1"], "2026-01-01T00:00:00Z")];
        let doors = vec![doors_at(
            "p1",
            "m1",
            vec![make_door("d1", None, None)],
            "2026-01-01T00:00:00Z",
        )];
        let report = door_report(
            "p1",
            &rooms,
            &doors,
            &OpeningPolicy::default(),
            &[],
            Some(&located("m1", "d1", None, None)),
        );
        assert_eq!(report.room_resolution_counts.unresolved.get("no_candidate"), Some(&2));
        assert_eq!(report.room_resolution_counts.derived, 0);
    }

    /// Resolution off means the check did not run, which is **not the same as
    /// clean** — the mode is echoed so a reader can tell.
    #[test]
    fn test_resolution_off_is_reported_as_off_not_as_clean() {
        let rooms = vec![rooms_at("p1", "m1", &["r1"], "2026-01-01T00:00:00Z")];
        let doors = vec![doors_at(
            "p1",
            "m1",
            vec![make_door("d1", None, Some("r1"))],
            "2026-01-01T00:00:00Z",
        )];
        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.room_resolution, RoomResolution::Off);
        assert!(report.room_geometry_mismatches.is_empty());
    }

    /// **Doors whose model has no rooms are PENDING, not dangling** — the
    /// distinction that replaced the ingest gate.
    ///
    /// This state used to be unreachable (ingest refused a doors push to a model
    /// without rooms) and so was read as "every reference dangles". Doors may now
    /// arrive first, which makes it ordinary, and reporting a whole correct push
    /// as broken would be the loudest possible way to be wrong about it. So it
    /// costs no discrepancies at all, and is reported once per door rather than
    /// once per side: nothing was checked, so there is no side to blame.
    #[test]
    fn test_doors_whose_model_has_no_rooms_are_pending_not_dangling() {
        let doors = vec![make_doors("p1", "m1", vec![make_door("d1", Some("r1"), Some("r2"))])];
        let report = door_report("p1", &[], &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.discrepancies.doors_unresolved_room, 0, "nothing dangles — nothing was checked");
        assert_eq!(report.discrepancies.total, 0, "a pending model is not a finding");
        assert_eq!(report.doors_pending_rooms.len(), 1, "one entry per door, not per side");
        assert_eq!(report.doors_pending_rooms[0].model_id, "m1");
        assert_eq!(report.doors_pending_rooms[0].door_id, "d1");
    }

    /// The same door, once its model's rooms arrive and do not contain the ids
    /// it names, IS a finding. Pending and dangling are the two answers to
    /// "unresolvable", and only the second is a fault — which is exactly what
    /// the ingest gate could not distinguish.
    #[test]
    fn test_a_pending_reference_becomes_dangling_once_rooms_arrive() {
        let doors = vec![make_doors("p1", "m1", vec![make_door("d1", Some("r1"), Some("r2"))])];
        let rooms = vec![rooms_for("p1", "m1", &["something-else"])];
        let report = door_report("p1", &rooms, &doors, &OpeningPolicy::default(), &[], None);
        assert!(report.doors_pending_rooms.is_empty(), "the model has rooms now");
        assert_eq!(report.discrepancies.doors_unresolved_room, 2, "both sides name rooms it does not have");
    }

    /// A pending model still reports the facts that are about the door alone.
    /// A door with **no** reference on either side is a finding whether or not
    /// its rooms have arrived — nothing about the rooms would change that
    /// answer, and the ordering in `door_report` is what keeps it reported.
    #[test]
    fn test_a_pending_model_still_reports_doors_with_no_reference() {
        let doors = vec![make_doors("p1", "m1", vec![make_door("d1", None, None)])];
        let report = door_report("p1", &[], &doors, &OpeningPolicy::default(), &[], None);
        assert_eq!(report.doors_without_room_reference.len(), 1);
        assert!(report.doors_pending_rooms.is_empty(), "it has no reference to be pending about");
    }

    /// The door report reaches the response even when the project configures no
    /// reference source at all — it is a door-versus-room finding that owes
    /// nothing to reference data, and a project reconciling against nothing is
    /// exactly the one nobody would otherwise be watching.
    #[test]
    fn test_door_report_survives_the_no_sources_bail() {
        let bundle = crate::state::ProjectSettings {
            reference: BTreeMap::new(),
            hierarchy: vec![],
            builtin_properties: vec![],
            room_label: vec![],
            milestones: vec![],
            comparison_key: None,
            comparison_properties: vec![],
            areas: Default::default(),
            doors: Default::default(),
            windows: Default::default(),
            hierarchy_exclusions: vec![],
        };
        let state = AppState::new(
            Box::new(crate::storage::MemStore::new()),
            std::collections::HashMap::from([("p1".to_string(), bundle)]),
            None,
        );
        let (_, rooms) = rooms_for("p1", "m1", &["r1"]);
        state.set_snapshot(rooms).unwrap();
        let (_, doors) = make_doors("p1", "m1", vec![make_door("d1", None, None)]);
        state.set_door_snapshot(doors).unwrap();

        let result = compute_project_validation(&state, "p1").unwrap();
        assert!(result.sources.is_empty(), "no reference source configured");
        assert_eq!(result.doors.discrepancies.doors_without_room_reference, 1, "reported anyway");
        assert_eq!(
            result.discrepancies.total, 0,
            "door findings stay out of the cross-source total, which counts sources"
        );
    }
}
