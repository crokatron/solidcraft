# Development fork changelog

This records changes in the `crokatron` development fork relative to upstream SolidCraft. It is not the upstream release history. Upstream authorship, licences and notices remain in place. This fork is not endorsed by Tangency Labs or by the CAD vendors named in compatibility documentation.

The entries below describe experimental implementations in the working source. They are not a promise of production readiness, commercial CAD feature parity or a passed full release qualification. See [feature requests](FEATURE_REQUESTS.md) for outstanding acceptance criteria.

## Unreleased — 2026-11-17

Base: upstream SolidCraft 0.40.1, commit `0c529228`. This date groups the current development work; it is not a public binary release date.

### Move bodies and components

- Shared editable handle coordinate system, **Move handles only**, reference CSYS selection and placement alignment.
- Automatic hover targets for vertices, exact edge midpoints, circle centres, cylindrical/conical face-axis centres and true 3D virtual intersections of straight edges.
- Orange inferred markers with dark outlines, retained hover targets and explicit snap modes.
- Axis clicks arm alignment to edge directions or face normals; dragging translates and double-clicking reverses an axis.
- Compact handle origin/orientation dock and editable distance/angle beside the selected manipulator. Manual values remain accessible.
- **OK** confirms and closes; **Apply** confirms and keeps the move tool open; middle-click applies while middle-drag remains navigation.

Alignment records a transform at the current reference pose. Associative geometry alignment and **Move Parents** are not implemented. See [move handles](docs/move-handles.md).

### Shortcuts and input devices

- Import supported user-authored NX `.mtx` accelerator assignments with an explicit mapping, conflict and unsupported-action report.
- Preserve unrelated bindings by default; optionally replace conflicts. Applied bindings persist in preferences.
- Per-device SpaceMouse button mappings, including adding numbered rows, discovering tapped buttons and retaining disabled mappings.

MTX import does not imply support for every NX command or every factory shortcut. Button configuration does not modify vendor-driver profiles. Personal shortcut files and settings are not part of the fork. See [shortcut documentation](docs/nx-shortcuts.md).

### Scan editing and reconstruction — experimental

- Mesh editing workspace with lasso delete/isolate, direct vertex/edge/face operations, region selection, automatic region detection, cleanup, island removal, smoothing and topology inspection.
- Plane trimming controls, scan alignment to planes/datums, landmark alignment and scan mirror-plane tools.
- Texture/colour data carried through supported mesh edits using per-corner UVs, colours and material assignments. Global remeshing and decimation transfer these attributes and retain source image materials; protected boundaries can prevent reaching a requested reduction target.
- Copies of selected scan patches and point clouds, live slices, section sketches and automatic section-profile tools.
- Guided reverse-engineering workspace: preserve and clean, establish datums, inspect regions, create profiles, reconstruct surfaces, then check and deliver. This groups tools; it does not automatically certify completion of each stage.
- Bounded plane, cylinder, cone and freeform surface fitting with retained trim loops and measured sample residuals. The current freeform fitter uses a bicubic height-graph patch, not a general automatic automotive patch network.
- Neighbouring surface-patch matching includes G1/G2 construction and diagnostic reports for supported patch configurations.

CAD-to-scan deviation is not yet available in the guided workspace. Measured fit residuals and continuity diagnostics are not continuous error certificates. Performance targets and broader texture/seam qualification remain open.

### 3D sketching — experimental

- Spatial coincident/tangent constraints, driving dimensions and spatial freedom analysis with viewport controls and reports.
- Axis guidance and sketch interaction updates, plus 3D line-fillet tooling.

Freedom is local/instantaneous, not proof of unrestricted finite motion. Current analysis is bounded to 96 editable fit points and can report unknown or conflicting states.

### Import, export and notifications — experimental

- STEP and Parasolid export now ask which CAD bodies to write, using the current selection or visible CAD bodies by default. Reference scan meshes are excluded from this panel.
- Parasolid export preserves internal cavities through separate shell sides and bounded void regions. Regression checks cover one and two cavities; the selected PSU enclosure also reimports as a closed solid with two shells and unchanged volume.
- Dedicated Import / Export ribbon and persistent export notifications with copy-folder-path action and session notification history.
- Independent readers for supported NX `.prt` and Parasolid `.x_t`/`.x_b` representations; no installed NX is required. Unsupported/incomplete geometry is refused, and native source feature history is not reconstructed.
- Parasolid text export for supported closed CAD solids, including assembly/instance records and component grouping. Explicit body-subset export produces independent placed bodies.
- Named STEP assembly exchange, presented separately from native NX export.

Native NX `.prt` writing and binary Parasolid `.x_b` writing are not implemented. Parasolid export does not support every surface/topology, transmit feature history or preserve shared-definition associativity; commercial interoperability is not certified. See [Parasolid limitations](docs/parasolid-export.md), [NX export research](docs/native-nx-export-research.md) and [Import / Export](docs/import-export-ui.md).

### Validation and release status

Focused local checks have been reported for move snapping, manipulator values/layout, shortcut import and SpaceMouse bindings. These do not establish that the complete modified tree passes upstream CI. A release must record its exact commit, checks, platform and remaining failures.

The 2026-11-17 fork-preparation run of `cargo xtask ci` stopped at its first `cargo fmt --check` step with Windows error 206 (filename or extension too long). The remaining CI gates did not run. This development snapshot must not be described as CI-passing.

No permission to redistribute a modified branded application is asserted here. The [Brand and Interface Licence](LICENSE-BRAND) remains applicable; any future downloadable build needs a separate distribution review. Keep personal designs, recordings, credentials, recovery copies and preferences out of source commits and release archives.

## Maintaining this file

Add user-visible changes under **Unreleased**. Link each change to its issue/commit when those exist. Record breaking changes, data-format effects and known limitations. At a qualified release, move the reviewed entries under the actual tag/date and retain a new Unreleased section; do not relabel unfinished requests as completed changes.
