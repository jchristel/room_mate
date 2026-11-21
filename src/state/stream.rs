//! A snapshot written element by element (`StreamingSnapshot`) and the elements a scoped push carries over from the latest snapshot
//! (`Carried`). The JSON framing lives here, not in the store.

use super::*;

/// The elements of a model's latest snapshot that a scoped push leaves alone, as wire lines.
///
/// A level-scoped push (`scoped_push`) replaces only some levels of a model: what it carries over
/// are the OTHER levels' elements of the latest snapshot. They are decoded (the stored property
/// triples expanded back to plain maps) and written as the lines a producer would have sent, so
/// they pass through the ordinary ingest -- the same checks, the same encoding -- and the stored
/// result is a complete snapshot like any other. Lines are made one at a time; the decoded
/// payload is the only thing held.
pub struct Carried {
    /// Elements in the latest snapshot.
    pub total: usize,
    /// Elements carried over: those not on the replaced levels.
    pub kept: usize,
    pub(super) lines: Box<dyn Iterator<Item = anyhow::Result<Vec<u8>>> + Send>,
}

impl Iterator for Carried {
    type Item = anyhow::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.lines.next()
    }
}

/// One wire line: the element's own object with the model id added.
pub(super) fn wire_line<T: serde::Serialize>(model_id: &str, element: &T) -> anyhow::Result<Vec<u8>> {
    let mut value = serde_json::to_value(element)?;
    let serde_json::Value::Object(map) = &mut value else {
        anyhow::bail!("an element must serialise to a JSON object");
    };
    map.insert("model_id".to_string(), serde_json::Value::String(model_id.to_string()));
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

macro_rules! carry {
    ($bytes:expr, $payload:ty, $field:ident, $model:expr, $levels:expr, $key:expr) => {{
        let payload: $payload = decode_snapshot($bytes)
            .with_context(|| format!("malformed snapshot for {}/{}", $key.project_id, $key.model_id))?;
        let total = payload.$field.len();
        let model = $model.to_string();
        let levels: Vec<String> = $levels.to_vec();
        let kept: Vec<_> = payload.$field.into_iter().filter(|e| !levels.contains(&e.level_id)).collect();
        Carried {
            total,
            kept: kept.len(),
            lines: Box::new(kept.into_iter().map(move |e| wire_line(&model, &e))),
        }
    }};
}
pub(super) use carry;

/// One snapshot being written element by element — the JSON half of
/// `SnapshotStore::put_streaming`.
///
/// **The format lives here, not in the store.** The store's boundary is bytes
/// (`CLAUDE.md`: "The store takes bytes plus a `SnapshotMeta`, never a payload
/// type"), and a store that parsed the payload in order to write it would be the
/// first crack in that rule. This is the layer that already owns serde, so it is
/// the one that knows a snapshot is a JSON object with an array of elements in
/// it.
///
/// The file it produces is **one element per line, then the property
/// dictionary as the last line**:
///
/// ```text
/// {"schema_version":7,...,"rooms":[
/// {"id":"r1",...,"properties":[[0,1,"Kitchen"],[1,0,"12.5"]]},
/// {"id":"r2",...,"properties":[[0,1,"Store"]]}
/// ]
/// ,"property_codec":1,"property_keys":["Name","Area"],"storage_types":[null,"String"],"type_property_sets":[]}
/// ```
///
/// Every property map in an element is written as index triples against that
/// dictionary, and a type-property bag as an index into its table of distinct
/// bags — see `contract::property_codec` for why. **The dictionary comes last
/// because it cannot come first**: a push is streamed, and the keys are only all
/// known once every element has gone by.
///
/// The buffered path (`encode_snapshot`) writes the same layout on fewer lines;
/// the two differ only in whitespace and key order, and nothing re-reads a
/// snapshot by shape beyond finding that last line. One-per-line is what makes
/// the file writable incrementally at all, and still diffable line by line —
/// though a line now names its properties by index, so reading one by eye takes
/// the dictionary beside it.
///
/// **Dropping without committing discards the whole snapshot**, and that is the
/// ordinary path rather than an error one: a rooms push is only known to be
/// empty once its rooms have been counted, which is after writing has started.
pub struct StreamingSnapshot<'a> {
    writer: Box<dyn SnapshotWriter + 'a>,
    /// Elements written so far — also what tells `push` whether it owes a comma.
    count: usize,
    /// The property dictionary every element so far was encoded against,
    /// written out at `commit`.
    encoder: Box<Encoder>,
}

impl<'a> StreamingSnapshot<'a> {
    /// Open the store's writer and emit everything up to the opening bracket of
    /// the element array.
    ///
    /// `envelope` is the payload type itself, serialized and then stripped of
    /// its element array — the array key being `kind.label()`, which is already
    /// exactly the JSON field name for both entities. Reusing the payload type
    /// rather than mirroring it is what keeps a new envelope field from silently
    /// going missing on the streamed path only.
    pub(super) fn open<T: serde::Serialize>(
        store: &'a dyn SnapshotStore,
        meta: &SnapshotMeta<'_>,
        kind: SnapshotKind,
        envelope: &T,
    ) -> anyhow::Result<Self> {
        let mut fields = match serde_json::to_value(envelope).context("could not serialise snapshot envelope")? {
            serde_json::Value::Object(map) => map,
            other => anyhow::bail!("snapshot envelope must serialise to a JSON object, got {other}"),
        };
        // Drop the element array the caller passed empty; it is written below,
        // one line at a time, from the elements that actually arrive.
        fields.remove(kind.label());

        let mut writer = store.put_streaming(meta)?;
        let mut header =
            serde_json::to_vec(&serde_json::Value::Object(fields)).context("could not serialise snapshot envelope")?;
        // `header` is a complete JSON object; splice the element array in before
        // its closing brace. An envelope with no fields at all is not reachable
        // (identity is validated at ingest), but an empty object would produce
        // `{,"rooms":[` — so the comma is conditional rather than assumed.
        let empty = header == b"{}";
        header.truncate(header.len() - 1);
        writer.write(&header)?;
        if !empty {
            writer.write(b",")?;
        }
        writer.write(format!("\"{}\":[\n", kind.label()).as_bytes())?;
        Ok(Self { writer, count: 0, encoder: Box::default() })
    }

    /// Append one element. The separator is written *before* the element rather
    /// than after, because only this side knows whether a previous one exists —
    /// a trailing comma is the one thing that would make the file unparseable.
    pub fn push<T: serde::Serialize>(&mut self, element: &T) -> anyhow::Result<()> {
        if self.count > 0 {
            self.writer.write(b",\n")?;
        }
        let bytes = self
            .encoder
            .encode(|| serde_json::to_vec(element))
            .context("could not serialise snapshot element")?;
        self.writer.write(&bytes)?;
        self.count += 1;
        Ok(())
    }

    /// How many elements have been written — the count an ingest route reports,
    /// and the one it checks before deciding whether the push is empty.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Close the array, write the dictionary as the last line, and publish the
    /// snapshot.
    pub fn commit(mut self) -> anyhow::Result<()> {
        self.writer.write(b"\n]\n")?;
        self.writer
            .write(&self.encoder.trailer().context("could not serialise property dictionary")?)?;
        self.writer.write(b"}\n")?;
        self.writer.commit()
    }
}
