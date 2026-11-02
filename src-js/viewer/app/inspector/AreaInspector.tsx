// The panel for one hierarchy footprint.
//
// Every figure here is already client-side: the group comes from `/areas` and
// the net is summed from the rooms the page holds, so this panel answers
// "what is this one footprint" without a request.
//
// **Footprint and net are both shown, and their difference is named.** A
// dissolved footprint is always at least the summed net of its rooms; the gap
// is enclosed wall bands plus filled voids, while an open courtyard is excluded
// from the footprint. Showing one number alone would invite it to be quoted as
// "the area", which is the mistake STRATEGY-AREA-CALCULATION.md exists to stop.

import { areaKey, netAreaIndex, pathKey, tierLabel } from "../../areas.js";
import { useViewer } from "../useViewer.js";
import { Head, Note, Section } from "./parts.js";
import type { Selection } from "../store.js";

export function AreaInspector({ selection }: { selection: Selection }) {
  const { areas, payload } = useViewer();
  const group = areas?.groups.find((g) => areaKey(g) === selection.id);
  if (!group) return <Note>That footprint is not in the current scope.</Note>;

  const depth = group.path.length - 1;
  const levelName = areas?.levels?.find((l) => l.id === group.level_id)?.name ?? group.level_id;
  const net = netAreaIndex(payload?.rooms ?? []).get(`${group.level_id}|${depth}|${pathKey(group.path, depth)}`) ?? 0;
  const fmt = (n: number) => n.toLocaleString(undefined, { maximumFractionDigits: 1 });
  const rooms = (payload?.rooms ?? []).filter(
    (r) => r.level_id === group.level_id && r.classification && pathKey(r.classification, depth) === pathKey(group.path, depth),
  );

  return (
    <>
      <Head kind="area" title={tierLabel(group.path[depth])} sub={`${levelName} · ${group.path[depth]?.tier ?? ""}`} zoneId={selection.zoneId} />
      <Section
        title="Path"
        rows={group.path.map((t) => [t.tier, tierLabel(t)] as const)}
      />
      <Section
        title="Area (model units²)"
        rows={[
          ["Footprint", fmt(group.area)],
          ["Net rooms", fmt(net)],
          ["Δ", fmt(group.area - net)],
          ["Rooms", String(rooms.length)],
          // A group that does not count upward is FLAGGED rather than hidden:
          // its area is real, it just does not roll into the tiers above.
          ["Counts upward", group.counted_upward === false ? "no" : "yes"],
        ]}
      />
      <Note>
        Δ is enclosed wall bands plus filled room voids. Open courtyards are excluded from the footprint, so they appear
        in neither figure.
      </Note>
    </>
  );
}
