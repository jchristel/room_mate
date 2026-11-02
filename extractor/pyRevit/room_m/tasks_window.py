"""The task chooser: the open model's queued tasks as a check box list, drawn from `tasks.xaml`.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

The same approach as the export and RLS settings windows: no x:Class, no DataGrid or ListView (a check box per
task in a StackPanel), handlers are plain methods named in the XAML, and every rule that is not drawing is in
`room_m.tasks_form` (tested without Revit). Filtering hides check boxes and never removes them, so a tick
survives a filter change.

**Not yet run in Revit.**
"""

import os

from System.Windows import Thickness, Visibility
from System.Windows.Controls import CheckBox

from room_m import tasks_form as form

XAML_PATH = os.path.join(os.path.dirname(__file__), "tasks.xaml")


def show(forms, model_title, projects, load):
    """Show the window. `load(project)` answers `(ok, rows_or_message)` for a project (`tasks_form.rows`).
    Returns `{"project": {"id", "name"}, "task_ids": [...]}` for the ticked tasks, or None when cancelled."""

    class TasksWindow(forms.WPFWindow):
        def __init__(self):
            forms.WPFWindow.__init__(self, XAML_PATH)
            self.result = None
            self._ready = False
            self._projects = list(projects)
            self._rows = []
            self._boxes = []
            self._message = ""
            self.tx_model.Text = "Model: {}".format(model_title)
            self.cb_project.ItemsSource = [p["name"] for p in self._projects]
            self.cb_kind.ItemsSource = [label for _key, label in form.KIND_FILTERS]
            self.cb_kind.SelectedIndex = 0
            self.cb_level.ItemsSource = [form.ALL_LEVELS]
            self.cb_level.SelectedIndex = 0
            self._ready = True
            if len(self._projects) == 1:
                self.cb_project.SelectedIndex = 0
            self._status()

        # -- handlers, named in the XAML ------------------------------------------------------

        def on_project(self, sender, args):
            if not self._ready or self.cb_project.SelectedIndex < 0:
                return
            self._load()

        def on_filter(self, sender, args):
            if self._ready:
                self._apply_filter()

        def on_tick_shown(self, sender, args):
            self._set_shown(True)

        def on_untick_shown(self, sender, args):
            self._set_shown(False)

        def on_cancel(self, sender, args):
            self.result = None
            self.Close()

        def on_run(self, sender, args):
            if form.problems(len(self._ticked_ids())):
                self._status()
                return
            self.result = {"project": self._projects[self.cb_project.SelectedIndex], "task_ids": self._ticked_ids()}
            self.Close()

        # -- helpers --------------------------------------------------------------------------

        def _load(self):
            ok, answer = load(self._projects[self.cb_project.SelectedIndex])
            self._rows = answer if ok else []
            self._message = "" if ok else str(answer)
            self.sp_tasks.Children.Clear()
            self._boxes = []
            for row in self._rows:
                box = CheckBox()
                box.Content = row["text"]
                box.Margin = Thickness(0, 2, 0, 2)
                box.Checked += self.on_filter
                box.Unchecked += self.on_filter
                self.sp_tasks.Children.Add(box)
                self._boxes.append(box)
            self._ready = False
            self.cb_level.ItemsSource = form.level_choices(self._rows)
            self.cb_level.SelectedIndex = 0
            self._ready = True
            self._apply_filter()

        def _filters(self):
            kind_key = form.KIND_FILTERS[max(self.cb_kind.SelectedIndex, 0)][0]
            level = self.cb_level.SelectedItem
            return self.tb_filter.Text, kind_key, level

        def _is_shown(self, row):
            text, kind_key, level = self._filters()
            return form.shown(row, text, kind_key, level)

        def _apply_filter(self):
            for row, box in zip(self._rows, self._boxes):
                box.Visibility = Visibility.Visible if self._is_shown(row) else Visibility.Collapsed
            self._status()

        def _set_shown(self, value):
            for row, box in zip(self._rows, self._boxes):
                if self._is_shown(row):
                    box.IsChecked = value

        def _ticked_ids(self):
            return [row["id"] for row, box in zip(self._rows, self._boxes) if box.IsChecked]

        def _status(self):
            if not self._ready:
                return
            ticked = len(self._ticked_ids())
            shown = len([r for r in self._rows if self._is_shown(r)])
            lines = [self._message] if self._message else []
            if self.cb_project.SelectedIndex < 0:
                lines.append("Choose a project.")
            elif not self._rows and not self._message:
                lines.append("No queued tasks for this model in this project.")
            else:
                lines.append(form.summary(ticked, shown, len(self._rows)))
            self.tx_status.Text = "\n".join(lines)
            self.bt_run.IsEnabled = ticked > 0

    window = TasksWindow()
    window.ShowDialog()
    return window.result
