// The pieces every inspector is built from: a header, a section of rows, and
// the note that says why a section is empty.
//
// Shared rather than repeated because the four panels differ in WHAT they
// show, never in how a row reads — and a row that reads differently per entity
// would make two panels look like two products.

import { isEmptyPropValue } from "../../properties.js";

/** One key/value row. An empty value is rendered as a dash in a muted style
 *  rather than as blank space: "this property exists and holds nothing" and
 *  "this property is not here" are different, and only the first gets a row. */
export function Row({ label, value, onClick }: { label: string; value: string; onClick?: () => void }) {
  const empty = isEmptyPropValue(value);
  return (
    <div className={`insp-row${onClick ? " insp-link" : ""}`} {...(onClick ? { onClick } : {})}>
      <div className="insp-k">{label}</div>
      <div className={`insp-v${empty ? " insp-empty" : ""}`}>{empty ? "—" : value}</div>
    </div>
  );
}

/** A titled block of rows. Renders nothing when there are none, so a panel
 *  never shows a heading over emptiness — except where the caller wants that
 *  said explicitly, which is what `Note` is for. */
export function Section({
  title,
  rows,
  source,
}: {
  title: string;
  rows: readonly (readonly [string, string])[];
  source?: string;
}) {
  if (!rows.length) return null;
  const cls = source ? ` src-${source.replace(/[^a-zA-Z0-9_-]/g, "-")}${source === "model" ? "" : " src-reference"}` : "";
  return (
    <>
      <h4>{title}</h4>
      <div className={`insp-rows${cls}`}>
        {rows.map(([k, v]) => (
          <Row key={k} label={k} value={v} />
        ))}
      </div>
    </>
  );
}

export function Note({ children }: { children: React.ReactNode }) {
  return <div className="insp-note">{children}</div>;
}

/**
 * The header every kind shares: a title, a sub-line, and the zone the click
 * came from.
 *
 * The KIND is spelled out for anything that is not a room, because "which of
 * the things under the pointer did I just select" is the question the panel has
 * to answer first, now that the plan draws seven selectable kinds.
 */
export function Head({
  kind,
  title,
  sub,
  zoneId,
  children,
}: {
  kind: string;
  title: string;
  sub: string;
  zoneId: string | null;
  children?: React.ReactNode;
}) {
  return (
    <div className="insp-head">
      <div className="insp-title">{title}</div>
      <div className="insp-sub">
        {kind === "room" ? "" : `${kind.toUpperCase()} · `}
        {sub}
        {zoneId ? ` · from ${zoneId}` : ""}
      </div>
      {children}
    </div>
  );
}
