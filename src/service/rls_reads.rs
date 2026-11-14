//! What a create-RLS request reads of the project, read ONCE for every room it names (2026-11-13).
//!
//! Each room of a request used to read the project's rooms about six times (qualifying it, its name and number, its
//! summary, both outlines, and both again for the room tag) and the whole project's FF&E twice (its item count and
//! the items to tag or see through), and a request may name [`tasks_api`]'s 200 rooms. On a large real project one
//! FF&E read takes seconds in a release build, so a batch from the RLS tool's auto mode ran to minutes. Every one of
//! those reads asked the same question of the same store within one request, so they are answered once here and
//! the per-room work runs over the answers.
//!
//! **The FF&E is read with its nested components** (`ItemScope::with_components`), because the tags need them
//! (`rls_tags::plan_tags` keeps a nested part a rule exists for). The room summary's item count is the PROJECT
//! policy's count, so [`RlsReads::item_count`] leaves a component out under `[ffe] nested_components = "exclude"`,
//! exactly as the policy's own read would. Both reads are lazy: a request that needs no FF&E (no tags, no 3D view, no
//! stored summary to compare) never reads it.
//!
//! **Not a cache.** It lives for one request and is dropped with it, so nothing here can serve a push that landed
//! after the request began, beyond what the request already saw.
//!
//! [`tasks_api`]: crate::tasks_api

use std::cell::OnceCell;

use super::blueprint_capture::qualify_room_in;
use super::items::{assemble_items, FfeResult, ItemScope};
use super::room_locator::RoomRef;
use super::rooms::{assemble_rooms, RoomScope, RoomsResult};
use super::ServiceError;
use crate::settings::NestedComponents;
use crate::state::AppState;

pub struct RlsReads<'a> {
    pub(super) state: &'a AppState,
    pub(super) project: &'a str,
    rooms: OnceCell<Option<RoomsResult>>,
    ffe: OnceCell<Option<FfeResult>>,
}

impl<'a> RlsReads<'a> {
    pub fn new(state: &'a AppState, project: &'a str) -> Self {
        RlsReads { state, project, rooms: OnceCell::new(), ffe: OnceCell::new() }
    }

    /// The project's rooms, as `/rooms` answers them; `None` when nothing has been pushed.
    pub fn rooms(&self) -> Result<Option<&RoomsResult>, ServiceError> {
        if self.rooms.get().is_none() {
            let read = assemble_rooms(self.state, &RoomScope { project: Some(self.project), ..Default::default() })?;
            let _ = self.rooms.set(read);
        }
        Ok(self.rooms.get().and_then(Option::as_ref))
    }

    /// The project's FF&E with its nested components; `None` when no FF&E has been pushed.
    pub fn ffe(&self) -> Result<Option<&FfeResult>, ServiceError> {
        if self.ffe.get().is_none() {
            let scope = ItemScope { project: Some(self.project), with_components: true, ..Default::default() };
            let _ = self.ffe.set(assemble_items(self.state, &scope)?);
        }
        Ok(self.ffe.get().and_then(Option::as_ref))
    }

    /// `blueprint_capture::qualify_room` with no model named, over the rooms read once.
    pub fn qualify(&self, room_id: &str) -> Result<Option<RoomRef>, ServiceError> {
        match self.rooms()? {
            Some(rooms) => qualify_room_in(rooms, room_id),
            None => Ok(None),
        }
    }

    /// How many items the project's own FF&E read attributes to `room`: a nested component counts only when the
    /// project includes them (`sheet_suggest::room_summary`'s count, from the read with components).
    pub fn item_count(&self, room: &RoomRef) -> Result<usize, ServiceError> {
        let include = self
            .state
            .settings()
            .settings_for(self.project)
            .is_some_and(|b| b.ffe.nested_components != NestedComponents::Exclude);
        Ok(self.ffe()?.map_or(0, |ffe| {
            ffe.ffe
                .iter()
                .filter(|it| (include || !it.is_component) && it.owner_rooms_qualified.contains(room))
                .count()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemStore;
    use std::collections::HashMap;

    #[test]
    fn a_store_with_nothing_pushed_reads_as_nothing_and_once() {
        let state = AppState::new(Box::new(MemStore::new()), HashMap::new(), None);
        let reads = RlsReads::new(&state, "p");
        assert!(reads.rooms().unwrap().is_none());
        assert!(reads.rooms.get().is_some(), "the answer is kept for the next room");
        assert!(reads.ffe().unwrap().is_none());
        assert_eq!(None, reads.qualify("1").unwrap());
        assert_eq!(0, reads.item_count(&RoomRef { model_id: "m".into(), room_id: "1".into() }).unwrap());
    }
}
