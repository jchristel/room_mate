"""The blueprint export window: the rooms of a phase, each with the blueprint's name and its sheets.
Drawn from `blueprint_export.xaml`.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The same approach as the other windows (`sheet_export_window`, `export_window`): no x:Class, no
DataGrid or ListView, handlers are plain methods named in the XAML, rules in
`room_m.blueprint_export_form`. The rows are a StackPanel of small Grids made in code **when the phase
is chosen** (the rooms depend on it), and a room's sheet list is a Popup of check boxes **made when it
is first opened**. A sheet ticked for one room is greyed in the others, because a sheet is offered once.

What the person chose is kept on the row (`state["chosen"]`), not read back from the list, so the matching rule can
set a room's sheets before its list was ever opened. A ticked row whose blueprint name another ticked row shares is
shaded and says so; the rule and the filter are `room_m.match_rule`, shared with the sheet window.

**Not yet run in Revit.**
"""

from room_m import server_config
import os

from System.Windows import Thickness
from System.Windows.Controls import Button, CheckBox, ColumnDefinition, Grid, StackPanel, TextBox
from System.Windows.Controls import ScrollBarVisibility, ScrollViewer
from System.Windows.Controls.Primitives import Popup, PlacementMode
from System.Windows.Media import Color, SolidColorBrush
from System.Windows import GridLength, GridUnitType, HorizontalAlignment, VerticalAlignment, Visibility

from room_m import blueprint_export_form as form
from room_m import export_form, match_rule
from room_m.utils import sheets as utils_sheets

XAML_PATH = os.path.join(os.path.dirname(__file__), "blueprint_export.xaml")


def _column(width):
    column = ColumnDefinition()
    column.Width = GridLength(1, GridUnitType.Star) if width is None else GridLength(width)
    return column


SHADE = SolidColorBrush(Color.FromRgb(255, 224, 224))


def show(forms, model_name, projects, phases, rooms_for, sheets, saved=None, phase_lookup=None):
    """Show the window. `phases` are phase names, `rooms_for(phase)` the pick-list entries of the
    placed rooms of that phase and `sheets` those of the model's sheets (`room_m.utils.sheets`). `saved` is what
    `room_m.prefs` kept from the last export (project, filter, sheet rule) and `phase_lookup(project_id)` the phases
    the server holds for this model, so the phase can be preselected as the export window does.
    Returns `{"project", "phase", "plan": [(room entry, name, [sheet entries])], "filter", "rule"}` for the rooms
    ticked, or None when cancelled."""

    class BlueprintExportWindow(forms.WPFWindow):
        def __init__(self):
            forms.WPFWindow.__init__(self, XAML_PATH)
            self.result = None
            self._ready = False
            self._projects = list(projects)
            self._phases = list(phases)
            self._rows = []
            self.tx_model.Text = "Model: {}".format(model_name)
            self.tx_server.Text = server_config.short()
            self.cb_project.ItemsSource = [p["name"] for p in self._projects]
            index, text, rule = form.remembered(saved, self._projects)
            self.cb_phase.ItemsSource = self._phases
            if index is not None:
                self.cb_project.SelectedIndex = index
            elif len(self._projects) == 1:
                self.cb_project.SelectedIndex = 0
            self._phase_for_project()
            if self._phase() is None and len(self._phases) == 1:
                self.cb_phase.SelectedIndex = 0
            self.cb_rule_relation.ItemsSource = [label for _key, label in form.RELATIONS]
            self.cb_rule_relation.SelectedIndex = [k for k, _l in form.RELATIONS].index(rule["relation"]) if rule else 0
            self.tb_rule_room.Text = rule["room"] if rule else match_rule.DEFAULT_ROOM
            self.tb_rule_sheet.Text = rule["sheet"] if rule else match_rule.DEFAULT_SHEET
            self.tb_find.Text = text
            self._ready = True
            self._fill_rooms()
            self._rule_note()
            self._status()

        # -- rows -----------------------------------------------------------------------------

        def _fill_rooms(self):
            """One row per placed room of the chosen phase; the earlier rows and their choices go."""
            self.sp_rooms.Children.Clear()
            self._rows = []
            phase = self._phase()
            if phase is None:
                return
            for entry in rooms_for(phase):
                self._add_row(entry)
            self._filter(self.tb_find.Text)
            self._mark_names()

        def _add_row(self, entry):
            row = Grid()
            row.Margin = Thickness(0, 2, 0, 2)
            row.ColumnDefinitions.Add(_column(None))
            row.ColumnDefinitions.Add(_column(260))
            row.ColumnDefinitions.Add(_column(170))

            tick = CheckBox()
            tick.Content = entry["label"]
            tick.VerticalAlignment = VerticalAlignment.Center
            tick.Checked += self.on_change
            tick.Unchecked += self.on_change
            row.Children.Add(tick)

            name = TextBox()
            name.Text = form.default_name(entry)
            name.Margin = Thickness(6, 0, 0, 0)
            name.TextChanged += self.on_change
            Grid.SetColumn(name, 1)
            row.Children.Add(name)

            button = Button()
            button.Content = form.sheets_caption([])
            button.Margin = Thickness(6, 0, 0, 0)
            button.HorizontalContentAlignment = HorizontalAlignment.Left
            Grid.SetColumn(button, 2)
            row.Children.Add(button)

            state = {"entry": entry, "row": row, "tick": tick, "name": name, "button": button, "popup": None, "boxes": [], "chosen": []}
            button.Click += lambda sender, args, s=state: self._open_sheets(s)
            self.sp_rooms.Children.Add(row)
            self._rows.append(state)

        def _open_sheets(self, state):
            if state["popup"] is None:
                state["popup"] = self._make_popup(state)
            self._grey_taken()
            state["popup"].IsOpen = True

        def _make_popup(self, state):
            popup = Popup()
            popup.PlacementTarget = state["button"]
            popup.Placement = PlacementMode.Bottom
            popup.StaysOpen = False
            popup.Width = 360
            holder = StackPanel()
            holder.Background = state["button"].Background
            scroller = ScrollViewer()
            scroller.MaxHeight = 280
            scroller.VerticalScrollBarVisibility = ScrollBarVisibility.Auto
            inner = StackPanel()
            chosen_ids = set(sheet["id"] for sheet in state["chosen"])
            for sheet in sheets:
                box = CheckBox()
                box.Content = sheet["label"]
                box.Margin = Thickness(6, 2, 6, 2)
                box.IsChecked = sheet["id"] in chosen_ids
                box.Checked += lambda sender, args, s=state: self._sheets_changed(s)
                box.Unchecked += lambda sender, args, s=state: self._sheets_changed(s)
                inner.Children.Add(box)
                state["boxes"].append((sheet, box))
            scroller.Content = inner
            holder.Children.Add(scroller)
            popup.Child = holder
            return popup

        @staticmethod
        def _chosen_sheets(state):
            return list(state["chosen"])

        def _sheets_changed(self, state):
            """A box in a room's sheet list was ticked or unticked: the room's sheets are the ticked ones."""
            if state.get("syncing"):
                return
            self._set_sheets(state, [sheet for sheet, box in state["boxes"] if box.IsChecked], from_boxes=True)

        def _set_sheets(self, state, chosen, from_boxes=False):
            """The sheets of a room, kept on the row, shown on the button as their numbers and mirrored into the
            list if it was ever opened."""
            state["chosen"] = list(chosen)
            state["button"].Content = form.sheets_caption(state["chosen"])
            if not from_boxes and state["boxes"]:
                ids = set(sheet["id"] for sheet in state["chosen"])
                state["syncing"] = True
                try:
                    for sheet, box in state["boxes"]:
                        box.IsChecked = sheet["id"] in ids
                finally:
                    state["syncing"] = False
            self._grey_taken()
            self._status()

        def _grey_taken(self):
            """A sheet ticked for one room cannot be ticked for another."""
            held = [(s["entry"]["id"], [sh["id"] for sh in s["chosen"]]) for s in self._rows]
            for state in self._rows:
                taken = form.taken_elsewhere(held, state["entry"]["id"])
                for sheet, box in state["boxes"]:
                    box.IsEnabled = sheet["id"] not in taken

        def _filter(self, text):
            for state in self._rows:
                visible = form.matches(state["entry"]["label"], text)
                state["row"].Visibility = Visibility.Visible if visible else Visibility.Collapsed
            self.tx_found.Text = "{} of {} rooms".format(len(self._listed()), len(self._rows))

        def _listed(self):
            return [s for s in self._rows if s["row"].Visibility == Visibility.Visible]

        def _mark_names(self):
            """Shade every ticked row whose blueprint name another ticked row shares, and say why on the name."""
            if not self._ready:
                return
            shared = form.duplicate_names([(s["name"].Text, bool(s["tick"].IsChecked)) for s in self._rows])
            for i, state in enumerate(self._rows):
                if i in shared:
                    state["row"].Background = SHADE
                    state["name"].ToolTip = "Another checked room has this blueprint name: give each its own."
                else:
                    state["row"].Background = None
                    state["name"].ToolTip = None

        def _phase_for_project(self):
            """Preselect the phase the server holds for this model in the chosen project (it came with the project
            list), or the first phase for a first push."""
            index = self.cb_project.SelectedIndex
            if index < 0 or not self._phases:
                return
            held = phase_lookup(self._projects[index]["id"]) if phase_lookup else []
            phase, why = export_form.default_phase(self._phases, held)
            if phase is not None:
                self.cb_phase.SelectedIndex = self._phases.index(phase)
                self.tx_phase_note.Text = why + " The rooms listed are the placed rooms of the phase."

        def _rule(self):
            index = self.cb_rule_relation.SelectedIndex
            return form.make_rule(
                self.tb_rule_sheet.Text, form.RELATIONS[index][0] if index >= 0 else None, self.tb_rule_room.Text)

        def _rule_note(self, extra=""):
            self.tx_rule.Text = (form.describe_rule(self._rule()) + " " + extra).strip()

        def _phase(self):
            index = self.cb_phase.SelectedIndex
            return self._phases[index] if 0 <= index < len(self._phases) else None

        def _project(self):
            index = self.cb_project.SelectedIndex
            return self._projects[index] if 0 <= index < len(self._projects) else None

        def _ticked(self):
            return [s for s in self._rows if s["tick"].IsChecked]

        # -- handlers, named in the XAML ------------------------------------------------------

        def on_change(self, sender, args):
            self._mark_names()
            self._status()

        def on_project(self, sender, args):
            if self._ready:
                self._phase_for_project()
                self._fill_rooms()
                self._status()

        def on_check_matched(self, sender, args):
            """Tick the listed rooms that have a sheet (from the rule or by hand): a blueprint needs one, so a room
            without is left as it is and counted, and a rule that matched nothing is seen."""
            ticked = skipped = 0
            for s in self._listed():
                if s["chosen"]:
                    s["tick"].IsChecked = True
                    ticked += 1
                else:
                    skipped += 1
            self._rule_note("Checked {} room(s) that have sheets; {} listed room(s) have no sheet and were left.".format(
                ticked, skipped))

        def on_check_none(self, sender, args):
            for s in self._rows:
                s["tick"].IsChecked = False

        def on_rule_change(self, sender, args):
            if self._ready:
                self._rule_note()

        def on_apply_rule(self, sender, args):
            """Set the sheets of every LISTED room from the rule. A room the rule pairs with no sheet keeps what it
            had, and a sheet already held by a room outside the list is not taken from it."""
            rule = self._rule()
            listed = self._listed()
            rooms_listed = utils_sheets.ensure_params([s["entry"] for s in listed])
            utils_sheets.ensure_params(sheets)
            paired = unpaired = 0
            for state in listed:
                others = [sh["id"] for s in self._rows if s is not state for sh in s["chosen"]]
                found = match_rule.sheets_for_room(rule, state["entry"], sheets, set(others))
                if found:
                    self._set_sheets(state, found)
                    paired += 1
                else:
                    unpaired += 1
            self._rule_note(" ".join(
                ["{} room(s) got sheets; {} had no sheet that matches.".format(paired, unpaired)]
                + match_rule.unknown_parameters(rule, sheets, rooms_listed)))

        def on_phase(self, sender, args):
            if self._ready:
                self._fill_rooms()
                self._status()

        def on_find(self, sender, args):
            if self._ready:
                self._filter(self.tb_find.Text)

        def on_cancel(self, sender, args):
            self.result = None
            self.Close()

        def on_export(self, sender, args):
            if self._problems():
                self._status()
                return
            self.result = {
                "project": self._project(),
                "phase": self._phase(),
                "plan": [(s["entry"], s["name"].Text.strip(), self._chosen_sheets(s)) for s in self._ticked()],
                "filter": self.tb_find.Text,
                "rule": self._rule(),
            }
            self.Close()

        # -- helpers --------------------------------------------------------------------------

        def _problems(self):
            ticked = [
                (s["entry"]["label"], s["name"].Text, len(s["chosen"])) for s in self._ticked()
            ]
            return form.problems(self._project(), self._phase(), ticked)

        def _status(self):
            if not self._ready:
                return
            lines = self._problems()
            self.tx_status.Text = "\n".join(lines)
            self.bt_export.IsEnabled = not lines

    window = BlueprintExportWindow()
    window.ShowDialog()
    return window.result
