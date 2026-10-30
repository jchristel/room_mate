"""The RLS settings in a Revit window: a XAML form, no web page and no server.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**Made to avoid what crashed an earlier WPF dialog** (a large grid, and a fight over class names): the
window has **no x:Class and no DataGrid** (only a ScrollViewer, TextBlocks, ComboBoxes and TextBoxes), it
is loaded from a XAML FILE by pyRevit's `forms.WPFWindow`, its handlers are plain methods named in the
XAML, and everything that is not drawing lives in `room_m.rls_settings_form` (tested without Revit). It
needs no server and no browser and returns its answer straight to the calling function (`window.settings`,
or None when cancelled). A status line under the form says what is still to be chosen and which template
words are not room parameters, as the person changes things.

The window was tried in Revit and opens and saves.
"""

import os

from room_m import rls_settings_form as form

XAML_PATH = os.path.join(os.path.dirname(__file__), "rls_settings.xaml")


def show(forms, model_name, settings, options):
    """Show the window and return the settings the person saved, or None when they cancelled."""

    class RlsSettingsWindow(forms.WPFWindow):
        def __init__(self):
            forms.WPFWindow.__init__(self, XAML_PATH)
            self.settings = None
            self._ready = False
            self._existing = dict(settings or {})
            self._values = {}
            self._errors = list((options or {}).get("errors") or [])
            self._parameters = list((options or {}).get("room_parameters") or [])
            self.tx_model.Text = "Model: {}".format(model_name)
            for name, option_key, path in form.COMBOS:
                labels, values, selected = form.choices(form.get_path(self._existing, path), (options or {}).get(option_key))
                self._values[name] = values
                combo = getattr(self, name)
                combo.ItemsSource = labels
                combo.SelectedIndex = selected
            for name, key in form.TEXTS:
                getattr(self, name).Text = self._existing.get(key) or ""
                token_combo = getattr(self, "tk_" + name[3:])
                token_combo.ItemsSource = form.token_labels(self._parameters)
                token_combo.SelectedIndex = 0
            self._ready = True
            self._status()

        # -- handlers, named in the XAML ------------------------------------------------------

        def on_token_sheet_number(self, sender, args):
            self._insert(sender, self.tb_sheet_number)

        def on_token_sheet_name(self, sender, args):
            self._insert(sender, self.tb_sheet_name)

        def on_token_view_name(self, sender, args):
            self._insert(sender, self.tb_view_name)

        def on_change(self, sender, args):
            self._status()

        def on_save(self, sender, args):
            self.settings = self._current()
            self.Close()

        def on_cancel(self, sender, args):
            self.settings = None
            self.Close()

        # -- helpers --------------------------------------------------------------------------

        def _insert(self, combo, box):
            index = combo.SelectedIndex
            if index < 1:
                return
            box.Text = form.add_token(box.Text, self._parameters, index)
            combo.SelectedIndex = 0
            self._status()

        def _current(self):
            """The settings as the form stands now."""
            picked = {}
            for name in self._values:
                index = getattr(self, name).SelectedIndex
                if index >= 0:
                    picked[name] = self._values[name][index]
            return form.to_settings(
                self._existing,
                picked,
                dict((name, getattr(self, name).Text or "") for name, _key in form.TEXTS),
            )

        def _status(self):
            """What is still missing, and any template word that is not a room parameter."""
            if not self._ready:
                return
            lines = form.problems(self._current(), self._parameters)
            lines.extend("Revit could not list {}".format(e) for e in self._errors)
            self.tx_status.Text = "\n".join(lines) if lines else "Complete: a room layout sheet can be made."

    window = RlsSettingsWindow()
    window.ShowDialog()
    return window.settings

