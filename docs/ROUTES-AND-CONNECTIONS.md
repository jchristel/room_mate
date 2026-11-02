# Routes, reach and connections in the viewer

Moved out of the README so the landing page stays short.

The viewer can find the shortest route between two rooms, and lets people state
the connections the model does not hold. All of it is in each zone's toolbar
(**Route**, **Connections**) and behind the endpoints in [the API](API.md); the reasoning,
measurements and open questions are in [the plan](PLAN-connectivity.md).

- **Routes.** Pick a start and an end room anywhere (the plan, the grid, a search
  chip). The route runs door to door by the exact shortest walk inside each room,
  or room centre to room centre. It can start and end at a point inside each room,
  and the start and end marks can be **dragged**. A **steps** list reads the whole
  route in order and marks where it changes level.
- **Width and height.** Give the size of the object (a bed, a trolley) and a door or
  open wall narrower than that, a door lower than it, or a **room** lower than it, is
  not a way through. The answer says the narrowest and lowest points and the largest
  object that fits. The route is drawn as wide as the object, so a corridor too narrow
  for it is visible. Door sizes are **estimates**; a room's height comes from the
  ceilings over it, else a room property, and where each is read from is named per
  project under **Routing** in the settings ([reference](SETTINGS.md)); the width of a corridor *inside* a room is not
  checked.
- **Open areas.** Rooms the model does not join by a door (bays, open-plan areas)
  can be declared one open space. Where members touch but do not connect, such as
  bays along a corridor, **Cut** closes a wall between two of them and **Only
  through…** makes the others connect only through one room.
- **Vertical links.** A lift or stair is a run of floor-to-floor links drawn by
  hand. The **stack view** shows only the levels that hold a room of the stack and
  suggests the next room above and below by plan overlap, for a person to confirm.
- **Saved routes.** Name a route, give it a colour and keep it for the whole
  project. Several can be shown at once, in their colours, and they appear in
  **Export SVGs** on every level they pass through, with a legend.
- **The tables.** While a connections editor or a route tool is open, the open
  areas, vertical links and saved routes are listed on the left of the plan, each
  with its levels and a delete.
- **Reach.** The **Reach** tool answers "what can be walked to from here": click one
  or more start rooms, set a walk length and a grid, and the plan shows a heat map of
  the walking distance from the nearest start (a hose reel's coverage, an exit's travel
  distance). It uses the same graph as the routes, so a cell's distance agrees with a
  route's length; **ignore cuts** drops the walls inside open areas. It is per zone, so
  two zones can compare two reaches.
- **Saved reaches.** Name a reach and keep it for the project beside the saved routes;
  they are listed on the left, and **Show** loads one into the focused zone. What is
  saved is the request, so a reach follows the model when it changes.
- **Tags.** Two toggles in the **Layers** menu, both off by default: **Route tags** show
  each hop's length and, in brackets, the walk from the start; **Reach tags** show the
  distance to the start in each grid cell. The heat map, the legend and any switched-on
  tags are included in **Export SVGs**.
- **Element tags.** Under FF&E, Spaces, Ceilings and Floors in the **Layers** menu, a
  **Tags** checkbox (off by default) writes text on each element, sized to fit its own
  outline like the room labels. What it says and its colours are set per entity in the
  settings page's **Element tags** section (`$id`, `$name`, `$type_name` or any property
  name). Included in **Export SVGs**.
