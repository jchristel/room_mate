"""The blueprint export window: the rooms of a phase, each with the blueprint's name and its sheets.
Drawn from `blueprint_export.xaml`.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The same approach as the other windows (`sheet_export_window`, `export_window`): no x:Class, no
DataGrid or ListView, handlers are plain methods named in the XAML, rules in
`room_m.blueprint_export_form`. The rows are a StackPanel of small Grids made in code **when the phase
is chosen** (the rooms depend on it), and a room's sheet list is a Popup of check boxes **made when it
is first opened**. A sheet ticked for one room is greyed in the others, because a sheet is offered once.

**Not yet run in Revit.**
"""

import os

from System.Windows import Thickness
from System.Windows.Controls import Button, CheckBox, ColumnDefinition, Grid, StackPanel, TextBox
from System.Windows.Controls import ScrollBarVisibility, ScrollViewer
from System.Windows.Controls.Primitives import Popup, PlacementMode
from System.Windows import GridLength, GridUnitType, HorizontalAlignment, VerticalAlignment, Visibility

from room_m import blueprint_export_form as form

XAML_PATH = os.path.join(os.path.dirname(__file__), "blueprint_export.xaml")


def _column(width):
    column = ColumnDefinition()
    column.Width = GridLength(1, GridUnitType.Star) if width is None else GridLength(width)
    return column


def show(forms, model_name, projects, phases, rooms_for, sheets):
    """Show the window. `phases` are phase names, `rooms_for(phase)` the pick-list entries of the
    placed rooms of that phase and `sheets` those of the model's sheets (`room_m.utils.sheets`).
    Returns `{"project", "phase", "plan": [(room entry, name, [sheet entries])]}` for the rooms
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
            self.cb_project.ItemsSource = [p["name"] for p in self._projects]
            if len(self._projects) == 1:
                self.cb_project.SelectedIndex = 0
            self.cb_phase.ItemsSource = self._phases
            if len(self._phases) == 1:
                self.cb_phase.SelectedIndex = 0
            self._ready = True
            self._fill_rooms()
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
            button.Content = form.sheets_caption(0)
            button.Margin = Thickness(6, 0, 0, 0)
            button.HorizontalContentAlignment = HorizontalAlignment.Left
            Grid.SetColumn(button, 2)
            row.Children.Add(button)

            state = {"entry": entry, "row": row, "tick": tick, "name": name, "button": button, "popup": None, "boxes": []}
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
            for sheet in sheets:
                box = CheckBox()
                box.Content = sheet["label"]
                box.Margin = Thickness(6, 2, 6, 2)
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
            return [sheet for sheet, box in state["boxes"] if box.IsChecked]

        def _sheets_changed(self, state):
            state["button"].Content = form.sheets_caption(len(self._chosen_sheets(state)))
            self._grey_taken()
            self._status()

        def _grey_taken(self):
            """A sheet ticked for one room cannot be ticked for another."""
            held = [(s["entry"]["id"], [sh["id"] for sh in self._chosen_sheets(s)]) for s in self._rows]
            for state in self._rows:
                taken = form.taken_elsewhere(held, state["entry"]["id"])
                for sheet, box in state["boxes"]:
                    box.IsEnabled = sheet["id"] not in taken

        def _filter(self, text):
            for state in self._rows:
                visible = form.matches(state["entry"]["label"], text)
                state["row"].Visibility = Visibility.Visible if visible else Visibility.Collapsed

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
            self._status()

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
            }
            self.Close()

        # -- helpers --------------------------------------------------------------------------

        def _problems(self):
            ticked = [
                (s["entry"]["label"], s["name"].Text, len(self._chosen_sheets(s))) for s in self._ticked()
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
