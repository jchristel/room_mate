// The stack view: the rooms joined by vertical links, as a column of levels, with
// the next hop up and down offered for a person to confirm.
//
// **Only the levels that hold a room of the stack are listed**, plus the one level a
// suggestion points at. RHH has two buildings with distinct levels, and a hospital
// lift's column must not list the car park's half-levels in between. The column is
// built from the rooms (`stack.ts`), never from the level list.
//
// **A suggestion is never a write.** `/stack` ranks the rooms over and under the
// end of the column; a link is saved only when a person presses Link, one hop at a
// time, and the column then grows and asks again. A room nothing sits over (a
// stair that moves, the top of a shaft) says so and stays a hop drawn by hand.

import { useEffect, useMemo, useState } from "react";

import { bodyAfterAddLink, bodyAfterDeleteLink, connectionsUrl, type Member } from "../connections.js";
import { buildStack, describeSide, percent, stackUrl, type StackAnswer, type StackSide } from "../stack.js";
import { put } from "./connectionsApi.js";
import { patchZoneEdit, setConnections, setZoneLevel, type ZoneRow } from "./store.js";
import { panToRoom } from "./zoneRegistry.js";
import { useViewer } from "./useViewer.js";

type Ask = { state: "idle" } | { state: "loading" } | { state: "error"; message: string } | { state: "done"; answer: StackAnswer };

export function StackPanel({ zone }: { zone: ZoneRow }) {
  const { scope, payload, connections } = useViewer();
  const edit = zone.edit!;
  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const url = connectionsUrl(scope);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const rooms = useMemo(() => new Map((payload?.rooms ?? []).map((r) => [r.id, r])), [payload]);
  const stack = useMemo(
    () => buildStack(edit.members[0]?.room_id ?? null, doc?.links ?? [], rooms, payload?.levels ?? []),
    [edit.members, doc, rooms, payload],
  );
  const topId = stack?.top?.id ?? null;
  const bottomId = stack?.bottom?.id ?? null;

  const asked = useAsk(scope.projectId, scope.milestone, topId, doc?.taken_at ?? "", payload?.revision ?? "");
  const askedBottom = useAsk(
    scope.projectId,
    scope.milestone,
    bottomId === topId ? null : bottomId,
    doc?.taken_at ?? "",
    payload?.revision ?? "",
  );
  const above = asked.state === "done" ? asked.answer : null;
  const belowAnswer = bottomId === topId ? above : askedBottom.state === "done" ? askedBottom.answer : null;

  const showRoom = (roomId: string) => {
    const room = rooms.get(roomId);
    if (!room) return;
    if (room.level_id) setZoneLevel(zone.id, room.level_id);
    window.setTimeout(() => panToRoom(room), 0);
  };

  const send = async (body: ReturnType<typeof bodyAfterAddLink>) => {
    if (!url || !doc) return;
    setSaving(true);
    setError(null);
    const r = await put(url, doc.taken_at, body);
    setSaving(false);
    if (r.ok) setConnections({ projectId: scope.projectId, doc: r.body, error: null });
    else setError(r.message);
  };

  const addLink = (from: Member, to: Member) => {
    if (doc) void send(bodyAfterAddLink(doc, from, to));
  };

  if (!stack) {
    return <div className="stackView hint">Pick a room (on the plan, in the grid, or from a search) to see its stack.</div>;
  }

  const topModel = above?.room.model_id ?? "";
  const bottomModel = belowAnswer?.room.model_id ?? "";

  return (
    <div className="stackView">
      <Suggestion
        label="above"
        side={above?.up ?? null}
        loading={asked.state === "loading"}
        error={asked.state === "error" ? asked.message : null}
        disabled={saving || !doc}
        onLink={(c) => addLink({ room_id: topId!, model_id: topModel || null }, { room_id: c.room_id, model_id: c.model_id })}
        onShow={showRoom}
      />
      <ol className="stackRows">
        {stack.rows.map((row, i) => (
          <li key={row.levelId}>
            <div className="stackRow">
              <span className="stackLevel">{row.levelName}</span>
              {row.rooms.map((r) => (
                <button
                  key={r.id}
                  className={`chip${r.id === edit.members[0]?.room_id ? " on" : ""}`}
                  onClick={() => showRoom(r.id)}
                  title="Show this room"
                >
                  {r.name}
                </button>
              ))}
            </div>
            {stack.joins[i] ? (
              <div className="stackJoin">
                {stack.joins[i]!.links.map((l) => (
                  <span key={l.id} className="stackLink">
                    ↕ linked{l.cost_ft != null ? `, ${l.cost_ft} ft` : ""}
                    <button
                      className="link"
                      title="Delete this vertical link"
                      onClick={() => {
                        if (doc && window.confirm("Delete this vertical link? Routes that used it will stop using it.")) {
                          void send(bodyAfterDeleteLink(doc, l.id));
                        }
                      }}
                    >
                      delete
                    </button>
                  </span>
                ))}
              </div>
            ) : null}
          </li>
        ))}
      </ol>
      <Suggestion
        label="below"
        side={belowAnswer?.down ?? null}
        loading={bottomId !== topId && askedBottom.state === "loading"}
        error={askedBottom.state === "error" ? askedBottom.message : null}
        disabled={saving || !doc}
        onLink={(c) => addLink({ room_id: bottomId!, model_id: bottomModel || null }, { room_id: c.room_id, model_id: c.model_id })}
        onShow={showRoom}
      />
      {error ? <span className="routeNotice">{error}</span> : null}
      <button className="link" onClick={() => patchZoneEdit(zone.id, { ...edit, members: [], error: null })}>
        pick another room
      </button>
    </div>
  );
}

/** One side's suggestion: the rooms over or under the end of the column. */
function Suggestion({
  label,
  side,
  loading,
  error,
  disabled,
  onLink,
  onShow,
}: {
  label: "above" | "below";
  side: StackSide | null;
  loading: boolean;
  error: string | null;
  disabled: boolean;
  onLink: (c: StackSide["candidates"][number]) => void;
  onShow: (roomId: string) => void;
}) {
  if (loading) return <div className="stackSuggest muted">Looking {label}…</div>;
  if (error) return <div className="stackSuggest"><span className="routeNotice">{error}</span></div>;
  if (!side) return <div className="stackSuggest muted">{label}: {describeSide(null)}</div>;
  return (
    <div className={`stackSuggest ${side.confidence}`}>
      <span className="stackLevel">{side.level_name}</span>
      <span className="muted">{describeSide(side)}</span>
      {side.candidates.map((c, i) => (
        <span key={`${c.model_id}/${c.room_id}`} className="stackCandidate">
          <button className="chip" onClick={() => onShow(c.room_id)} title="Show this room">
            {c.name}
          </button>
          <span className="muted">
            {percent(c.overlap)} over{c.same_stem ? ", same name" : ""}
          </span>
          <button className="ctl" disabled={disabled} onClick={() => onLink(c)} title="Save this vertical link">
            Link{side.confidence === "clear" && i === 0 ? "" : " this one"}
          </button>
        </span>
      ))}
    </div>
  );
}

/** The `/stack` answer for one room, asked again when the links or rooms move. */
function useAsk(
  projectId: string | null,
  milestone: string | null,
  roomId: string | null,
  version: string,
  revision: string,
): Ask {
  const [ask, setAsk] = useState<Ask>({ state: "idle" });
  useEffect(() => {
    const target = roomId ? stackUrl(projectId, roomId, milestone) : null;
    if (!target) {
      setAsk({ state: "idle" });
      return;
    }
    const ac = new AbortController();
    setAsk({ state: "loading" });
    void fetch(target, { cache: "no-store", signal: ac.signal })
      .then(async (res) => {
        if (ac.signal.aborted) return;
        if (res.status === 204) return setAsk({ state: "idle" });
        if (!res.ok) return setAsk({ state: "error", message: (await res.text()).trim() || `${target} -> ${res.status}` });
        setAsk({ state: "done", answer: (await res.json()) as StackAnswer });
      })
      .catch((err) => {
        if (!ac.signal.aborted) setAsk({ state: "error", message: `Could not read ${target}: ${String(err)}` });
      });
    return () => ac.abort();
  }, [projectId, milestone, roomId, version, revision]);
  return ask;
}
