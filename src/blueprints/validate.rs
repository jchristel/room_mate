//! What a blueprint's text and content may hold, checked before a revision is stored (`validate_content`): sizes, finite numbers,
//! follower loops and the item and door limits.

use super::*;

pub(super) fn clean_name(name: &str) -> Result<String, BlueprintError> {
    let name = name.trim();
    if name.is_empty() {
        return bad("a blueprint needs a name".to_string());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return bad(format!("a blueprint's name is at most {MAX_NAME_CHARS} characters"));
    }
    Ok(name.to_string())
}

pub(super) fn clean_note(note: Option<String>) -> Result<Option<String>, BlueprintError> {
    let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
        return bad(format!("a note is at most {MAX_NOTE_CHARS} characters"));
    }
    Ok(note)
}

pub(super) fn finite(label: &str, values: &[f64]) -> Result<(), BlueprintError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        bad(format!("{label} must be a finite number"))
    }
}

/// The one rule set for what a revision may hold. Pure, so it is testable without a disk.
pub fn validate_content(content: &RevisionContent) -> Result<(), BlueprintError> {
    let outline = &content.outline;
    if outline.coarse.len() < 3 {
        return bad("the coarse outline needs at least three corners".to_string());
    }
    if outline.coarse.len() > MAX_OUTLINE_POINTS || outline.fine.len() > MAX_OUTLINE_POINTS {
        return bad(format!("an outline is at most {MAX_OUTLINE_POINTS} points"));
    }
    for p in outline.coarse.iter().chain(&outline.fine) {
        finite("an outline point", &[p.x, p.y])?;
    }
    if content.doors.len() > MAX_DOORS {
        return bad(format!("a revision holds at most {MAX_DOORS} doors"));
    }
    for door in &content.doors {
        if door.edge >= outline.coarse.len() {
            return bad(format!("door {:?} is on edge {}, which the outline does not have", door.id, door.edge));
        }
        finite("a door's position", &[door.along_mm, door.edge_len_mm, door.width_mm])?;
    }
    if content.items.len() > MAX_ITEMS {
        return bad(format!("a revision holds at most {MAX_ITEMS} items"));
    }
    let mut seen = std::collections::HashSet::new();
    for item in &content.items {
        if item.id.trim().is_empty() {
            return bad("an item needs an id".to_string());
        }
        if !seen.insert(item.id.as_str()) {
            return bad(format!("item id {:?} appears twice", item.id));
        }
    }
    for item in &content.items {
        let p = &item.placement;
        if p.edge >= outline.coarse.len() {
            return bad(format!(
                "item {:?} is on edge {}, but the outline has {}",
                item.id,
                p.edge,
                outline.coarse.len()
            ));
        }
        finite(
            &format!("item {:?}'s placement", item.id),
            &[
                p.along_mm,
                p.off_mm,
                p.edge_len_mm,
                p.depth_mm,
                p.facing_deg,
                item.size_mm[0],
                item.size_mm[1],
            ],
        )?;
        if p.edge_len_mm <= 0.0 || p.depth_mm <= 0.0 {
            return bad(format!("item {:?}: the edge length and depth at capture must be positive", item.id));
        }
        if item.size_mm.iter().any(|s| *s < 0.0) {
            return bad(format!("item {:?}: a size cannot be negative", item.id));
        }
        if let Some(f) = &item.follows {
            if f.item == item.id {
                return bad(format!("item {:?} cannot follow itself", item.id));
            }
            if !seen.contains(f.item.as_str()) {
                return bad(format!("item {:?} follows {:?}, which is not in the revision", item.id, f.item));
            }
            finite(&format!("item {:?}'s follow gap", item.id), &[f.gap_mm])?;
        }
        if item.properties.len() > MAX_PROPERTIES || item.type_properties.len() > MAX_PROPERTIES {
            return bad(format!("item {:?} has more than {MAX_PROPERTIES} properties", item.id));
        }
    }
    // A chain of followers must end: an item cannot (through others) follow itself. The resolver
    // survives a loop and reports it, but a saved revision should not hold one.
    let follows: std::collections::HashMap<&str, &str> = content
        .items
        .iter()
        .filter_map(|i| i.follows.as_ref().map(|f| (i.id.as_str(), f.item.as_str())))
        .collect();
    for item in &content.items {
        let mut at = item.id.as_str();
        for _ in 0..=content.items.len() {
            match follows.get(at) {
                Some(&next) => at = next,
                None => break,
            }
            if at == item.id {
                return bad(format!("item {:?} follows itself through a loop of followers", item.id));
            }
        }
    }
    Ok(())
}
