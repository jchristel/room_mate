"""The export window: every export in one place, drawn from `export.xaml`.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The same approach as the RLS settings window (`rls_settings_window`), for the same reasons: no x:Class, no
DataGrid, handlers are plain methods named in the XAML, everything that is not drawing is in
`room_m.export_form` (tested without Revit). The level list is check boxes made in code inside a
StackPanel, so the number of levels needs no grid.

**Not yet run in Revit.**
"""

import os

from System.Windows import Thickness
from System.Windows.Controls import CheckBox

from room_m import export_form as form

XAML_PATH = os.path.join(os.path.dirname(__file__), "export.xaml")


def show(forms, docs, projects, phases, levels_by_doc):
    """Show the window. Returns `{"project": {"id", "name"}, "phase", "entities", "levels"}` (`levels` is
    None for the whole model, else the chosen level names), or None when cancelled."""
    names = form.level_names(levels_by_doc)

    class ExportWindow(forms.WPFWindow):
        def __init__(self):
            forms.WPFWindow.__init__(self, XAML_PATH)
            self.result = None
            self._ready = False
            self._projects = list(projects)
            self._level_boxes = []
            self.tx_models.Text = "Models: {}".format(", ".join(d.Title for d in docs))
            self.cb_project.ItemsSource = [p["name"] for p in self._projects]
            self.cb_phase.ItemsSource = list(phases)
            if len(self._projects) == 1:
                self.cb_project.SelectedIndex = 0
            if phases:
                self.cb_phase.SelectedIndex = 0
            for name in names:
                box = CheckBox()
                box.Content = name
                box.Margin = Thickness(0, 2, 0, 2)
                box.Checked += self.on_change
                box.Unchecked += self.on_change
                self.sp_levels.Children.Add(box)
                self._level_boxes.append(box)
            self._ready = True
            self._status()

        # -- handlers, named in the XAML ------------------------------------------------------

        def on_change(self, sender, args):
            self._status()

        def on_scope(self, sender, args):
            if not self._ready:
                return
            self.sp_levels.IsEnabled = bool(self.rb_levels.IsChecked)
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
                "phase": self.cb_phase.SelectedItem,
                "entities": form.entities_ticked(self._ticked()),
                "levels": None if self.rb_whole.IsChecked else self._chosen_levels(),
            }
            self.Close()

        # -- helpers --------------------------------------------------------------------------

        def _ticked(self):
            return set(name for name, _key, _label in form.ENTITIES if getattr(self, name).IsChecked)

        def _chosen_levels(self):
            return [str(b.Content) for b in self._level_boxes if b.IsChecked]

        def _problems(self):
            project = self._projects[self.cb_project.SelectedIndex] if self.cb_project.SelectedIndex >= 0 else None
            phase = self.cb_phase.SelectedItem
            return form.problems(
                self._ticked(), project, phase, bool(self.rb_whole.IsChecked), self._chosen_levels())

        def _status(self):
            if not self._ready:
                return
            lines = self._problems()
            self.tx_status.Text = "\n".join(lines)
            self.bt_export.IsEnabled = not lines

    window = ExportWindow()
    window.ShowDialog()
    return window.result
