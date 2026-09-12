// How this project's plan is coloured, per entity.
//
// **A grid, not a list of blocks**, because the question a reader brings here
// is comparative: "is the ceiling line going to be tellable apart from the room
// line underneath it?" Six rows against four columns answers that by being
// looked at; six separate panels would make anyone scroll to compare two
// colours that sit a few inches apart on the plan.
//
// **Every cell that exists is backed by something drawn.** The gaps in the grid
// are the point rather than an omission -- a space has no fill and no pick
// index, so a fill or a selection colour for it would be a control that does
// nothing. The three Rust types behind this (`RoomAppearance`,
// `ElementAppearance`, `OutlineAppearance`) are shaped so those cells cannot be
// written by accident: there is no field to bind them to.
//
// The fallbacks below are the renderer's own hard defaults from
// `gl/colour.ts::readPalette`, not the live CSS variables. They are what the
// swatch SHOWS while a colour is unset, so they only need to be the right
// colour in spirit -- reading the live theme here would make the swatch chase
// the editor's theme rather than describe the viewer's.

import { Block, OptionalColour } from "../fields.js";
import type { Appearance, ElementAppearance, OutlineAppearance, RoomAppearance, Settings } from "../types.js";

/** `readPalette`'s fallbacks, and the theme token each one comes from. */
const INK = "#222222";
const FILL = "#dddddd";
const FILL_HOVER = "#cccccc";
const ACCENT = "#c8102e";

const EMPTY: Appearance = {};

export function AppearanceSection({
  settings,
  edit,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
}) {
  const appearance: Appearance = settings.appearance ?? EMPTY;

  const editAppearance = (change: (a: Appearance) => Appearance) =>
    edit((s) => ({ ...s, appearance: change(s.appearance ?? EMPTY) }));

  const rooms: RoomAppearance = appearance.rooms ?? {};
  const editRooms = (change: (r: RoomAppearance) => RoomAppearance) =>
    editAppearance((a) => ({ ...a, rooms: change(a.rooms ?? {}) }));

  // One row builder for the three element layers: they share a shape because
  // they share a drawing convention (footprint, glyph over it, selection ring),
  // which is the same reason `ElementAppearance` is one type rather than three.
  const elementRow = (
    key: "doors" | "windows" | "ffe",
    label: string,
    glyph: string,
  ) => {
    const current: ElementAppearance = appearance[key] ?? {};
    const editElement = (change: (e: ElementAppearance) => ElementAppearance) =>
      editAppearance((a) => ({ ...a, [key]: change(a[key] ?? {}) }));
    return (
      <tr key={key}>
        <th scope="row">{label}</th>
        <td>
          <OptionalColour
            value={current.line}
            fallback={INK}
            title={`The ${glyph} drawn over the footprint.`}
            onChange={(line) => editElement((e) => ({ ...e, line }))}
          />
        </td>
        <td>
          <OptionalColour
            value={current.fill}
            fallback={INK}
            title="The footprint rectangle under the glyph. Drawn semi-transparent whatever colour it is given, so it never hides the room beneath it."
            onChange={(fill) => editElement((e) => ({ ...e, fill }))}
          />
        </td>
        <td>
          <OptionalColour
            value={current.selection}
            fallback={ACCENT}
            title="The ring drawn around this element when it is selected."
            onChange={(selection) => editElement((e) => ({ ...e, selection }))}
          />
        </td>
        <td className="cell-absent" title="Hover is a room-only state today, so there is nothing here to colour.">
          &mdash;
        </td>
      </tr>
    );
  };

  const outlineRow = (
    key: "spaces" | "ceilings",
    label: string,
    fallback: string,
    note: string,
  ) => {
    const current: OutlineAppearance = appearance[key] ?? {};
    return (
      <tr key={key}>
        <th scope="row">{label}</th>
        <td>
          <OptionalColour
            value={current.line}
            fallback={fallback}
            title={note}
            onChange={(line) => editAppearance((a) => ({ ...a, [key]: { ...(a[key] ?? {}), line } }))}
          />
        </td>
        <td className="cell-absent" title="Drawn as an outline over the rooms, never filled.">
          &mdash;
        </td>
        <td className="cell-absent" title="Not selectable: this layer has no pick index.">
          &mdash;
        </td>
        <td className="cell-absent" title="Not hoverable: this layer has no pick index.">
          &mdash;
        </td>
      </tr>
    );
  };

  return (
    <Block
      title="Appearance"
      hint={
        <>
          What each layer is drawn in. A colour left on <strong>theme</strong> follows the reader&apos;s light/dark
          palette, which is where every project starts &mdash; pin all fifteen and the plan will read correctly in one
          theme and badly in the other, so override what needs distinguishing and leave the rest. A dash is a colour
          this layer has nothing to apply it to.
        </>
      }
    >
      <table className="appearance-grid">
        <thead>
          <tr>
            <th scope="col">layer</th>
            <th scope="col">line</th>
            <th scope="col">fill</th>
            <th scope="col">selection</th>
            <th scope="col">hover</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <th scope="row">Rooms</th>
            <td>
              <OptionalColour
                value={rooms.line}
                fallback={INK}
                title="The room outline."
                onChange={(line) => editRooms((r) => ({ ...r, line }))}
              />
            </td>
            <td>
              <OptionalColour
                value={rooms.fill}
                fallback={FILL}
                title="The room fill. A room coloured by an active colour plan keeps the plan's colour -- the plan is the more specific answer and already wins."
                onChange={(fill) => editRooms((r) => ({ ...r, fill }))}
              />
            </td>
            <td>
              <OptionalColour
                value={rooms.selection}
                fallback={ACCENT}
                title="The dashed ring around the selected room."
                onChange={(selection) => editRooms((r) => ({ ...r, selection }))}
              />
            </td>
            <td>
              <OptionalColour
                value={rooms.hover}
                fallback={FILL_HOVER}
                title="The fill under the pointer. A room flagged by the QA report still hovers to the accent, which is a state beating a preference."
                onChange={(hover) => editRooms((r) => ({ ...r, hover }))}
              />
            </td>
          </tr>
          {elementRow("doors", "Doors", "swing arc and leaf")}
          {elementRow("windows", "Windows", "pane symbol")}
          {elementRow("ffe", "FF&E", "marker and orientation tick")}
          {outlineRow(
            "spaces",
            "Spaces",
            ACCENT,
            "The space ring. It draws in the accent by default because a space is a SERVICES boundary held up against the architecture -- colouring it like the rooms is a way to lose that contrast.",
          )}
          {outlineRow(
            "ceilings",
            "Ceilings",
            INK,
            "The ceiling ring. It draws in the room ink by default and is told apart from the room outline beneath it by its DASH, which is not settable here.",
          )}
        </tbody>
      </table>
    </Block>
  );
}
