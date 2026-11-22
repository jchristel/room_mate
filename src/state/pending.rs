//! A push whose phase disagrees with its lineage's, quarantined and promotable (`AppState`'s pending half).

use super::*;

impl AppState {
    /// Quarantine a push whose phase disagrees with the lineage's — see
    /// `SnapshotStore::put_pending_raw`. Stored but not live, awaiting an
    /// explicit promotion.
    ///
    /// **Generic over the payload, exactly as `open_opening_snapshot` is**, and
    /// for the same reason: this is the serde-at-the-boundary layer, so it
    /// serialises whatever it is handed and the store still takes bytes plus a
    /// kind. A `set_pending_spaces_snapshot` beside a rooms one would be the
    /// parallel method set R1 exists to prevent.
    pub fn set_pending_snapshot<P: crate::contract::SnapshotEnvelope + serde::Serialize>(
        &self,
        key: &ModelKey,
        kind: SnapshotKind,
        payload: &P,
    ) -> anyhow::Result<()> {
        let json = encode_snapshot(payload).context("could not serialise pending snapshot")?;
        self.store.put_pending_raw(key, kind, payload.taken_at(), payload.phase(), &json)
    }

    /// The quarantined rooms push waiting on one model, if any — see
    /// `SnapshotStore::get_pending_raw`.
    ///
    /// Rooms-typed on the way *out* while the way in is generic, and the
    /// asymmetry is real rather than an oversight: writing needs only bytes, and
    /// reading needs to know what to parse them into. `pending_spaces_snapshot`
    /// is its sibling; a caller always knows which kind it is asking about,
    /// because the route says so.
    pub fn pending_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<RoomPayload>> {
        self.store
            .get_pending_raw(key, SnapshotKind::Rooms)?
            .map(|bytes| parse_rooms(key, &bytes))
            .transpose()
    }

    /// The quarantined **spaces** push waiting on one model, if any.
    pub fn pending_spaces_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<SpacePayload>> {
        self.store
            .get_pending_raw(key, SnapshotKind::Spaces)?
            .map(|bytes| parse_spaces(key, &bytes))
            .transpose()
    }

    /// Make a quarantined push live, re-phasing the lineage — see
    /// `SnapshotStore::promote_pending`. The one deliberate way a model's phase
    /// changes after it is first set.
    ///
    /// The parse happens *here* rather than in the store because promoting is
    /// defined in terms of the pending payload's own phase, and the store sees
    /// only bytes. Reading it first also means a corrupt quarantined file fails
    /// before anything is written, leaving it intact and retryable.
    pub fn promote_pending_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<RoomPayload>> {
        let Some(payload) = self.pending_snapshot(key)? else {
            return Ok(None);
        };
        if self.store.promote_pending(&rooms_meta(key, &payload))? {
            Ok(Some(payload))
        } else {
            // The quarantine was cleared between the read above and the
            // promotion — report it as "nothing was pending", the same answer
            // the caller would have got had it arrived a moment later.
            Ok(None)
        }
    }

    /// `promote_pending_snapshot`'s spaces counterpart, on identical terms.
    ///
    /// **A spaces lineage is re-phased by promoting a spaces push**, which is
    /// the whole point of D11: a services model usually holds no rooms, so
    /// "re-phase it with a rooms push first" is not an escape hatch it has.
    pub fn promote_pending_spaces_snapshot(&self, key: &ModelKey) -> anyhow::Result<Option<SpacePayload>> {
        let Some(payload) = self.pending_spaces_snapshot(key)? else {
            return Ok(None);
        };
        let meta = SnapshotMeta {
            kind: SnapshotKind::Spaces,
            key,
            project_name: &payload.project.name,
            model_name: &payload.model.name,
            taken_at: &payload.snapshot.taken_at,
            phase: payload.phase.as_deref(),
            model_to_shared: payload.model_to_shared,
        };
        if self.store.promote_pending(&meta)? {
            Ok(Some(payload))
        } else {
            Ok(None)
        }
    }
}
