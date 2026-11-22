//! The storeys on screen: which elements a read keeps (`StoreyScope`, `StoreyAdmitter`). A superset of the viewer's own rule
//! (`renderer/storey.ts`), never a re-implementation of it.

use super::*;

/// The storeys an element read is narrowed to -- the ones the viewer is
/// showing, which may be several (one per zone).
///
/// **Why the server narrows at all.** An element read used to ship every storey
/// and let the viewer throw most of it away: a large project's `/ffe` is tens of thousands of items across
/// dozens of storeys to draw the one or two a reader is looking at. Parsing what
/// is stored still reads the whole snapshot, but skipping an element here spares
/// its clone, its reference joins, its geometric probe, its serialisation, the
/// transfer and the browser's parse.
///
/// **A superset of what the viewer keeps, never a second opinion.** The rule
/// that puts an element on a storey is `onStorey` in `src-js/renderer/storey.ts`
/// -- name AND elevation, with announced fallbacks decided over the WHOLE
/// payload. The server does not re-implement that; it only guarantees that
/// everything `onStorey` could keep for any requested storey is still in the
/// body, so every one of its decisions comes out the same:
///
/// - a model declaring levels keeps an element whose own level is within
///   `LEVEL_EPS_MM` of ANY requested elevation. Elevation alone, deliberately
///   looser than `onStorey`'s name-and-elevation: the viewer's "by elevation"
///   fallback needs those elements, and it narrows the rest itself;
/// - a model declaring NO levels keeps an element whose `level_id` is one of the
///   requested storeys' ids -- `onStorey`'s id fallback for such a model;
/// - a read where no model declares any level is not narrowed at all, because
///   `onStorey` then shows everything and says so.
///
/// `levels_by_model` on the response stays whole, since those fallbacks read it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreyScope {
    pub(super) elevations: Vec<f64>,
    pub(super) level_ids: std::collections::BTreeSet<String>,
}

impl StoreyScope {
    /// From the query's `storey_elevations` and `storey_level_ids`, both
    /// comma-separated. `None` when neither is given, which is an unscoped read.
    ///
    /// Two independent sets rather than pairs, because the rule uses them
    /// independently: elevations for models that declare levels, ids for models
    /// that do not.
    pub fn parse(elevations: Option<&str>, level_ids: Option<&str>) -> Result<Option<Self>, String> {
        if elevations.is_none() && level_ids.is_none() {
            return Ok(None);
        }
        let parts = |s: Option<&str>| {
            s.unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let elevations = parts(elevations)
            .iter()
            .map(|p| {
                p.parse::<f64>()
                    .ok()
                    .filter(|e| e.is_finite())
                    .ok_or_else(|| format!("storey_elevations: {p:?} is not a number"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(Self { elevations, level_ids: parts(level_ids).into_iter().collect() }))
    }

    /// This scope prepared against one read's own levels.
    pub fn admitter<'a>(
        &'a self,
        levels_by_model: &'a BTreeMap<String, Vec<crate::contract::Level>>,
    ) -> StoreyAdmitter<'a> {
        let by_model: BTreeMap<&str, BTreeMap<&str, f64>> = levels_by_model
            .iter()
            .map(|(model, levels)| (model.as_str(), levels.iter().map(|l| (l.id.as_str(), l.elevation)).collect()))
            .collect();
        let have_levels = by_model.values().any(|levels| !levels.is_empty());
        StoreyAdmitter { scope: self, by_model, have_levels }
    }
}

/// A `StoreyScope` resolved against one read's `levels_by_model`: built once,
/// asked once per element.
pub struct StoreyAdmitter<'a> {
    pub(super) scope: &'a StoreyScope,
    pub(super) by_model: BTreeMap<&'a str, BTreeMap<&'a str, f64>>,
    pub(super) have_levels: bool,
}

impl StoreyAdmitter<'_> {
    /// Whether an element on `level_id` of `model_id` belongs in the body. See
    /// `StoreyScope` for the three cases and why each is a superset.
    pub fn admits(&self, model_id: &str, level_id: &str) -> bool {
        if !self.have_levels {
            return true;
        }
        match self.by_model.get(model_id).filter(|levels| !levels.is_empty()) {
            None => self.scope.level_ids.contains(level_id),
            Some(levels) => levels.get(level_id).is_some_and(|elevation| {
                self.scope
                    .elevations
                    .iter()
                    .any(|wanted| (elevation - wanted).abs() <= room_locator::LEVEL_EPS_MM)
            }),
        }
    }
}

/// What an element gives the locator, whatever entity it is.
///
/// **`normal` is the whole difference between the two entities' geometry.** An
/// opening carries one and is probed on both sides of it; an item does not and
/// is tested where it stands. Carrying it as `Option` rather than splitting the
/// struct keeps one preparation path -- position, elevation, frame, probe size
/// -- which is the part that is genuinely identical and the part most likely to
/// drift if it were written twice.
pub struct Probe<'a> {
    pub insertion_point: Option<Point2D>,
    pub loops: &'a [Loop],
    pub level_id: &'a str,
    /// The plan direction to step along. `None` for an element that sits in a
    /// room rather than between two, where there is nothing to step off.
    pub normal: Option<Point2D>,
}
