//! How a snapshot is read and written as bytes: the room and opening payload parsers, `encode_snapshot` / `decode_snapshot` and the
//! dictionary on the LAST line (`property_names_of`). The store never sees a payload type; this is the serde layer.

use super::*;

/// The store-side description of a rooms push, derived from the payload.
///
/// This function *is* the typed layer `SnapshotStore` gave up: the store takes
/// bytes and needs five facts alongside them, and these are where those five
/// facts come from for rooms. Kept next to `parse_rooms` so the two halves of
/// the round-trip are read together.
pub(super) fn rooms_meta<'a>(key: &'a ModelKey, payload: &'a RoomPayload) -> SnapshotMeta<'a> {
    SnapshotMeta {
        kind: SnapshotKind::Rooms,
        key,
        project_name: &payload.project.name,
        model_name: &payload.model.name,
        taken_at: &payload.snapshot.taken_at,
        phase: payload.phase.as_deref(),
        model_to_shared: payload.model_to_shared,
    }
}

/// Parse stored bytes back into a `RoomPayload`, naming the model in the error.
///
/// The store used to name the *file path* here, which it can no longer do. The
/// model key is the better identifier anyway: it is what a reader would search
/// the settings and the manifest for, and it stays meaningful for a store that
/// has no paths at all.
pub(super) fn parse_rooms(key: &ModelKey, bytes: &[u8]) -> anyhow::Result<RoomPayload> {
    decode_snapshot(bytes).with_context(|| format!("malformed rooms snapshot for {}/{}", key.project_id, key.model_id))
}

/// `parse_rooms`' spaces counterpart. A separate function rather than a generic
/// one so the error names the entity a reader is actually looking for: a
/// services model can hold both, and "malformed snapshot" would not say which.
pub(super) fn parse_spaces(key: &ModelKey, bytes: &[u8]) -> anyhow::Result<SpacePayload> {
    decode_snapshot(bytes).with_context(|| format!("malformed spaces snapshot for {}/{}", key.project_id, key.model_id))
}

/// `rooms_meta`'s counterpart for doors. Deliberately a second small function
/// rather than a trait over the two payloads: they share five of six fields
/// today, but the moment a third entity's meta differs at all, a trait would
/// have to grow an escape hatch — and there is nothing here to reuse but six
/// field reads.
pub(super) fn doors_meta<'a>(key: &'a ModelKey, payload: &'a DoorPayload) -> SnapshotMeta<'a> {
    SnapshotMeta {
        kind: SnapshotKind::Doors,
        key,
        project_name: &payload.project.name,
        model_name: &payload.model.name,
        taken_at: &payload.snapshot.taken_at,
        phase: payload.phase.as_deref(),
        model_to_shared: payload.model_to_shared,
    }
}

pub(super) fn parse_doors(key: &ModelKey, bytes: &[u8]) -> anyhow::Result<DoorPayload> {
    parse_opening(SnapshotKind::Doors, key, bytes)
}

/// Parse one stored opening snapshot, naming the kind in the failure.
///
/// The kind is carried purely for the message. It is worth the parameter: the
/// three entities' snapshots are structurally similar enough that "malformed
/// snapshot for p/m" would leave a reader guessing which file to open, and this
/// is the error that fires at boot when a store has been hand-edited.
pub(super) fn parse_opening<P: serde::de::DeserializeOwned>(
    kind: SnapshotKind,
    key: &ModelKey,
    bytes: &[u8],
) -> anyhow::Result<P> {
    decode_snapshot(bytes)
        .with_context(|| format!("malformed {} snapshot for {}/{}", kind.label(), key.project_id, key.model_id))
}

/// A whole payload as stored bytes: its property maps encoded against a
/// dictionary, and the dictionary appended as the last line.
///
/// The buffered twin of `StreamingSnapshot`, and the two must produce the same
/// layout -- the last line is how `decode_snapshot` finds the dictionary, so a
/// buffered write that put it anywhere else would be read back as a legacy
/// snapshot and fail on its first triple. Compact rather than pretty for the
/// same reason: `to_vec_pretty` ends on a line of its own that is not the
/// trailer.
pub(super) fn encode_snapshot<T: serde::Serialize>(payload: &T) -> anyhow::Result<Vec<u8>> {
    let mut encoder = Encoder::default();
    let mut bytes = encoder.encode(|| serde_json::to_vec(payload))?;
    // A compact top-level object ends on its closing brace; the trailer goes
    // inside it, on a line of its own.
    anyhow::ensure!(
        bytes.len() > 2 && bytes.last() == Some(&b'}'),
        "a snapshot must serialise to a non-empty JSON object"
    );
    bytes.pop();
    bytes.push(b'\n');
    bytes.extend(encoder.trailer()?);
    bytes.extend_from_slice(b"}\n");
    Ok(bytes)
}

/// Parse stored snapshot bytes of either layout.
///
/// **The last line decides.** A snapshot written through `property_codec` ends
/// on its dictionary line; one written before ends on `]}` or `}`. Only the last
/// line is ever inspected, so telling the two apart costs nothing on a 300 MB
/// legacy file, and a legacy file that happens to contain the marker text
/// somewhere inside it is not mistaken for an encoded one.
///
/// A last line that carries the marker but is not a dictionary falls back to the
/// legacy parse, whose error then names what is actually wrong with the file.
pub(super) fn decode_snapshot<P: serde::de::DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<P> {
    let decoder = trailer_line(bytes)
        .and_then(|line| Decoder::from_trailer(line).ok())
        .unwrap_or_else(Decoder::legacy);
    decoder.decode(|| serde_json::from_slice(bytes))
}

/// Every property name a stored snapshot holds -- instance and type tier together --
/// read from its dictionary line alone, so a 300 MB snapshot costs one line.
///
/// `None` for a legacy snapshot, which has no dictionary: its names are only known by
/// parsing it, and "cannot tell" must not be reported as "none".
pub fn property_names_of(bytes: &[u8]) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Keys {
        property_keys: Vec<String>,
    }
    let line = trailer_line(bytes)?;
    let mut object = Vec::with_capacity(line.len());
    object.push(b'{');
    object.extend_from_slice(line.get(1..)?);
    serde_json::from_slice::<Keys>(&object).ok().map(|k| k.property_keys)
}

/// The snapshot's last line, when it is a property-codec trailer.
pub(super) fn trailer_line(bytes: &[u8]) -> Option<&[u8]> {
    let body = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let start = body.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let line = &body[start..];
    line.starts_with(TRAILER_MARKER).then_some(line)
}
