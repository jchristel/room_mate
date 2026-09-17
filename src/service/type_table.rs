//! The response-side type table: each distinct type-property bag sent once per
//! read, and every element pointing at its row.
//!
//! **Why the wire needed its own.** Storage and memory already hold one bag per
//! distinct type (`contract::property_codec`), but a response that serialised
//! each element whole wrote that shared bag out again for every instance. On
//! RHH that made `/ffe` a 280 MB body for 38,913 items, most of it the same
//! 1,005 bags repeated -- and the viewer polls that body before it can draw.
//!
//! **Moved, not copied, and only at the wire.** `Assembled` and the service
//! results keep every bag on its element, because the property filter and the
//! milestone comparison read the type tier through `PropertyTiers`. The table is
//! built as the very last step, when a wire result is made from them: each bag
//! is moved out of its element into the table (leaving the element's field
//! empty, which its serde attribute then omits) and the element is given
//! `type_properties_ref`, its row.
//!
//! **Rows are keyed by content, never by `type_id`**, for the storage codec's
//! reason: nothing enforces one bag per type id, and a table keyed by it would
//! serve one instance another's bag. Pointer identity is checked first, since
//! bags read from one encoded snapshot are already shared allocations; content
//! is the fallback, which is what a legacy snapshot's unshared bags need.
//!
//! A row is only meaningful within the response that carries it.

use std::collections::HashMap;
use std::sync::Arc;

use crate::contract::PropertyMap;

/// Move every element's type bag into one table and give each element its row.
///
/// `bag` names, for one element, its type-property field and its row field --
/// the one thing that differs between an opening, an item and a surface.
pub fn tabulate<E>(
    elements: &mut [E],
    bag: impl Fn(&mut E) -> (&mut Arc<PropertyMap>, &mut Option<u32>),
) -> Vec<Arc<PropertyMap>> {
    let mut sets: Vec<Arc<PropertyMap>> = Vec::new();
    let mut by_pointer: HashMap<*const PropertyMap, u32> = HashMap::new();
    let mut by_content: HashMap<Arc<PropertyMap>, u32> = HashMap::new();

    for element in elements {
        let (field, row) = bag(element);
        let taken = std::mem::take(field);
        let index = match by_pointer.get(&Arc::as_ptr(&taken)) {
            Some(&index) => index,
            None => {
                let index = match by_content.get(&taken) {
                    Some(&index) => index,
                    None => {
                        let index = sets.len() as u32;
                        sets.push(taken.clone());
                        by_content.insert(taken.clone(), index);
                        index
                    }
                };
                by_pointer.insert(Arc::as_ptr(&taken), index);
                index
            }
        };
        *row = Some(index);
    }
    sets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::CustomValue;

    struct Element {
        type_properties: Arc<PropertyMap>,
        type_properties_ref: Option<u32>,
    }

    fn bag(value: &str) -> Arc<PropertyMap> {
        Arc::new(
            [("Width".into(), CustomValue { value: value.into(), storage_type: None })]
                .into_iter()
                .collect(),
        )
    }

    /// One row per distinct bag, whether the elements shared an allocation or
    /// only equal content, and every element left empty and pointing at its row.
    #[test]
    fn test_each_distinct_bag_becomes_one_row() {
        let shared = bag("820");
        let mut elements: Vec<Element> = [shared.clone(), shared, bag("820"), bag("920")]
            .into_iter()
            .map(|type_properties| Element { type_properties, type_properties_ref: None })
            .collect();

        let sets = tabulate(&mut elements, |e| (&mut e.type_properties, &mut e.type_properties_ref));

        assert_eq!(sets.len(), 2, "820 once, whether shared or merely equal; 920 once");
        let rows: Vec<_> = elements.iter().map(|e| e.type_properties_ref).collect();
        assert_eq!(rows, [Some(0), Some(0), Some(0), Some(1)]);
        assert_eq!(sets[0]["Width"].value, "820");
        assert_eq!(sets[1]["Width"].value, "920");
        assert!(elements.iter().all(|e| e.type_properties.is_empty()), "moved, not copied");
    }
}
