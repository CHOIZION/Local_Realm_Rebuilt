# Built-in brushes

Files dropped here ship with Local Realm and appear under **Built-in** in the
map maker's brush picker, in every world.

- `.abr` — Photoshop brush packs (v1, v2, v6, v7, v10). Each sampled brush in
  the pack becomes a picture brush.
- `.png` — paints where the picture is opaque; partial transparency is honoured.
- `.jpg` / `.jpeg` — paints where the picture is dark.

Names are shown as they are here, without the extension, so name files the way
users should see them ("Mountain ranges.abr", "Coastline.png").

Anything else in this folder (this README included) is ignored by the picker.
After adding or changing files, rebuild the app: the folder is copied next to
the executable, into the installers, and so into the Steam depot.
