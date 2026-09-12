// The editor's small parts: a row, a labelled field, the inputs, and the
// reorder/remove controls every list section uses.
//
// **Extracted because the sections repeat, not to abstract for its own sake.**
// The page edits eleven sections and five of them are ordered lists with the
// same ↑/↓/✕ controls; the `static/settings.html` this page replaced solved that
// with `rowControls` and `textInput` helpers, and these are their typed
// equivalents. Anything used by exactly one section stays in that section's file.
//
// Every input here is CONTROLLED and writes straight through to the settings
// object the page holds. The one deliberate exception is `NumberInput`: a number
// field has to keep what was typed while it is half-typed ("-", "1.", "") and
// cannot round-trip that through a `number`, so it is string-backed and reports
// `null` until the text parses.

import type { ReactNode } from "react";

export function Row({ children }: { children: ReactNode }) {
  return <div className="row">{children}</div>;
}

/** A list row's leading marker: the index, or one of the page's ↳ / ↳↳ glyphs
 *  for a row that belongs to the one above it. */
export function RowHead({ children }: { children: ReactNode }) {
  return <span className="row-head">{children}</span>;
}

export function Field({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <label className="field">
      {label}
      {children}
    </label>
  );
}

interface TextInputProps {
  value: string;
  onChange: (value: string) => void;
  size?: number;
  placeholder?: string;
  /** A `<datalist>` id, for the property-name and CSV-label vocabularies. */
  list?: string;
  readOnly?: boolean;
  title?: string;
}

export function TextInput({ value, onChange, size = 16, placeholder, list, readOnly, title }: TextInputProps) {
  return (
    <input
      type="text"
      size={size}
      value={value}
      placeholder={placeholder ?? ""}
      list={list ?? undefined}
      readOnly={readOnly ?? false}
      title={title ?? undefined}
      onChange={(ev) => onChange(ev.target.value)}
    />
  );
}

/**
 * A text input over an optional string: blank in the box means `null` in the
 * settings, which is what "not configured" is on every one of these fields.
 *
 * Blank is normalised on the way out rather than on the way in, so typing a
 * space does not fight the cursor — the same call `saveBody` makes for the
 * top-level scalars.
 */
export function OptionalText({
  value,
  onChange,
  ...rest
}: Omit<TextInputProps, "value" | "onChange"> & {
  value: string | null | undefined;
  onChange: (value: string | null) => void;
}) {
  return <TextInput value={value ?? ""} onChange={(text) => onChange(text.trim() === "" ? null : text)} {...rest} />;
}

/**
 * A number input that keeps what is typed.
 *
 * `value` is the settings value; the box shows `text`, which the caller holds,
 * because "" and "-" and "1." are all states a `number` cannot represent. The
 * caller gets `null` while the text does not parse, which is exactly the shape
 * `Option<f64>` fields want — and for a required number (`max_wall_thickness`)
 * the caller keeps the last good value and lets the server reject a blank.
 */
export function NumberInput({
  text,
  onChange,
  size = 8,
  placeholder,
  title,
}: {
  text: string;
  onChange: (text: string, parsed: number | null) => void;
  size?: number;
  placeholder?: string;
  title?: string;
}) {
  return (
    <input
      type="text"
      inputMode="decimal"
      size={size}
      value={text}
      placeholder={placeholder ?? ""}
      title={title ?? undefined}
      onChange={(ev) => {
        const next = ev.target.value;
        const parsed = next.trim() === "" ? null : Number(next);
        onChange(next, parsed !== null && Number.isFinite(parsed) ? parsed : null);
      }}
    />
  );
}

/** A `<select>` over a string-union type — every enum in the settings tree
 *  generates as one, so the options are exhaustive by construction. */
export function Select<T extends string>({
  value,
  options,
  onChange,
  title,
}: {
  value: T;
  options: readonly (readonly [T, string])[];
  onChange: (value: T) => void;
  title?: string;
}) {
  return (
    <select value={value} title={title ?? undefined} onChange={(ev) => onChange(ev.target.value as T)}>
      {options.map(([option, label]) => (
        <option key={option} value={option}>
          {label}
        </option>
      ))}
    </select>
  );
}

/** The same, over an OPTIONAL enum: the first option is the absent state, whose
 *  label says what absent *means* rather than showing an empty row. */
export function OptionalSelect<T extends string>({
  value,
  options,
  absentLabel,
  onChange,
  title,
}: {
  value: T | null | undefined;
  options: readonly (readonly [T, string])[];
  absentLabel: string;
  onChange: (value: T | null) => void;
  title?: string;
}) {
  return (
    <select
      value={value ?? ""}
      title={title ?? undefined}
      onChange={(ev) => onChange(ev.target.value === "" ? null : (ev.target.value as T))}
    >
      <option value="">{absentLabel}</option>
      {options.map(([option, label]) => (
        <option key={option} value={option}>
          {label}
        </option>
      ))}
    </select>
  );
}

export function Check({
  checked,
  onChange,
  children,
  title,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  children: ReactNode;
  title?: string;
}) {
  return (
    <label className="field" title={title ?? undefined}>
      <input type="checkbox" checked={checked} onChange={(ev) => onChange(ev.target.checked)} />
      {children}
    </label>
  );
}

/**
 * One optional colour: a swatch, and a button that hands the answer back to the
 * theme.
 *
 * **Absent is a real, useful value here, not an empty field**, which is why a
 * bare `<input type="color">` will not do: it has no null. A colour left unset
 * means "whatever the reader's light/dark theme says", and that is the state
 * every project starts in and most should stay in for most entities -- a plan
 * with all fifteen colours pinned looks correct in one theme and unreadable in
 * the other.
 *
 * So the button is the important half of this control. It reads `theme` and is
 * disabled while the colour is following the theme, and `reset` once it is not,
 * which makes the two states tellable apart at a glance across a grid of
 * fifteen.
 *
 * `fallback` is what the swatch shows while absent -- the theme's own value for
 * this colour, passed in by the caller because only it knows which of the seven
 * palette entries this field falls back to.
 */
export function OptionalColour({
  value,
  fallback,
  onChange,
  title,
}: {
  value: string | null | undefined;
  fallback: string;
  /** `null` clears the override. NOT `undefined`: the generated types spell an
   *  absent `Option<String>` as `string | null`, and the page compiles under
   *  `exactOptionalPropertyTypes`, where those two are not interchangeable. */
  onChange: (value: string | null) => void;
  title?: string;
}) {
  const set = typeof value === "string" && value !== "";
  return (
    <span className="colour-field" title={title ?? undefined}>
      <input
        type="color"
        className={set ? "colour-swatch set" : "colour-swatch"}
        value={set ? value : fallback}
        onChange={(ev) => onChange(ev.target.value)}
      />
      <button
        type="button"
        className="colour-reset"
        disabled={!set}
        title={set ? "Clear this override and follow the theme again" : "Following the theme"}
        onClick={() => onChange(null)}
      >
        {set ? "reset" : "theme"}
      </button>
    </span>
  );
}

/**
 * Move-up / move-down / remove for one entry of an ordered list.
 *
 * Order is meaning in every list that uses this — hierarchy tiers are
 * outermost-first, room label fields are the label's reading order, colour
 * bands must be sorted — so the controls travel with the row rather than being
 * a drag-and-drop nicety.
 */
export function ListControls<T>({
  list,
  index,
  onChange,
}: {
  list: readonly T[];
  index: number;
  onChange: (next: T[]) => void;
}) {
  const move = (to: number) => {
    if (to < 0 || to >= list.length) return;
    const next = [...list];
    const [moved] = next.splice(index, 1);
    if (moved === undefined) return;
    next.splice(to, 0, moved);
    onChange(next);
  };
  return (
    <>
      <button type="button" className="action icon" title="move up" onClick={() => move(index - 1)}>
        ↑
      </button>
      <button type="button" className="action icon" title="move down" onClick={() => move(index + 1)}>
        ↓
      </button>
      <button
        type="button"
        className="action icon"
        title="remove"
        onClick={() => onChange(list.filter((_, i) => i !== index))}
      >
        ✕
      </button>
    </>
  );
}

/** A section of the editor: heading, the "why this exists" hint, and its rows. */
export function Block({ title, hint, children }: { title: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <section className="block">
      <h2>{title}</h2>
      {hint !== undefined && <p className="hint">{hint}</p>}
      {children}
    </section>
  );
}
