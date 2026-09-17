//! How property maps are held in memory and written to storage: each distinct
//! key once, each distinct type-property bag once.
//!
//! **Why this exists.** Every element carries its properties as a map keyed by
//! the source's own parameter names, and every door, window, item, ceiling and
//! floor also carries its family *type's* whole map. On RHH's FF&E that was 61%
//! of every snapshot byte for `type_properties` alone -- 16,397 items sharing
//! 1,005 distinct bags -- and the instance keys repeat on every element too.
//! Parsing is what a read costs, and the parse was dominated by allocating the
//! same few thousand strings millions of times, then again when `/ffe` cloned
//! each item into its response. Measured on the stored RHH FF&E, parse plus
//! clone: 29 s as maps of owned strings, 4.3 s with the storage shape below and
//! these types in memory.
//!
//! **The in-memory half** is `PropertyKey = Arc<str>` and a type bag behind
//! `Arc<PropertyMap>`. A key or bag is allocated once per snapshot and every
//! element that uses it holds a pointer -- which is also what makes cloning an
//! element cheap. The on-disk dictionary alone is not enough: expanded back into
//! owned strings, the same files parsed in 9.1 s.
//!
//! **The storage half** replaces, inside a stored snapshot only, each element's
//! `properties` with `[key, storage_type, value]` index triples and its
//! `type_properties` with an index into a table of distinct bags. The dictionary
//! those indices name is written once, as the snapshot's last line (see
//! `state::StreamingSnapshot`) -- last, because ingest streams and only knows
//! every key once every element has gone by.
//!
//! **Neither wire changes.** A push still sends maps and a response still
//! returns them: the same field codecs below write a plain map unless an
//! [`Encoder`] is active on the thread, and read a plain map as readily as
//! triples. So the extractor, the viewer and every snapshot already on disk are
//! untouched -- a stored snapshot written before this reads through the map
//! path, and gets the shared keys anyway because a [`Decoder`] interns them.
//!
//! **The context is thread-local, and that is deliberate rather than a
//! shortcut.** The alternative is a second set of storage-only element structs,
//! one per entity, each mirroring its contract type field for field -- the
//! parallel-type failure the store boundary exists to prevent, and one that
//! would silently drop a field the day someone added it to only one side.
//! Serialization and parsing are synchronous, so the context lives exactly as
//! long as the one call it wraps.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use super::{CustomValue, PropertyKey, PropertyMap};

/// The dictionary layout version, the first field of every trailer.
pub const PROPERTY_CODEC: u32 = 1;

/// What the last line of an encoded snapshot starts with.
///
/// Recognised only on the last line, so a legacy snapshot that happened to
/// carry a property of this name somewhere inside it is never mistaken for one.
pub const TRAILER_MARKER: &[u8] = b",\"property_codec\":";

thread_local! {
    static ENCODER: RefCell<Option<Encoder>> = const { RefCell::new(None) };
    static DECODER: RefCell<Option<Decoder>> = const { RefCell::new(None) };
}

/// The dictionary a snapshot's elements are encoded against, collected while
/// they are written.
#[derive(Default)]
pub struct Encoder {
    keys: Vec<PropertyKey>,
    key_index: HashMap<PropertyKey, u32>,
    storage_types: Vec<Option<Arc<str>>>,
    storage_type_index: HashMap<Option<Arc<str>>, u32>,
    type_sets: Vec<Vec<(u32, u32, String)>>,
    /// Keyed by content, never by `type_id`. Every measured snapshot agrees that
    /// one type id means one bag, but nothing enforces it, and a table keyed by
    /// the id would silently keep the first bag and serve it to an instance
    /// that exported another. Keyed by content, a disagreement costs one more
    /// row and loses nothing.
    type_set_index: HashMap<Vec<(u32, u32, String)>, u32>,
}

impl Encoder {
    /// Run `serialize` with this encoder active, so every property map it
    /// writes is encoded against, and added to, this dictionary.
    pub fn encode<R>(&mut self, serialize: impl FnOnce() -> R) -> R {
        /// Puts the encoder back even if `serialize` unwinds, so a panic cannot
        /// leave the next serialization on this thread writing triples.
        struct Restore<'a>(&'a mut Encoder);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                if let Some(encoder) = ENCODER.with(|slot| slot.borrow_mut().take()) {
                    *self.0 = encoder;
                }
            }
        }
        ENCODER.with(|slot| {
            let mut slot = slot.borrow_mut();
            debug_assert!(slot.is_none(), "property encoders do not nest");
            *slot = Some(std::mem::take(self));
        });
        let _restore = Restore(self);
        serialize()
    }

    /// The trailer: `,"property_codec":1,...` with no closing brace, ready to
    /// be spliced in as the last fields of the snapshot's top-level object.
    pub fn trailer(&self) -> serde_json::Result<Vec<u8>> {
        #[derive(Serialize)]
        struct Trailer<'a> {
            property_codec: u32,
            property_keys: &'a [PropertyKey],
            storage_types: &'a [Option<Arc<str>>],
            type_property_sets: &'a [Vec<(u32, u32, String)>],
        }
        let mut bytes = serde_json::to_vec(&Trailer {
            property_codec: PROPERTY_CODEC,
            property_keys: &self.keys,
            storage_types: &self.storage_types,
            type_property_sets: &self.type_sets,
        })?;
        // `{...}` -> `,...`: the caller owns the top-level object's braces.
        bytes[0] = b',';
        bytes.pop();
        Ok(bytes)
    }

    fn key(&mut self, key: &PropertyKey) -> u32 {
        if let Some(&index) = self.key_index.get(key) {
            return index;
        }
        let index = self.keys.len() as u32;
        self.keys.push(key.clone());
        self.key_index.insert(key.clone(), index);
        index
    }

    fn storage_type(&mut self, storage_type: &Option<Arc<str>>) -> u32 {
        if let Some(&index) = self.storage_type_index.get(storage_type) {
            return index;
        }
        let index = self.storage_types.len() as u32;
        self.storage_types.push(storage_type.clone());
        self.storage_type_index.insert(storage_type.clone(), index);
        index
    }

    fn triples<'m>(&mut self, map: &'m PropertyMap) -> Vec<(u32, u32, &'m str)> {
        map.iter()
            .map(|(key, value)| (self.key(key), self.storage_type(&value.storage_type), value.value.as_str()))
            .collect()
    }

    fn type_set(&mut self, map: &PropertyMap) -> u32 {
        let triples: Vec<(u32, u32, String)> =
            self.triples(map).into_iter().map(|(k, s, v)| (k, s, v.to_string())).collect();
        if let Some(&index) = self.type_set_index.get(&triples) {
            return index;
        }
        let index = self.type_sets.len() as u32;
        self.type_sets.push(triples.clone());
        self.type_set_index.insert(triples, index);
        index
    }
}

/// The dictionary a stored snapshot's elements are decoded against.
pub struct Decoder {
    keys: Vec<PropertyKey>,
    storage_types: Vec<Option<Arc<str>>>,
    type_sets: Vec<Arc<PropertyMap>>,
    /// Keys and storage types met in plain maps, so a snapshot written before
    /// the codec still shares one allocation per distinct string.
    interned: HashSet<Arc<str>>,
}

impl Decoder {
    /// A decoder with an empty dictionary, for a snapshot that has no trailer.
    pub fn legacy() -> Self {
        Self {
            keys: Vec::new(),
            storage_types: Vec::new(),
            type_sets: Vec::new(),
            interned: HashSet::new(),
        }
    }

    /// Build the dictionary from a snapshot's trailer line
    /// (`,"property_codec":...}`).
    pub fn from_trailer(line: &[u8]) -> serde_json::Result<Self> {
        #[derive(Deserialize)]
        struct Trailer {
            property_codec: u32,
            property_keys: Vec<String>,
            storage_types: Vec<Option<String>>,
            type_property_sets: Vec<Vec<(u32, u32, String)>>,
        }
        let mut object = Vec::with_capacity(line.len());
        object.push(b'{');
        object.extend_from_slice(line.get(1..).unwrap_or_default());
        let trailer: Trailer = serde_json::from_slice(&object)?;
        if trailer.property_codec != PROPERTY_CODEC {
            return Err(de::Error::custom(format!(
                "snapshot property codec {} is not the supported {PROPERTY_CODEC}",
                trailer.property_codec
            )));
        }
        let mut decoder = Self {
            keys: trailer.property_keys.into_iter().map(Arc::from).collect(),
            storage_types: trailer.storage_types.into_iter().map(|s| s.map(Arc::from)).collect(),
            type_sets: Vec::new(),
            interned: HashSet::new(),
        };
        decoder.type_sets = trailer
            .type_property_sets
            .into_iter()
            .map(|set| {
                let mut map = PropertyMap::new();
                for triple in set {
                    let (key, value) = decoder.resolve(triple).map_err(de::Error::custom)?;
                    map.insert(key, value);
                }
                Ok(Arc::new(map))
            })
            .collect::<serde_json::Result<_>>()?;
        Ok(decoder)
    }

    /// Run `deserialize` with this decoder active.
    pub fn decode<R>(self, deserialize: impl FnOnce() -> R) -> R {
        /// Clears the slot even if `deserialize` unwinds.
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                DECODER.with(|slot| slot.borrow_mut().take());
            }
        }
        DECODER.with(|slot| {
            let mut slot = slot.borrow_mut();
            debug_assert!(slot.is_none(), "property decoders do not nest");
            *slot = Some(self);
        });
        let _clear = Clear;
        deserialize()
    }

    fn resolve(&self, (key, storage_type, value): (u32, u32, String)) -> Result<(PropertyKey, CustomValue), String> {
        let key = self
            .keys
            .get(key as usize)
            .ok_or_else(|| format!("property key index {key} is outside the snapshot's dictionary"))?;
        let storage_type = self
            .storage_types
            .get(storage_type as usize)
            .ok_or_else(|| format!("storage type index {storage_type} is outside the snapshot's dictionary"))?;
        Ok((key.clone(), CustomValue { value, storage_type: storage_type.clone() }))
    }

    fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some(shared) = self.interned.get(s) {
            return shared.clone();
        }
        let shared: Arc<str> = Arc::from(s);
        self.interned.insert(shared.clone());
        shared
    }
}

/// A string shared through the active decoder, or allocated when none is.
struct Interned(Arc<str>);

impl<'de> Deserialize<'de> for Interned {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct InternVisitor;
        impl Visitor<'_> for InternVisitor {
            type Value = Interned;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Interned, E> {
                Ok(Interned(DECODER.with(|slot| match slot.borrow_mut().as_mut() {
                    Some(decoder) => decoder.intern(s),
                    None => Arc::from(s),
                })))
            }
        }
        deserializer.deserialize_str(InternVisitor)
    }
}

/// One property value as a plain map spells it.
#[derive(Deserialize)]
struct MapValue {
    value: String,
    #[serde(default)]
    storage_type: Option<Interned>,
}

struct MapVisitor;

impl<'de> Visitor<'de> for MapVisitor {
    type Value = PropertyMap;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a property map, or property triples inside a stored snapshot")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<PropertyMap, A::Error> {
        let mut map = PropertyMap::new();
        while let Some(Interned(key)) = access.next_key()? {
            let MapValue { value, storage_type } = access.next_value()?;
            map.insert(key, CustomValue { value, storage_type: storage_type.map(|s| s.0) });
        }
        Ok(map)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<PropertyMap, A::Error> {
        let mut map = PropertyMap::new();
        while let Some(triple) = seq.next_element::<(u32, u32, String)>()? {
            let (key, value) = DECODER
                .with(|slot| match slot.borrow().as_ref() {
                    Some(decoder) => decoder.resolve(triple),
                    None => Err("property triples outside a stored snapshot".to_string()),
                })
                .map_err(de::Error::custom)?;
            map.insert(key, value);
        }
        Ok(map)
    }
}

/// `#[serde(with = "property_codec::map")]` for a `PropertyMap` field.
pub mod map {
    use super::*;

    pub fn serialize<S: Serializer>(map: &PropertyMap, serializer: S) -> Result<S::Ok, S::Error> {
        match ENCODER.with(|slot| slot.borrow_mut().as_mut().map(|encoder| encoder.triples(map))) {
            Some(triples) => triples.serialize(serializer),
            None => map.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PropertyMap, D::Error> {
        deserializer.deserialize_any(MapVisitor)
    }
}

/// `skip_serializing_if` for a type-property bag: an empty one is not written.
///
/// Absent and empty mean the same on every reader (`#[serde(default)]`), so this
/// loses nothing -- and it is how a wire result drops each element's copy of its
/// bag once `service::type_table` has moved the bag into the response's table.
pub fn is_empty(map: &Arc<PropertyMap>) -> bool {
    map.is_empty()
}

/// `#[serde(with = "property_codec::shared_map")]` for a type-property bag,
/// `Arc<PropertyMap>`: stored as an index into the snapshot's table of bags.
pub mod shared_map {
    use super::*;

    pub fn serialize<S: Serializer>(map: &Arc<PropertyMap>, serializer: S) -> Result<S::Ok, S::Error> {
        match ENCODER.with(|slot| slot.borrow_mut().as_mut().map(|encoder| encoder.type_set(map))) {
            Some(index) => serializer.serialize_u32(index),
            None => (**map).serialize(serializer),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<PropertyMap>, D::Error> {
        struct SharedVisitor;
        impl<'de> Visitor<'de> for SharedVisitor {
            type Value = Arc<PropertyMap>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a property map, or a type-property index inside a stored snapshot")
            }
            fn visit_map<A: MapAccess<'de>>(self, access: A) -> Result<Self::Value, A::Error> {
                MapVisitor.visit_map(access).map(Arc::new)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                MapVisitor.visit_seq(seq).map(Arc::new)
            }
            fn visit_u64<E: de::Error>(self, index: u64) -> Result<Self::Value, E> {
                DECODER
                    .with(|slot| match slot.borrow().as_ref() {
                        Some(decoder) => {
                            decoder.type_sets.get(index as usize).cloned().ok_or_else(|| {
                                format!("type-property index {index} is outside the snapshot's dictionary")
                            })
                        }
                        None => Err("a type-property index outside a stored snapshot".to_string()),
                    })
                    .map_err(E::custom)
            }
        }
        deserializer.deserialize_any(SharedVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Element {
        #[serde(with = "map")]
        properties: PropertyMap,
        #[serde(with = "shared_map")]
        type_properties: Arc<PropertyMap>,
    }

    fn element() -> Element {
        let props: PropertyMap = [("Mark".into(), CustomValue { value: "D-1".into(), storage_type: None })]
            .into_iter()
            .collect();
        Element { properties: props.clone(), type_properties: Arc::new(props) }
    }

    /// With no encoder active a property map is written as the plain map it
    /// always was. This is what keeps the push wire and every response
    /// unchanged while storage alone changes shape.
    #[test]
    fn test_without_an_encoder_the_maps_are_plain() {
        let json = serde_json::to_value(element()).unwrap();
        let expected = serde_json::json!({"value": "D-1", "storage_type": null});
        assert_eq!(json["properties"]["Mark"], expected);
        assert_eq!(json["type_properties"]["Mark"], expected);
    }

    /// Triples or a type index outside a stored snapshot name a dictionary that
    /// is not there. That is a malformed input, never an empty map.
    #[test]
    fn test_indices_without_a_dictionary_are_an_error() {
        assert!(serde_json::from_str::<Element>(r#"{"properties":[[0,0,"x"]],"type_properties":{}}"#).is_err());
        assert!(serde_json::from_str::<Element>(r#"{"properties":{},"type_properties":0}"#).is_err());
    }

    /// An index past the end of the dictionary is an error naming the index,
    /// not a panic and not a silently dropped property.
    #[test]
    fn test_an_index_outside_the_dictionary_is_an_error() {
        let decoder = Decoder::from_trailer(
            br#","property_codec":1,"property_keys":["Mark"],"storage_types":[null],"type_property_sets":[]}"#,
        )
        .unwrap();
        let err = decoder
            .decode(|| serde_json::from_str::<Element>(r#"{"properties":[[4,0,"x"]],"type_properties":{}}"#))
            .err()
            .expect("index 4 names no key");
        assert!(err.to_string().contains("index 4"), "{err}");
    }

    /// A dictionary from a future layout is refused rather than misread.
    #[test]
    fn test_an_unknown_codec_version_is_refused() {
        let line = br#","property_codec":99,"property_keys":[],"storage_types":[],"type_property_sets":[]}"#;
        assert!(Decoder::from_trailer(line).is_err());
    }
}
