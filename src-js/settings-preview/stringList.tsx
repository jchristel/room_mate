// An ordered list of strings, edited as chips.
//
// Five settings are this shape — the room label, the project's comparison
// properties, each entity policy's comparison properties, a space policy's room
// models, and a hierarchy colour plan's tiers — and in every one of them ORDER
// IS MEANING: the label's reading order, the comparison report's column order.
// So the chips carry move-left/move-right rather than being a set.
//
// The add box is deliberately separate from the chips: a chip is a value that
// exists, and a half-typed one is not that yet. Enter adds, because the field is
// otherwise a mouse round-trip per entry.

import { useState } from "react";
import type { KeyboardEvent } from "react";

export function StringList({
  values,
  onChange,
  placeholder,
  list,
  addLabel = "+ field",
}: {
  values: readonly string[];
  onChange: (next: string[]) => void;
  placeholder?: string;
  /** A `<datalist>` id, when the vocabulary is known (property names, CSV columns). */
  list?: string;
  addLabel?: string;
}) {
  const [draft, setDraft] = useState("");

  const add = () => {
    const value = draft.trim();
    if (value === "") return;
    onChange([...values, value]);
    setDraft("");
  };

  const move = (from: number, to: number) => {
    if (to < 0 || to >= values.length) return;
    const next = [...values];
    const [moved] = next.splice(from, 1);
    if (moved === undefined) return;
    next.splice(to, 0, moved);
    onChange(next);
  };

  const onKeyDown = (ev: KeyboardEvent<HTMLInputElement>) => {
    if (ev.key === "Enter") {
      ev.preventDefault();
      add();
    }
  };

  return (
    <>
      <div className="chips">
        {values.map((value, index) => (
          <span className="chip" key={`${index}:${value}`}>
            <span>{value}</span>
            <button type="button" title="move left" onClick={() => move(index, index - 1)}>
              ‹
            </button>
            <button type="button" title="move right" onClick={() => move(index, index + 1)}>
              ›
            </button>
            <button type="button" title="remove" onClick={() => onChange(values.filter((_, i) => i !== index))}>
              ✕
            </button>
          </span>
        ))}
        {values.length === 0 && <span className="row-head">none</span>}
      </div>
      <label className="field">
        <input
          type="text"
          size={16}
          value={draft}
          placeholder={placeholder ?? ""}
          list={list ?? undefined}
          onChange={(ev) => setDraft(ev.target.value)}
          onKeyDown={onKeyDown}
        />
      </label>
      <button type="button" className="action" onClick={add}>
        {addLabel}
      </button>
    </>
  );
}
