This folder is filled by the build, and is empty in the repository on purpose.

Two packages land here:

  room_m    copied from extractor\pyRevit\room_m, which stays the one source
            for the extractor. It is not kept here as well, because two copies
            of it is the exact failure this move was made to end.

  duHast    fetched at the commit named in extractor\duhast.lock, and stamped
            with that commit. It is not committed to this repository: duHast
            has its own, and a copy here would fork it silently.

So a checkout of this repository is NOT a loadable pyRevit extension. Build it
with installer\build.ps1, whose -Dev mode assembles the extension from the
working tree and writes it where pyRevit looks for it.

Why ship duHast at all, rather than use whichever one the machine has: duHast
decides what an export MEANS -- an item's level, a door's footprint, whether a
hole in a floor is a hole. A copy of unknown age answers those differently, and
the export still looks successful. Every RoomMate run prints the duHast it
loaded and where it came from, for the same reason.
