# License:
#
#
# Revit Batch Processor Sample Code
#
# BSD License
# Copyright 2026, Jan Christel
# All rights reserved.

# Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

# - Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
# - Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
# - Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.
#
# This software is provided by the copyright holder "as is" and any express or implied warranties, including, but not limited to, the implied warranties of merchantability and fitness for a particular purpose are disclaimed.
# In no event shall the copyright holder be liable for any direct, indirect, incidental, special, exemplary, or consequential damages (including, but not limited to, procurement of substitute goods or services; loss of use, data, or profits;
# or business interruption) however caused and on any theory of liability, whether in contract, strict liability, or tort (including negligence or otherwise) arising in any way out of the use of this software, even if advised of the possibility of such damage.
#
#
#

"""One model's contribution to a run's CEILINGS bucket, and the push for the
whole bucket: `room_m.exporters.surfaces` bound to ceilings.

The sixth entity. Every rule that governs it -- the range phase test, the
count guard, the empty-run refusal, nothing re-derived -- is shared with floors
and stated in `exporters.surfaces`.
"""

from Autodesk.Revit.DB import BuiltInCategory, BuiltInParameter

from duHast.Revit.Ceilings.Export.to_data_ceiling import get_all_ceiling_data
from duHast.Revit.Ceilings.ceilings import (
    get_all_ceiling_instances_in_model_by_category,
)
from duHast.Data.Objects.Collectors import data_ceiling as dc

from room_m.exporters import surfaces
from room_m.post_ceilings import (
    post_payload_stream,
)


SPEC = surfaces.SurfaceExport(
    plural="ceilings",
    singular="ceiling",
    category=BuiltInCategory.OST_Ceilings,
    collect=get_all_ceiling_instances_in_model_by_category,
    export=get_all_ceiling_data,
    data_type=dc.DataCeiling.data_type,
    offset_parameter=BuiltInParameter.CEILING_HEIGHTABOVELEVEL_PARAM,
    post_stream=post_payload_stream,
)


# A ceiling has no boundary regime and no other per-model fact to stamp beyond
# what the shared identity envelope already carries, so this entity contributes
# nothing extra to its model block -- the same `None` doors, windows and FFE
# use, rather than a no-op function.
stamp_envelope = None


def export_model(selected_doc, phase_name, return_value, level_ids=None):
    """See `exporters.surfaces.export_model`."""
    return surfaces.export_model(SPEC, selected_doc, phase_name, return_value, level_ids=level_ids)


def post_bucket(run_envelope, entries, return_value):
    """See `exporters.surfaces.post_bucket`."""
    return surfaces.post_bucket(SPEC, run_envelope, entries, return_value)
