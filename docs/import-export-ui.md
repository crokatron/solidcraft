# Import / Export

The Import / Export ribbon tab groups supported imports, exact CAD export, mesh export and drawing/data export. File also offers Parasolid and NX Assembly (STEP).

**Parasolid** exports a complete design with component occurrences as an XT assembly. Exporting an explicit body subset through the command instead produces separate placed bodies. See [Parasolid export](parasolid-export.md) for supported geometry and current interoperability limits.

**NX Assembly (STEP)** writes the complete named STEP assembly and its placements. It does not create native NX `.prt` files. NX's STEP translator must preserve assembly structure instead of flattening it; the receiving NX session creates its own parts.

Successful file exports keep their notification visible until dismissed. **Copy folder path** copies the destination directory. The app-bar **Notifications** button opens this session's message history; dismissing a toast retains its history entry. **Dismiss all** hides pending notices; **Clear** deletes the message history. History is currently limited to 200 entries and is not retained after quitting.
