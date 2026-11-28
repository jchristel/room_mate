//! What changed between two revisions of a blueprint, item by item. Derived on every read and stored nowhere: the
//! two revisions are the record.
//!
//! **Ids first, then place.** An item's id is the Revit element id in the model it was captured from, so a
//! re-capture of the same room keeps them and a rebuilt room, a copy in another model or a family replaced by
//! delete-and-place does not. Pass one pairs by id (`Matched::ById`); pass two pairs what is left by family, type
//! and distance in the blueprint's own frame (`Matched::ByPosition`) and says so, because a pairing by place is
//! a guess a person may want to check. Pass two pairs the globally nearest candidates first, not each item's nearest
//! in turn: a diff only labels a change, so it does not need the audit's optimal assignment, but a run of identical
//! chairs must not all collapse onto the first one.
//!
//! **It compares recipes, never a room.** Whether a change matters in a given room (an override over it, a fit that
//! no longer holds, an element already where the new recipe puts it) is `blueprint_impact`'s question.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use super::blueprint_geometry::Pt;
use super::blueprint_model::{BlueprintItem, RevisionContent};

/// One thing that differs on an item. An item may have several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// The recipe puts it somewhere else (`BlueprintItem::recipe_hash`: edge, distances and modes, facing).
    Moved,
    /// Another family or another type.
    TypeChanged,
    /// The parts its family brings with it differ. Said only when BOTH revisions recorded a kit: `None` is "not
    /// recorded", and a revision that learned to record it has not changed the item.
    KitChanged,
    /// Instance properties differ.
    PropertiesChanged,
    /// In the new revision and not the old, or no longer excluded.
    Added,
    /// In the old and not the new, or newly excluded. Never deleted from the earlier revision.
    Retired,
}

/// How an item was found in the other revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Matched {
    ById,
    /// No item with its id, so paired with the nearest of the same family and type. A guess, said so.
    ByPosition,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemDiff {
    /// The item's id in the NEW revision, or in the old one for a retired item.
    pub item_id: String,
    /// Its id in the old revision, when it was paired and the ids differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched: Option<Matched>,
    pub type_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    /// What differs. Empty is unchanged.
    pub changes: Vec<ChangeKind>,
    /// For a moved item: how far its centre moved in the blueprint's own frame, mm. Absent when either centre
    /// cannot be read (an edge index past the outline).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_mm: Option<f64>,
}

/// The two outlines, because every room's fit depends on them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutlineDiff {
    pub old_edges: usize,
    pub new_edges: usize,
    /// A vertex moved by more than a millimetre, or the edge count differs.
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RevisionDiff {
    pub outline: OutlineDiff,
    /// Every item that exists in either revision and is not excluded in both. Unchanged ones are listed too: a
    /// reader needs to know the diff looked at them.
    pub items: Vec<ItemDiff>,
    /// Items by change, plus `unchanged`.
    pub counts: BTreeMap<String, usize>,
    /// How many items were paired by place and not by id: nonzero says the ids moved between the revisions.
    pub matched_by_position: usize,
}

impl RevisionDiff {
    /// The items that changed in some way.
    pub fn changed(&self) -> impl Iterator<Item = &ItemDiff> {
        self.items.iter().filter(|d| !d.changes.is_empty())
    }
}

fn coarse_pts(content: &RevisionContent) -> Vec<Pt> {
    content.outline.coarse.iter().map(|p| (p.x, p.y)).collect()
}

fn centre_of(item: &BlueprintItem, coarse: &[Pt]) -> Option<Pt> {
    item.placement.centre_in(coarse)
}

fn outline_diff(old: &RevisionContent, new: &RevisionContent) -> OutlineDiff {
    let (a, b) = (&old.outline.coarse, &new.outline.coarse);
    let changed = a.len() != b.len() || a.iter().zip(b).any(|(p, q)| (p.x - q.x).hypot(p.y - q.y) > 1.0);
    OutlineDiff { old_edges: a.len(), new_edges: b.len(), changed }
}

/// What differs between an item and its pair.
fn changes_between(old: &BlueprintItem, new: &BlueprintItem) -> Vec<ChangeKind> {
    let mut out = Vec::new();
    if old.recipe_hash() != new.recipe_hash() {
        out.push(ChangeKind::Moved);
    }
    if old.family_name != new.family_name || old.type_name != new.type_name {
        out.push(ChangeKind::TypeChanged);
    }
    if let (Some(a), Some(b)) = (&old.kit, &new.kit)
        && a != b
    {
        out.push(ChangeKind::KitChanged);
    }
    if old.properties != new.properties {
        out.push(ChangeKind::PropertiesChanged);
    }
    out
}

fn entry(item: &BlueprintItem, changes: Vec<ChangeKind>) -> ItemDiff {
    ItemDiff {
        item_id: item.id.clone(),
        old_item_id: None,
        matched: None,
        type_name: item.type_name.clone(),
        family_name: item.family_name.clone(),
        changes,
        moved_mm: None,
    }
}

/// Pass two: pair the unmatched old items with the unmatched new ones of the same family and type, nearest first,
/// within `near_mm`. Returns `(old index, new index, distance)`.
fn pair_by_position(
    old: &[&BlueprintItem],
    new: &[&BlueprintItem],
    old_coarse: &[Pt],
    new_coarse: &[Pt],
    near_mm: f64,
) -> Vec<(usize, usize, f64)> {
    let mut candidates = Vec::new();
    for (i, o) in old.iter().enumerate() {
        let Some(oc) = centre_of(o, old_coarse) else { continue };
        for (j, n) in new.iter().enumerate() {
            if o.family_name != n.family_name || o.type_name != n.type_name {
                continue;
            }
            let Some(nc) = centre_of(n, new_coarse) else { continue };
            let d = (oc.0 - nc.0).hypot(oc.1 - nc.1);
            if d <= near_mm {
                candidates.push((i, j, d));
            }
        }
    }
    candidates.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    let (mut used_old, mut used_new) = (HashSet::new(), HashSet::new());
    let mut out = Vec::new();
    for (i, j, d) in candidates {
        if used_old.contains(&i) || used_new.contains(&j) {
            continue;
        }
        used_old.insert(i);
        used_new.insert(j);
        out.push((i, j, d));
    }
    out
}

/// The difference between two revisions' content. `near_mm` is how far apart two items of one family and type may
/// be in the blueprint's frame and still be called the same item when their ids differ.
pub fn diff(old: &RevisionContent, new: &RevisionContent, near_mm: f64) -> RevisionDiff {
    let (old_coarse, new_coarse) = (coarse_pts(old), coarse_pts(new));
    let mut items: Vec<ItemDiff> = Vec::new();
    let mut matched_by_position = 0;

    // Excluded in both is not in the blueprint in either; excluded in one is added or retired.
    let old_live: Vec<&BlueprintItem> = old.items.iter().filter(|i| !i.excluded).collect();
    let new_live: Vec<&BlueprintItem> = new.items.iter().filter(|i| !i.excluded).collect();
    let old_by_id: BTreeMap<&str, &BlueprintItem> = old.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let new_by_id: BTreeMap<&str, &BlueprintItem> = new.items.iter().map(|i| (i.id.as_str(), i)).collect();

    let mut unmatched_old: Vec<&BlueprintItem> = Vec::new();
    let mut paired_new: HashSet<&str> = HashSet::new();
    for o in &old_live {
        match new_by_id.get(o.id.as_str()) {
            Some(n) if !n.excluded => {
                paired_new.insert(n.id.as_str());
                let mut d = entry(n, changes_between(o, n));
                d.matched = Some(Matched::ById);
                d.moved_mm = moved_mm(o, n, &old_coarse, &new_coarse, &d.changes);
                items.push(d);
            }
            // Still in the new revision but excluded there: retired.
            Some(_) => items.push(entry(o, vec![ChangeKind::Retired])),
            None => unmatched_old.push(o),
        }
    }
    // A new live item that was excluded in the old revision is added back; one the old never had is a candidate for pass two.
    let mut unmatched_new: Vec<&BlueprintItem> = Vec::new();
    for n in &new_live {
        if paired_new.contains(n.id.as_str()) {
            continue;
        }
        match old_by_id.get(n.id.as_str()) {
            Some(_) => items.push(entry(n, vec![ChangeKind::Added])),
            None => unmatched_new.push(n),
        }
    }

    let pairs = pair_by_position(&unmatched_old, &unmatched_new, &old_coarse, &new_coarse, near_mm);
    let (mut took_old, mut took_new) = (HashSet::new(), HashSet::new());
    for (i, j, _) in pairs {
        let (o, n) = (unmatched_old[i], unmatched_new[j]);
        took_old.insert(i);
        took_new.insert(j);
        matched_by_position += 1;
        let mut d = entry(n, changes_between(o, n));
        d.old_item_id = Some(o.id.clone());
        d.matched = Some(Matched::ByPosition);
        d.moved_mm = moved_mm(o, n, &old_coarse, &new_coarse, &d.changes);
        items.push(d);
    }
    for (i, o) in unmatched_old.iter().enumerate() {
        if !took_old.contains(&i) {
            items.push(entry(o, vec![ChangeKind::Retired]));
        }
    }
    for (j, n) in unmatched_new.iter().enumerate() {
        if !took_new.contains(&j) {
            items.push(entry(n, vec![ChangeKind::Added]));
        }
    }

    items.sort_by(|a, b| a.item_id.cmp(&b.item_id));
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for d in &items {
        if d.changes.is_empty() {
            *counts.entry("unchanged".to_string()).or_default() += 1;
        }
        for c in &d.changes {
            let name = serde_json::to_value(c)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            *counts.entry(name).or_default() += 1;
        }
    }
    RevisionDiff { outline: outline_diff(old, new), items, counts, matched_by_position }
}

fn moved_mm(
    old: &BlueprintItem,
    new: &BlueprintItem,
    old_coarse: &[Pt],
    new_coarse: &[Pt],
    changes: &[ChangeKind],
) -> Option<f64> {
    if !changes.contains(&ChangeKind::Moved) {
        return None;
    }
    let (a, b) = (centre_of(old, old_coarse)?, centre_of(new, new_coarse)?);
    Some((a.0 - b.0).hypot(a.1 - b.1))
}

#[cfg(test)]
mod tests;
