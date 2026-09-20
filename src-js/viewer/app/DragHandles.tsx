// The two draggable edges of the bottom region.
//
// `#regionDrag` moves the region's OUTER edge against the plans; `#bandDivide`
// moves the boundary INSIDE it, between band 1's results and the grid. The two
// never fight, because one moves the outer edge and the other an inner one.
//
// Both write a CSS custom property rather than an inline height: the stylesheet
// already expresses the whole layout in terms of `--bottom-h` and `--band1-h`,
// including the two shrink states a folded region uses, and a height written
// onto the element would outrank all of it.

import { useCallback } from "react";

/** Leave at least this much of the plans visible, whatever the drag asks for:
 *  a region dragged to the top turns the page into a table with a sliver of
 *  plan, which is not a state anyone chooses on purpose. */
const MIN_PLAN_PX = 120;
const MIN_REGION_PX = 80;

export function RegionDrag() {
  const onPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    handle.classList.add("dragging");
    const move = (ev: PointerEvent) => {
      const height = Math.min(
        Math.max(window.innerHeight - ev.clientY, MIN_REGION_PX),
        Math.max(window.innerHeight - MIN_PLAN_PX, MIN_REGION_PX),
      );
      document.documentElement.style.setProperty("--bottom-h", `${height}px`);
    };
    const up = () => {
      handle.classList.remove("dragging");
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }, []);

  return <div id="regionDrag" title="Drag to resize the whole panel" onPointerDown={onPointerDown} />;
}

export function BandDivide({ visible }: { visible: boolean }) {
  const onPointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    const handle = e.currentTarget;
    const band1 = document.getElementById("band1");
    if (!band1) return;
    handle.setPointerCapture(e.pointerId);
    handle.classList.add("dragging");
    const top = band1.getBoundingClientRect().top;
    const move = (ev: PointerEvent) => {
      const height = Math.max(ev.clientY - top, 40);
      band1.classList.add("sized");
      band1.style.setProperty("--band1-h", `${height}px`);
    };
    const up = () => {
      handle.classList.remove("dragging");
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }, []);

  // Shown only while a band-1 block is expanded: there is nothing to
  // redistribute between a collapsed summary strip and the grid.
  return (
    <div
      id="bandDivide"
      className={visible ? "" : "hidden"}
      title="Drag to split space between the results and the table"
      onPointerDown={onPointerDown}
    />
  );
}

/**
 * The split between two adjacent band-1 blocks.
 *
 * `#bandDivide`'s role — band 1 against band 2 — generalised to a pair sharing
 * band 1's own height. Shown only when BOTH neighbours are open: a collapsed
 * block is a fixed strip, with nothing to trade height with.
 *
 * Dragging gives the block ABOVE an explicit height (`.sized`, `--band-h`)
 * rather than an equal automatic share. `flex: 1 1` keeps it yielding under
 * real space pressure — the region shrunk, a third block opened — so the
 * squeeze shows up as internal scrolling rather than the pair spilling out of
 * band 1.
 */
export function BandSplit({ visible, aboveId, title }: { visible: boolean; aboveId: string; title: string }) {
  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const handle = e.currentTarget;
      const above = document.getElementById(aboveId);
      if (!above) return;
      handle.setPointerCapture(e.pointerId);
      handle.classList.add("dragging");
      const top = above.getBoundingClientRect().top;
      const move = (ev: PointerEvent) => {
        above.classList.add("sized");
        above.style.setProperty("--band-h", `${Math.max(ev.clientY - top, 40)}px`);
      };
      const up = () => {
        handle.classList.remove("dragging");
        handle.removeEventListener("pointermove", move);
        handle.removeEventListener("pointerup", up);
      };
      handle.addEventListener("pointermove", move);
      handle.addEventListener("pointerup", up);
    },
    [aboveId],
  );

  return (
    <div className={`band-split${visible ? "" : " hidden"}`} title={title} onPointerDown={onPointerDown} />
  );
}
