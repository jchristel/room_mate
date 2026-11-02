// Pan and zoom for an SVG plan: a view box, wheel zoom about the cursor, drag to pan.
//
// Shared by the blueprint canvas and the assign canvas, which are two drawings of the same kind of
// thing (a room in millimetres, Y flipped on its way to the screen) and must move the same way.

import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";

export interface View {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PanZoom {
  view: View;
  svg: RefObject<SVGSVGElement | null>;
  onPointerDown(e: ReactPointerEvent<SVGSVGElement>): void;
  onPointerMove(e: ReactPointerEvent<SVGSVGElement>): void;
  /** Ends a drag; true when the press did not move, i.e. it was a click on the background. */
  onPointerUp(): boolean;
}

/** The view resets to `fitted` whenever that changes, and otherwise belongs to the person. */
export function usePanZoom(fitted: View): PanZoom {
  const [view, setView] = useState<View>(fitted);
  useEffect(() => setView(fitted), [fitted]);
  const svg = useRef<SVGSVGElement>(null);
  const drag = useRef<{ x: number; y: number; view: View; moved: boolean } | null>(null);

  // A native listener, because React's wheel is passive and a page that scrolls under the plan
  // while it zooms is unusable.
  useEffect(() => {
    const el = svg.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const m = el.getScreenCTM();
      if (!m) return;
      const at = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
      const k = e.deltaY > 0 ? 1.15 : 1 / 1.15;
      setView((v) => ({ x: at.x - (at.x - v.x) * k, y: at.y - (at.y - v.y) * k, w: v.w * k, h: v.h * k }));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const onPointerDown = useCallback(
    (e: ReactPointerEvent<SVGSVGElement>) => {
      drag.current = { x: e.clientX, y: e.clientY, view, moved: false };
      e.currentTarget.setPointerCapture(e.pointerId);
    },
    [view],
  );
  const onPointerMove = useCallback((e: ReactPointerEvent<SVGSVGElement>) => {
    const d = drag.current;
    const el = svg.current;
    if (!d || !el) return;
    const [dx, dy] = [e.clientX - d.x, e.clientY - d.y];
    if (Math.abs(dx) + Math.abs(dy) > 3) d.moved = true;
    const rect = el.getBoundingClientRect();
    // The viewBox keeps its aspect ratio, so the larger of the two scales is the true one.
    const k = Math.max(d.view.w / rect.width, d.view.h / rect.height);
    setView({ ...d.view, x: d.view.x - dx * k, y: d.view.y - dy * k });
  }, []);
  const onPointerUp = useCallback((): boolean => {
    const click = !!drag.current && !drag.current.moved;
    drag.current = null;
    return click;
  }, []);

  return { view, svg, onPointerDown, onPointerMove, onPointerUp };
}
