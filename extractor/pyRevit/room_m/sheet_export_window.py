"""The sheet export window: sheets in one column, the rooms each covers in the other. Drawn from
`sheet_export.xaml`.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The same approach as the other windows (`export_window`, `rls_settings_window`): no x:Class, no DataGrid or
ListView, handlers are plain methods named in the XAML, rules in `room_m.sheet_export_form`. The rows are a
StackPanel of small Grids made in code. A room list is a Popup of check boxes **made when it is first
opened**, because a model has hundreds of rooms and a sheet list has dozens of sheets.

**Not yet run in Revit.**
"""

from room_m import server_config
import os

from System.Windows import Thickness
from System.Windows.Controls import Button, CheckBox, ColumnDefinition, Grid, StackPanel, TextBlock, TextBox
from System.Windows.Controls import ScrollBarVisibility, ScrollViewer
from System.Windows.Controls.Primitives import Popup, PlacementMode
from System.Windows import GridLength, GridUnitType, HorizontalAlignment, VerticalAlignment, Visibility

from room_m import match_rule
from room_m import sheet_export_form as form
from room_m.utils import sheets as utils_sheets

XAML_PATH = os.path.join(os.path.dirname(__file__), "sheet_export.xaml")


def show(forms, model_name, projects, sheets, rooms, saved=None):
    """Show the window. `sheets` and `rooms` are the pick-list entries of `room_m.utils.sheets`; `saved` is what
    `room_m.prefs` kept from the last export (project, filter, room rule).
    Returns `{"project", "assignments": [(sheet entry, None, [room entries])], "filter", "rule"}` for the sheets
    ticked, or None when cancelled."""

    class SheetExportWindow(forms.WPFWindow):
        def __init__(self):
            forms.WPFWindow.__init__(self, XAML_PATH)
            self.result = None
            self._ready = False
            self._projects = list(projects)
            self._rows = []
            self.tx_model.Text = "Model: {}".format(model_name)
            self.tx_server.Text = server_config.short()
            self.cb_project.ItemsSource = [p["name"] for p in self._projects]
            index, text, rule = form.remembered(saved, self._projects)
            if index is not None:
                self.cb_project.SelectedIndex = index
            elif len(self._projects) == 1:
                self.cb_project.SelectedIndex = 0
            self.cb_rule_relation.ItemsSource = [label for _key, label in form.RELATIONS]
            self.cb_rule_relation.SelectedIndex = [k for k, _l in form.RELATIONS].index(rule["relation"]) if rule else 1
            self.tb_rule_sheet.Text = rule["sheet"] if rule else match_rule.DEFAULT_SHEET
            self.tb_rule_room.Text = rule["room"] if rule else match_rule.DEFAULT_ROOM
            for entry in sheets:
                self._add_row(entry)
            self.tb_filter.Text = text
            self._ready = True
            self._apply_filter()
            self._rule_note()
            self._status()

        # -- rows -----------------------------------------------------------------------------

        def _add_row(self, entry):
            row = Grid()
            row.Margin = Thickness(0, 2, 0, 2)
            wide = ColumnDefinition()
            wide.Width = GridLength(1, GridUnitType.Star)
            fixed = ColumnDefinition()
            fixed.Width = GridLength(320)
            row.ColumnDefinitions.Add(wide)
            row.ColumnDefinitions.Add(fixed)

            tick = CheckBox()
            tick.Content = entry["label"]
            tick.VerticalAlignment = VerticalAlignment.Center
            tick.Checked += self.on_change
            tick.Unchecked += self.on_change
            row.Children.Add(tick)

            button = Button()
            button.Content = form.rooms_caption([])
            button.HorizontalContentAlignment = HorizontalAlignment.Left
            Grid.SetColumn(button, 1)
            row.Children.Add(button)

            state = {"entry": entry, "tick": tick, "button": button, "popup": None, "boxes": [], "chosen": [], "row": row}
            button.Click += lambda sender, args, s=state: self._open_rooms(s)
            self.sp_sheets.Children.Add(row)
            self._rows.append(state)

        def _open_rooms(self, state):
            if state["popup"] is None:
                state["popup"] = self._make_popup(state)
            state["popup"].IsOpen = True

        def _make_popup(self, state):
            popup = Popup()
            popup.PlacementTarget = state["button"]
            popup.Placement = PlacementMode.Bottom
            popup.StaysOpen = False
            popup.Width = 320
            holder = StackPanel()
            holder.Background = state["button"].Background
            find = TextBox()
            find.Margin = Thickness(4)
            holder.Children.Add(find)
            scroller = ScrollViewer()
            scroller.MaxHeight = 280
            scroller.VerticalScrollBarVisibility = ScrollBarVisibility.Auto
            inner = StackPanel()
            chosen_ids = set(r["id"] for r in state["chosen"])
            for room in rooms:
                box = CheckBox()
                box.Content = room["label"]
                box.Margin = Thickness(6, 2, 6, 2)
                box.IsChecked = room["id"] in chosen_ids
                box.Checked += lambda sender, args, s=state: self._rooms_changed(s)
                box.Unchecked += lambda sender, args, s=state: self._rooms_changed(s)
                inner.Children.Add(box)
                state["boxes"].append((room, box))
            scroller.Content = inner
            holder.Children.Add(scroller)
            find.TextChanged += lambda sender, args, s=state, f=find: self._filter(s, f.Text)
            popup.Child = holder
            return popup

        def _filter(self, state, text):
            for room, box in state["boxes"]:
                box.Visibility = Visibility.Visible if form.matches(room["label"], text) else Visibility.Collapsed

        def _rooms_changed(self, state):
            """A box in the room list was ticked or unticked: the row's rooms are the ticked ones."""
            if state.get("syncing"):
                return
            self._set_rooms(state, [room for room, box in state["boxes"] if box.IsChecked], from_boxes=True)

        def _set_rooms(self, state, chosen, from_boxes=False):
            """The rooms a sheet covers, kept on the row so they exist before its list was ever opened (the rule
            sets them without it), shown on the button as their numbers, and mirrored into the list if it was."""
            state["chosen"] = list(chosen)
            state["button"].Content = form.rooms_caption(state["chosen"])
            if not from_boxes and state["boxes"]:
                ids = set(r["id"] for r in state["chosen"])
                state["syncing"] = True
                try:
                    for room, box in state["boxes"]:
                        box.IsChecked = room["id"] in ids
                finally:
                    state["syncing"] = False

        @staticmethod
        def _chosen_rooms(state):
            return list(state["chosen"])

        # -- the filter and the rule ---------------------------------------------------------

        def _listed(self):
            """The rows the filter lists."""
            return [s for s in self._rows if s["row"].Visibility == Visibility.Visible]

        def _apply_filter(self):
            text = self.tb_filter.Text
            for s in self._rows:
                s["row"].Visibility = Visibility.Visible if form.filter_matches(s["entry"]["label"], text) else Visibility.Collapsed
            self.tx_sheets.Text = "Sheets ({} of {} listed)".format(len(self._listed()), len(self._rows))

        def _rule(self):
            index = self.cb_rule_relation.SelectedIndex
            return form.make_rule(
                self.tb_rule_sheet.Text, form.RELATIONS[index][0] if index >= 0 else None, self.tb_rule_room.Text)

        def _rule_note(self, extra=""):
            self.tx_rule.Text = (form.describe_rule(self._rule()) + " " + extra).strip()

        # -- handlers, named in the XAML ------------------------------------------------------

        def on_change(self, sender, args):
            self._status()

        def on_filter(self, sender, args):
            if self._ready:
                self._apply_filter()

        def on_check_matched(self, sender, args):
            """Tick the listed sheets that have a room (from the rule or by hand). A listed sheet with none is left as
            it is and counted, so a rule that matched nothing is seen and not silently exported without rooms."""
            ticked = skipped = 0
            for s in self._listed():
                if s["chosen"]:
                    s["tick"].IsChecked = True
                    ticked += 1
                else:
                    skipped += 1
            self._rule_note("Checked {} sheet(s) that have rooms; {} listed sheet(s) have no room and were left.".format(
                ticked, skipped))

        def on_check_none(self, sender, args):
            for s in self._rows:
                s["tick"].IsChecked = False

        def on_rule_change(self, sender, args):
            if self._ready:
                self._rule_note()

        def on_apply_rule(self, sender, args):
            """Set the rooms of every LISTED sheet from the rule. A sheet the rule pairs with no room keeps what it
            had, so a choice made by hand is never wiped by a rule that does not cover it."""
            rule = self._rule()
            sheets_listed = utils_sheets.ensure_params([s["entry"] for s in self._listed()])
            utils_sheets.ensure_params(rooms)
            paired = unpaired = 0
            for s in self._listed():
                found = form.rooms_for_sheet(rule, s["entry"], rooms)
                if found:
                    self._set_rooms(s, found)
                    paired += 1
                else:
                    unpaired += 1
            self._rule_note(" ".join(
                ["{} sheet(s) got rooms; {} had no room that matches.".format(paired, unpaired)]
                + match_rule.unknown_parameters(rule, sheets_listed, rooms)))

        def on_cancel(self, sender, args):
            self.result = None
            self.Close()

        def on_export(self, sender, args):
            if self._problems():
                self._status()
                return
            self.result = {
                "project": self._projects[self.cb_project.SelectedIndex],
                "assignments": [
                    (s["entry"], None, self._chosen_rooms(s)) for s in self._rows if s["tick"].IsChecked
                ],
                "filter": self.tb_filter.Text,
                "rule": self._rule(),
            }
            self.Close()

        # -- helpers --------------------------------------------------------------------------

        def _problems(self):
            project = self._projects[self.cb_project.SelectedIndex] if self.cb_project.SelectedIndex >= 0 else None
            return form.problems(project, sum(1 for s in self._rows if s["tick"].IsChecked))

        def _status(self):
            if not self._ready:
                return
            lines = self._problems()
            self.tx_status.Text = "\n".join(lines)
            self.bt_export.IsEnabled = not lines

    window = SheetExportWindow()
    window.ShowDialog()
    return window.result
