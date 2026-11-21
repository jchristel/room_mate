// Zoom and pan for the sheet drawing: 1 is the whole sheet, the view box then shrinks about a centre kept inside the
// paper. Ctrl or Cmd with the wheel (a trackpad's pinch is one) zooms about the pointer; a plain wheel still scrolls
// the page. The arithmetic is pure (`zoomedAbout`, `viewBoxOf`) and tested; the hook only holds the state and the
// listeners.

import { useEffect, useRef, useState, type PointerEvent, type RefObject } from "react";

/** How far the drawing may be zoomed in. */
export const MAX_ZOOM = 12;

/** The paper and the margin drawn round it (paper mm). */
const MARGIN_MM = 10;

export interface Paper {
  width: number;
  height: number;
}

export interface Zoom {
  k: number;
  /** Where the view is centred, paper mm; null is the sheet's middle (whole-sheet view). */
  c: { x: number; y: number } | null;
}

export const WHOLE: Zoom = { k: 1, c: null };

const middle = (paper: Paper) => ({ x: paper.width / 2, y: paper.height / 2 });

/** A centre kept inside the paper. */
export const kept = (paper: Paper, c: { x: number; y: number }) => ({ x: Math.min(paper.width, Math.max(0, c.x)), y: Math.min(paper.height, Math.max(0, c.y)) });

/** The view box (x, y, width, height) that shows `zoom`. */
export function viewBoxOf(paper: Paper, zoom: Zoom): { x: number; y: number; w: number; h: number; centre: { x: number; y: number } } {
  const [w, h] = [(paper.width + MARGIN_MM) / zoom.k, (paper.height + MARGIN_MM) / zoom.k];
  const centre = kept(paper, zoom.c ?? middle(paper));
  return { x: centre.x - w / 2, y: centre.y - h / 2, w, h, centre };
}

/** `zoom` changed by `by` (above 1 in, below 1 out), keeping the paper point `p` where it is on screen: its share of the
 * view box is the same before and after. Back to the whole sheet at 1. */
export function zoomedAbout(paper: Paper, zoom: Zoom, p: { x: number; y: number }, by: number): Zoom {
  const k = Math.min(MAX_ZOOM, Math.max(1, zoom.k * by));
  if (k === 1) return WHOLE;
  const c = zoom.c ?? middle(paper);
  const [w0, h0, w1, h1] = [(paper.width + MARGIN_MM) / zoom.k, (paper.height + MARGIN_MM) / zoom.k, (paper.width + MARGIN_MM) / k, (paper.height + MARGIN_MM) / k];
  return { k, c: { x: p.x - ((p.x - c.x) / w0) * w1, y: p.y - ((p.y - c.y) / h0) * h1 } };
}

/** What to spread on the `<svg>`, besides its view box, to pan it by dragging whatever a box or grip did not take. */
export interface PanHandlers {
  onPointerDown(e: PointerEvent<SVGSVGElement>): void;
  onPointerMove(e: PointerEvent<SVGSVGElement>): void;
  onPointerUp(): void;
  onPointerCancel(): void;
}

export function useZoomPan(svg: RefObject<SVGSVGElement | null>, paper: Paper | null) {
  const [zoom, setZoom] = useState<Zoom>(WHOLE);
  const [pan, setPan] = useState<{ x: number; y: number; c: { x: number; y: number } } | null>(null);
  // the wheel listener is native (React's is passive and cannot stop the page zooming too) and reads the latest of these
  const wheel = useRef<(e: WheelEvent) => void>(() => undefined);
  wheel.current = (e) => {
    if (!paper || !(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    const m = svg.current?.getScreenCTM();
    if (!m) return;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
    setZoom((z) => zoomedAbout(paper, z, { x: p.x, y: p.y }, e.deltaY < 0 ? 1.25 : 0.8));
  };
  useEffect(() => {
    const el = svg.current;
    if (!el) return;
    const on = (e: WheelEvent) => wheel.current(e);
    el.addEventListener("wheel", on, { passive: false });
    return () => el.removeEventListener("wheel", on);
  });

  const box = paper ? viewBoxOf(paper, zoom) : null;
  const handlers: PanHandlers = {
    onPointerDown: (e) => {
      if (zoom.k <= 1 || !box) return;
      e.currentTarget.setPointerCapture(e.pointerId);
      setPan({ x: e.clientX, y: e.clientY, c: box.centre });
    },
    onPointerMove: (e) => {
      if (!pan || !paper || !box) return;
      const r = e.currentTarget.getBoundingClientRect();
      setZoom((z) => ({ k: z.k, c: kept(paper, { x: pan.c.x - ((e.clientX - pan.x) / r.width) * box.w, y: pan.c.y - ((e.clientY - pan.y) / r.height) * box.h }) }));
    },
    onPointerUp: () => setPan(null),
    onPointerCancel: () => setPan(null),
  };
  return {
    k: zoom.k,
    box,
    handlers,
    zoomBy: (by: number) => paper && box && setZoom((z) => zoomedAbout(paper, z, box.centre, by)),
    fit: () => setZoom(WHOLE),
  };
}
