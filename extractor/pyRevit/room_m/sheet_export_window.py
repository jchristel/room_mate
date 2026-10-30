"""The sheet export window: sheets in one column, the rooms each covers in the other. Drawn from
`sheet_export.xaml`.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The same approach as the other windows (`export_window`, `rls_settings_window`): no x:Class, no DataGrid or
ListView, handlers are plain methods named in the XAML, rules in `room_m.sheet_export_form`. The rows are a
StackPanel of small Grids made in code. A room list is a Popup of check boxes **made when it is first
opened**, because a model has hundreds of rooms and a sheet list has dozens of sheets.

**Not yet run in Revit.**
"""

import os

from System.Windows import Thickness
from System.Windows.Controls import Button, CheckBox, ColumnDefinition, Grid, StackPanel, TextBlock, TextBox
from System.Windows.Controls import ScrollBarVisibility, ScrollViewer
from System.Windows.Controls.Primitives import Popup, PlacementMode
from System.Windows import GridLength, GridUnitType, HorizontalAlignment, VerticalAlignment, Visibility

from room_m import sheet_export_form as form

XAML_PATH = os.path.join(os.path.dirname(__file__), "sheet_export.xaml")


def show(forms, model_name, projects, sheets, rooms):
    """Show the window. `sheets` and `rooms` are the pick-list entries of `room_m.utils.sheets`.
    Returns `{"project", "assignments": [(sheet entry, None, [room entries])]}` for the sheets ticked, or
    None when cancelled."""

    class SheetExportWindow(forms.WPFWindow):
        def __init__(self):
            forms.WPFWindow.__init__(self, XAML_PATH)
            self.result = None
            self._ready = False
            self._projects = list(projects)
            self._rows = []
            self.tx_model.Text = "Model: {}".format(model_name)
            self.cb_project.ItemsSource = [p["name"] for p in self._projects]
            if len(self._projects) == 1:
                self.cb_project.SelectedIndex = 0
            for entry in sheets:
                self._add_row(entry)
            self._ready = True
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
            button.Content = form.rooms_caption(0)
            button.HorizontalContentAlignment = HorizontalAlignment.Left
            Grid.SetColumn(button, 1)
            row.Children.Add(button)

            state = {"entry": entry, "tick": tick, "button": button, "popup": None, "boxes": []}
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
            for room in rooms:
                box = CheckBox()
                box.Content = room["label"]
                box.Margin = Thickness(6, 2, 6, 2)
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
            state["button"].Content = form.rooms_caption(len(self._chosen_rooms(state)))

        @staticmethod
        def _chosen_rooms(state):
            return [room for room, box in state["boxes"] if box.IsChecked]

        # -- handlers, named in the XAML ------------------------------------------------------

        def on_change(self, sender, args):
            self._status()

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
