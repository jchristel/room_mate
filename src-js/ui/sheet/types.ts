// The shape of a sheet as duHast exports it, as far as the sheet drawing reads it. The sheet is opaque beyond its
// viewports; what each viewport's view carries depends on its kind.

/** A 2D box in millimetres on the sheet, as duHast exports it. */
export interface PaperBox {
  min_x: number;
  min_y: number;
  max_x: number;
  max_y: number;
}

export interface Point3 {
  x: number;
  y: number;
  z?: number;
}

/** What a viewport's view carries beyond its scale; each kind of view fills its own. */
export interface ViewInfo {
  view_scale?: number;
  name?: string;
  /** Schedule: rows in the schedule. */
  total_number_of_rows?: number;
  /** Elevation: which index of its marker the view is. */
  marker_index?: number | null;
  /** Elevation: the line in the model, mm; start to end runs to the viewer's right. */
  line_start?: Point3 | null;
  line_end?: Point3 | null;
  /** Plan and elevation: the annotation crop is on, and where its edges stand outside the view crop, mm on paper. */
  annotation_crop_active?: boolean | null;
  annotation_crop_offsets?: { left: number; right: number; top: number; bottom: number } | null;
  /** Plan: the crop box in the model's frame, mm. */
  bounding_box?: PaperBox;
  /** 3D: the direction the camera looks along (any length), and the section box in the model's frame, mm. */
  forward_direction?: Point3 | null;
  up_direction?: Point3 | null;
  section_box_min?: Point3 | null;
  section_box_max?: Point3 | null;
}

export interface ViewportInfo {
  /** "floor plan", "elevation", "three d" or "schedule". */
  vp_type?: string;
  /** The viewport's outline on the sheet. */
  bounding_box?: PaperBox;
  /** Where the viewport sits on the sheet, mm. */
  centre_point?: { x: number; y: number };
  view?: ViewInfo;
}

/** What `previewOf` draws: a sheet's measured paper size and its viewports. */
export interface SheetDrawing {
  /** Paper size measured from the title block, mm. */
  size_mm?: { width_mm: number; height_mm: number };
  /** The sheet as duHast exports it: opaque here beyond its viewports. */
  sheet: { view_ports?: ViewportInfo[] } & Record<string, unknown>;
}
