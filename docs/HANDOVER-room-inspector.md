# HANDOVER — the right-hand room inspector

**Status: steps 1–5 built and verified** (§7). The panel, the sections, the
hide-empty default and the type-to-filter box are in. **Step 6 — the checkbox
picker — is not built**; hide-empty plus the filter box turned out to cover the
cases it was meant for, so it is left as a recorded option rather than
speculative UI.

One design change came out of testing and is folded in below: **hide-empty must
not apply to the Classification section.** Applied uniformly it deleted that
section entirely for every `sample-project` room, whose tiers are all
resolved-but-`undefined` — see §5.1.
**Scope:** `static/index.html` only — no Rust, no server, no wire format. Every
field it displays is already on the `/rooms` response.
**Delivers:** HANDOVER-ui-layout **Decision 3**, the last unbuilt item in that
document, plus a property filter that Decision 3 did not anticipate needing.

---

## 1. Why this is buildable now

Decision 3 was blocked on one thing: *"the viewer has hover `<title>` tooltips,
not click selection."* **That blocker is gone.** Room click-selection landed with
the adjacency graph, and the selection layer was deliberately built as page state
with an extension point for exactly this consumer:

| What | Where | Note |
|---|---|---|
| `selectedRoomId` / `selectedZoneId` | index.html:634–635 | Page state. `selectedZoneId` is *a label*, not focus machinery. |
| `selectRoom(roomId, zoneId)` | index.html:2640 | Single entry point; notifies listeners. |
| `selectionListeners` | index.html:640 | The comment there already says: *"the future room inspector adds itself rather than editing `selectRoom`"*. |
| `applySelection(zone)` | index.html:2629 | Already draws the `.selected` outline in every zone showing the room. |
| `roomAtNode(zone, node)` | index.html:2647 | Click → room. |

So the inspector is a **listener plus a renderer**. It must not touch
`selectRoom`, must not introduce an `activeZoneId`, and must not re-render a
zone — selection changes are a class toggle today and must stay that way.

### Correction to HANDOVER-ui-layout

That document says the layout work reserved the column:

> `main` becomes `grid-template-columns: 1fr auto` — zones, inspector […] The CSS
> layout must tolerate a zero-width or `display: none` inspector column.

**It was never built that way, and the reason is structural rather than an
oversight.** `<main id="zones">` (index.html:443) *is* the zone container, and
its columns are written from JS on every zone add/remove:

```js
zonesEl.style.gridTemplateColumns = `repeat(${Math.min(zones.length, 3)}, 1fr)`;  // :2937
```

There is no spare column to reserve, and any static `grid-template-columns` on
`main` is overwritten by that line. `body` is a single column
(`minmax(0, 1fr)`, index.html:25). So this plan adds the column rather than
filling a reserved one — see §3.

## 2. What it shows, and what is actually there

Measured against House A (26 rooms) as served by `/rooms`:

- `id`, `name`, `level_id` — always.
- `label` — the project's configured `room_label` fields, already resolved
  server-side.
- `classification` — the resolved tier path (`[{tier, code, name, undefined}]`),
  the same one the areas service groups by.
- `properties` — **45 distinct names**, every one present on every room.
- `drofus` — the joined record, *absent* when the room did not match. House A has
  no dRofus configured, so this must render as "not joined" rather than empty.
- `drofus_labels[projectId]` on the payload — the source's full column
  vocabulary, including columns no room matched.

`loops` is also present and deliberately **not** shown: geometry belongs on the
plan, and the derived figures (area, adjacency degree) already have homes.

## 3. Layout

Add a wrapper so the middle band can hold two columns:

```html
<div id="mainRow">          <!-- body's 1fr row -->
  <main id="zones"></main>  <!-- unchanged, JS still owns its columns -->
  <aside id="inspector" class="hidden"></aside>
</div>
```

```css
#mainRow { display: grid; grid-template-columns: minmax(0, 1fr) auto; min-height: 0; }
#inspector { width: 22rem; border-left: 1px solid var(--ink); overflow-y: auto; min-height: 0; }
#inspector.hidden { display: none; }   /* no selection -> costs no width */
```

Three constraints, each learned the hard way elsewhere in this codebase:

- **`min-width: 0` must propagate down the new chain.** `body` already carries
  `minmax(0, 1fr)` with a comment explaining that a grid item's `min-width`
  defaults to `auto`; the wide band-2 table once stretched the page to ~4,900px
  and blanked the plan canvas because of it. A long unwrapped property value
  would do the same through `#mainRow`.
- **Not `position: absolute` inside `.zone-canvas`.** That is how
  `.validation-panel` and `.areas-panel` used to work, and moving them out was
  the point of the bottom-region work. The inspector reflows the plans; it does
  not cover them.
- **One inspector for the page, never one per zone** — selection is page state,
  so a per-zone inspector sits empty in every zone but one.

## 4. The property problem — measured, not assumed

45 properties per room is already awkward in a 22rem column. It is worse than
that number suggests:

> **19 of the 45 (42%) have no real value on *any* House A room** — the value is
> either blank or the literal string **`"None"`**, which is what the Revit
> extractor emits for an unset parameter. A further 2 have a value on ≤10% of
> rooms.

So the useful default view is **26 of 45**, reachable with no user configuration
at all. That single observation drives the design below, and the literal-`"None"`
detail is the part that would be missed: a naive `value === ""` emptiness test
hides nothing.

## 5. Filtering and selecting properties

Four mechanisms, sequenced by value-per-unit-of-UI. **Build 5.1 and 5.2 first**;
they need no state and no configuration and between them solve most of it.

### 5.1 Hide-empty toggle — default ON

One checkbox: *Hide empty*. Treats blank **and** literal `"None"`
(case-insensitive, trimmed) as empty. Cuts 45 → 26 on House A with zero effort.

Default on, because a property with no value on the selected room is noise. The
toggle exists at all so nobody has to wonder whether a field is missing or
merely unset — flick it and see. The count is shown either way
(*"22 of 45 model properties shown"*), so the hidden ones are never a silent
omission.

> **Scope it to the Model section only — learned in testing.** Applied to every
> section it deleted **Classification** outright on `sample-project`, whose
> 5,375 rooms all resolve to `Building [undefined] / Department [undefined]`.
> That is not missing data: the classifier ran and put them in the undefined
> bucket, which `service::areas` treats as a real group. So Classification is
> exempt from hide-empty (the name filter still applies to it) and an undefined
> tier renders as an explicit `(undefined)` rather than a dash. `inspApplyFilters`
> takes a `hideEmpty` option for this.

### 5.2 Type-to-filter box

A single input filtering on property *name*, substring, case-insensitive.
Instant, stateless, and the right answer to "I know it's called something like
*ceiling*". No persistence, cleared on selection change? **No — kept**, because
the common use is comparing the same field across rooms by clicking each in turn.

### 5.3 Checkbox picker — reuse the search field panel

For "always show me exactly these". There is an exact precedent to copy rather
than invent: `refreshFieldsPanel()` (index.html:2672) with `#fieldsPanel` /
`fieldsToggle`, which already implements the whole pattern — All/None actions,
ticks preserved across rebuilds, and **a genuinely new field defaults to on**
(index.html:2681) so a newly loaded project never silently hides data.

Copy that behaviour, including that last rule. Its CSS class (`.fields-panel`)
is reusable as-is.

### 5.4 Sections, always

Group rather than one flat list, in this order:

1. **Identity** — name, id, level, the zone the selection came from.
2. **Classification** — the resolved tier path.
3. **Model** — the 45.
4. **dRofus** — the joined record, or *"not joined"*.

Model and dRofus are visually separated because
[Sources](STRATEGY-SOURCES.md) keeps dRofus a distinct sub-object with its own
lifecycle and provenance — band 2 already honours this with grouped column
headers and a per-source tint, and the inspector must not be the one place that
blurs it.

## 6. Persistence — do nothing, deliberately

`linkViews`, the labels toggle, `showRoomLabels` and the search field selection
**all fail to persist today**, and HANDOVER-ui-layout Decision 4 records the
reason as a standing decision: *"Either persist global view prefs consistently or
not at all — a single persisted flag is the kind of inconsistency that confuses
the next reader."*

So: no `localStorage`, no URL param. If persistence is wanted later it should
arrive for every view preference at once.

The durable, shareable version of "which properties matter" already exists
server-side as `room_label` in project settings. If users start re-picking the
same columns every session, that is the signal to extend *that*, not to bolt
`localStorage` onto this panel.

## 7. Steps

1. **Wrapper + empty panel.** `#mainRow`, the CSS above, `#inspector.hidden`.
   Verify zones still size correctly at 1, 2 and 3 zones and that the plan canvas
   does not stretch.
2. **Register the listener.** `selectionListeners.push(renderInspector)`. Show on
   selection, hide on clear. No zone re-render.
3. **Sections 5.4** with all properties, unfiltered.
4. **Hide-empty (5.1)** including the literal-`"None"` rule, plus the
   *"n of m shown"* count.
5. **Type-to-filter (5.2).**
6. **Checkbox picker (5.3)** modelled on `refreshFieldsPanel`.

Steps 1–4 are a shippable increment.

## 8. Test plan — run 2026-07-26, all passing

Driven against the live viewer. Results in the right-hand column.

| # | Check | Result |
|---|---|---|
| 1 | Click a room | Panel appears; no zone re-render, pan/zoom kept |
| 2 | Same room selected from a third zone | Label follows: `2626838 · LEVEL 00 · from zone-2` |
| 3 | Clear selection | Panel hides, measured width **0** — zones reclaim it |
| 4 | 1 / 2 / 3 zones | Columns even (`1302px` → `650.5 650.5` → `433 433 433`), inspector holds 352px, no page overflow |
| 5 | 400-character property value | Wraps inside the panel; **page does not widen** — the `min-width: 0` chain holds |
| 6 | House A hide-empty | 22–23 of 45 on, 45 of 45 off; literal `"None"` counts as empty |
| 7 | `sample-project` dRofus | Matched room: tinted dRofus section. Unmatched: *"Not joined — this room matched no dRofus record."* |
| 8 | Filter box | `finish` → 2 rows; no match → `0 of 45`, stated not silently blank; survives selection change |
| 9 | 2s poll | Title, visibility and `.selected` outline all survive |
| 10 | SVG export | 4 polygons, no `insp-` markup — untouched |
| 11 | Project switch with a room selected | Selection cleared upstream ([:3212](../static/index.html:3212)), panel hides cleanly |

The original plan, for reference:

| # | Check | Expect |
|---|---|---|
| 1 | Click a room | Panel appears with that room; plan does not re-render, pan/zoom unchanged |
| 2 | Click the same room in a second zone showing the same level | Panel updates; `selectedZoneId` label names the second zone |
| 3 | Clear selection (click empty space) | Panel hides; zones reclaim the width |
| 4 | 1, 2 and 3 zones open | Zone columns still even; plan canvas never stretches horizontally |
| 5 | A very long property value | Wraps or scrolls inside the panel; page does not widen (the §3 trap) |
| 6 | House A room | Hide-empty on shows 26 of 45; off shows 45; literal `"None"` counts as empty |
| 7 | A project with dRofus (`sample-project`) | dRofus section populated; a room with no match reads "not joined", not blank |
| 8 | Filter box | Substring, case-insensitive; survives a selection change |
| 9 | Level switch / poll re-render | Selection and panel survive (`renderLevel` re-applies selection) |
| 10 | SVG export | Unaffected — the inspector is not inside the SVG |

## 9. Docs to update on landing

- **[STRATEGY-BROWSER.md](STRATEGY-BROWSER.md)** — add the inspector to
  Implemented; it is the third consumer of the selection layer.
- **[HANDOVER-ui-layout.md](HANDOVER-ui-layout.md)** — Decision 3 done. **That
  retires the document**, so it moves to `Superseded/` and Browser absorbs its
  outcome, exactly as its own header says.
- This file → `Superseded/` with it.

## 10. Deliberately out of scope

- **Editing.** Read-only, same stance as band 2. Editing means dirty state,
  per-field validation and conflict against the 2s poll.
- **Multi-select / compare two rooms.** Selection is deliberately singular; band
  2's grid is where N rooms are compared.
- **Duplicating band 2.** The grid answers *"how do these rooms compare"*; the
  inspector answers *"what is this one thing"* — the region model's split. If
  they converge, the inspector is wrong, not the grid.
- **Geometry display.** `loops` stays on the plan.
