"""The RLS settings in a Revit window: a XAML form, no web page and no server.

ASCII only, and no em-dashes: IronPython 2.7 will not parse a file containing one.

**A proof of concept, tried again on purpose.** An earlier attempt at a WPF dialog from IronPython crashed
Revit with a large grid and fought the language over class names. This one is made to avoid both: the
window has **no x:Class and no DataGrid** (only a ScrollViewer, TextBlocks, ComboBoxes and TextBoxes), it
is loaded from a XAML FILE by pyRevit's `forms.WPFWindow`, its handlers are plain methods named in the
XAML, and everything that is not drawing lives in `room_m.rls_settings_form` (tested without Revit). If it
works, it has one advantage over the web page: it needs no server and no browser, and returns its answer
straight to the calling function (`window.settings`, or None when cancelled).

**Not yet run in Revit.**
"""

import os

from room_m import rls_settings
from room_m import rls_settings_form as form

XAML_PATH = os.path.join(os.path.dirname(__file__), "rls_settings.xaml")


def show(forms, model_name, settings, options):
    """Show the window and return the settings the person saved, or None when they cancelled."""

    class RlsSettingsWindow(forms.WPFWindow):
        def __init__(self):
            forms.WPFWindow.__init__(self, XAML_PATH)
            self.settings = None
            self._existing = dict(settings or {})
            self._values = {}
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
            self._status()

        # -- handlers, named in the XAML ------------------------------------------------------

        def on_token_sheet_number(self, sender, args):
            self._insert(sender, self.tb_sheet_number)

        def on_token_sheet_name(self, sender, args):
            self._insert(sender, self.tb_sheet_name)

        def on_token_view_name(self, sender, args):
            self._insert(sender, self.tb_view_name)

        def on_save(self, sender, args):
            self.settings = form.to_settings(
                self._existing,
                dict((name, self._values[name][getattr(self, name).SelectedIndex]) for name in self._values),
                dict((name, getattr(self, name).Text or "") for name, _key in form.TEXTS),
            )
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

        def _status(self):
            self.tx_status.Text = ""

    window = RlsSettingsWindow()
    window.ShowDialog()
    return window.settings


def missing_after(settings):
    """What still stops these settings being used, for the line after saving."""
    return rls_settings.missing(settings)
