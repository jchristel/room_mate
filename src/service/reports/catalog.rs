//! What a report may ask for: the columns each entity offers (its intrinsics, its measures, the project's stored properties and the
//! reference sources' fields) and their value types (`column_catalog`).

use super::*;

/// One thing a column or a condition may name.
#[derive(Debug, Clone, Serialize)]
pub struct ColumnInfo {
    pub name: String,
    /// `property` (from the model), `intrinsic` (a field of the record itself),
    /// `reference` (a joined source's label) or `measure` (what the join
    /// produced). A picker groups by this; a reader learns from it why a name
    /// exists at all.
    pub kind: &'static str,
    /// `text` or `number`, which decides the operators a filter offers.
    pub value_type: &'static str,
}

/// What a report over one entity may name, in this project.
#[derive(Debug, Clone, Serialize)]
pub struct ColumnCatalog {
    pub entity: Vec<ColumnInfo>,
    /// The room side, for a by-room report. Empty for an entity that cannot be
    /// reported by room.
    pub rooms: Vec<ColumnInfo>,
    /// What the join measured, empty where it measured nothing.
    pub measures: Vec<ColumnInfo>,
}

/// The intrinsics each entity answers — fields of the record rather than
/// properties of the model.
///
/// **A code fact, so it is a list here rather than a read.** Each one is an arm
/// in that entity's `FilterTarget::presence`, and the two must stay in step:
/// a name here that no arm answers is a column that silently resolves to
/// nothing.
pub(super) fn intrinsics(entity: Entity) -> Vec<ColumnInfo> {
    let text = |name: &str| ColumnInfo { name: name.to_string(), kind: "intrinsic", value_type: "text" };
    let number = |name: &str| ColumnInfo { name: name.to_string(), kind: "intrinsic", value_type: "number" };
    match entity {
        Entity::Rooms | Entity::Spaces => vec![text("$id"), text("$name"), text("$level_id"), text("$level")],
        Entity::Doors | Entity::Windows => vec![
            text("$id"),
            text("$type_name"),
            text("$type_id"),
            text("$level_id"),
            text("$level"),
            text("$from_room"),
            text("$to_room"),
        ],
        Entity::Ceilings | Entity::Floors => vec![
            text("$id"),
            text("$type_name"),
            text("$type_id"),
            text("$level_id"),
            text("$level"),
            number("$height_offset"),
        ],
        Entity::Ffe => vec![
            text("$id"),
            text("$category"),
            text("$type_name"),
            text("$type_id"),
            text("$level_id"),
            text("$level"),
            text("$room"),
        ],
    }
}

/// What the join produced, per entity. Serving it keeps the page from holding
/// its own copy of a list only the server can be right about.
pub(super) fn measures(entity: Entity) -> Vec<ColumnInfo> {
    let number = |name: &str| ColumnInfo { name: name.to_string(), kind: "measure", value_type: "number" };
    match entity {
        Entity::Ceilings | Entity::Floors => vec![
            number("overlap_area"),
            number("fraction_of_room"),
            number("fraction_of_element"),
            number("mean_width"),
        ],
        Entity::Ffe => vec![ColumnInfo { name: "room_origin".to_string(), kind: "measure", value_type: "text" }],
        // The key a space matched on, and whether that key named more than one
        // room. Both are text: the key is a Revit value and the flag is a
        // sentence, because a reader scanning a sheet should not have to know
        // what `true` meant.
        Entity::Spaces => vec![
            ColumnInfo { name: "match_key".to_string(), kind: "measure", value_type: "text" },
            ColumnInfo { name: "ambiguous_key".to_string(), kind: "measure", value_type: "text" },
        ],
        _ => Vec::new(),
    }
}

/// Revit's storage type, as a filter cares about it. Anything that is not a
/// number is text, including an `ElementId` — you compare it, you do not order
/// it.
pub(super) fn value_type(storage_type: Option<&str>) -> &'static str {
    match storage_type {
        Some(t) if t.eq_ignore_ascii_case("Double") || t.eq_ignore_ascii_case("Integer") => "number",
        _ => "text",
    }
}

/// Every property name this project's snapshots of one kind carry.
///
/// **Read from each snapshot's last line, never by assembling the entity.**
/// The dictionary is written last precisely so this costs a tail read; see
/// `SnapshotStore::get_latest_trailer`. A snapshot stored before the property
/// codec has no dictionary and contributes nothing — its properties are still
/// filterable, they just cannot be *offered*, which is why the picker keeps
/// taking free text.
pub(super) fn stored_properties(state: &AppState, project: Option<&str>, kind: SnapshotKind) -> Vec<ColumnInfo> {
    let Ok(index) = state.model_index() else {
        return Vec::new();
    };
    let mut seen: BTreeMap<String, &'static str> = BTreeMap::new();
    for row in index.iter().filter(|r| project.is_none_or(|p| r.key.project_id == p)) {
        let Ok(Some(trailer)) = state.store().get_latest_trailer(kind, &row.key) else {
            continue;
        };
        let Ok(decoder) = crate::contract::property_codec::Decoder::from_trailer(&trailer) else {
            continue;
        };
        for (name, storage_type) in decoder.vocabulary() {
            seen.entry(name.to_string()).or_insert_with(|| value_type(storage_type));
        }
    }
    seen.into_iter()
        .map(|(name, value_type)| ColumnInfo { name, kind: "property", value_type })
        .collect()
}

/// The labels a joined reference source contributes, for the sources scoped to
/// this entity. Named `<source>.<label>`, which is the flat namespace a filter
/// and a column already share.
pub(super) fn reference_labels(state: &AppState, project: Option<&str>, entity: Entity) -> Vec<ColumnInfo> {
    let registry = state.settings();
    let Some(bundle) = project.and_then(|p| registry.settings_for(p)) else {
        return Vec::new();
    };
    let wanted = match entity {
        Entity::Rooms => crate::settings::ReferenceEntity::Rooms,
        Entity::Doors => crate::settings::ReferenceEntity::Doors,
        Entity::Windows => crate::settings::ReferenceEntity::Windows,
        Entity::Ffe => crate::settings::ReferenceEntity::Ffe,
        Entity::Spaces => crate::settings::ReferenceEntity::Spaces,
        // No source declares a surface today; the day one does, this is where
        // it starts being offered.
        Entity::Ceilings | Entity::Floors => return Vec::new(),
    };
    let mut out = Vec::new();
    for (name, source) in &bundle.reference {
        if source.entity != wanted {
            continue;
        }
        let Some(data) = &source.data else { continue };
        for label in &data.all_labels {
            out.push(ColumnInfo {
                name: format!("{name}.{label}"),
                kind: "reference",
                value_type: reference_value_type(&source.fields, label),
            });
        }
    }
    out
}

/// A reference label's type, from the field config that declares it.
///
/// **The declaration is the answer where there is one.** A CSV column is text
/// on the wire whatever it holds, so nothing about the data says which
/// operators a filter should offer — but `[[sources.reference.<name>.fields]]`
/// already states `type` for the fields QA compares, and that is the same
/// question asked for a different reason. A label with no config is text: the
/// safe half, since every text operator works on a number written as one, while
/// offering `>` on a name would silently match nothing.
///
/// A `Date` is text here rather than a third kind. Ordering dates as strings is
/// right only for ISO-8601 and the config carries a `format` precisely because
/// they are not all ISO, so `>` would be wrong often enough to be worse than
/// absent.
pub(super) fn reference_value_type(fields: &[crate::settings::ReferenceFieldConfig], label: &str) -> &'static str {
    match fields.iter().find(|f| f.label == label).map(|f| f.field_type) {
        Some(crate::settings::FieldType::Numeric) => "number",
        _ => "text",
    }
}

/// What a report over this entity may name, in this project.
pub fn column_catalog(state: &AppState, project: Option<&str>, entity: Entity) -> ColumnCatalog {
    let mut columns = intrinsics(entity);
    columns.extend(stored_properties(state, project, entity.kinds(false)[0]));
    columns.extend(reference_labels(state, project, entity));

    let rooms = if entity.joins_rooms() {
        let mut rooms = intrinsics(Entity::Rooms);
        rooms.extend(stored_properties(state, project, SnapshotKind::Rooms));
        rooms.extend(reference_labels(state, project, Entity::Rooms));
        rooms
    } else {
        Vec::new()
    };

    ColumnCatalog { entity: columns, rooms, measures: measures(entity) }
}
