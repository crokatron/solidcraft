# NX shortcuts and SpaceMouse buttons

Preferences > Shortcuts > Import accepts user-authored NX `.mtx` accelerator files and SolidCraft JSON shortcut profiles. The NX Shortcut Workbench also provides **Export for SolidCraft**, exporting its current edited assignments as a shortcut-only `.mtx`.

Expand **NX shortcut import report**, review the mappings, then use **Apply supported shortcuts**. Existing bindings are retained unless **Replace conflicting SolidCraft shortcuts** is checked. Restore previous shortcuts reverses this import during the current session. Export a SolidCraft profile before restarting if you need a permanent backup. Applied shortcuts persist with normal preferences; Save report preserves the full mapping audit.

An MTX contains explicit custom accelerators, not necessarily every factory NX shortcut. Unsupported actions and suppressed keys remain in the report. NX suppression does not disable unrelated SolidCraft keys. Sketch bindings run in sketch context. Linear, radial and angular dimension bindings all open SolidCraft's selection-driven dimension tool; the report identifies this difference.

Preferences > 3D mouse > Buttons configures each detected device independently. Use **Add button mapping** to create numbered rows without pressing the device, or tap a physical button to identify its row. Choose a command, standard view, supported modifier or Nothing. Disabled rows persist. Commands respect the current tool/context; holding a command button does not repeatedly invoke it. Six-axis navigation settings remain separate from button assignments. This does not modify vendor-driver profiles.

Automation can review/apply the same import with `ui.shortcuts_import {text?, source?, apply?, replace?}`. Omit text to inspect the current report. Errors leave prior bindings intact when parsing fails; per-binding conflicts remain in the report.
