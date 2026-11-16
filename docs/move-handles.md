# Position the move handles before moving a part

Body Move/Copy (Free Move or Translate) and Move Component share an editable handle coordinate system.

1. Select the body or component and open its Move tool.
2. Turn on **Move handles only**. Drag the arrows, centre or rotation rings to place the handles without moving the geometry. You can also enter the handle origin and orientation, or choose **Handle CSYS** and press **Set handles from CSYS**.
3. Turn **Move handles only** off. The geometry stays where it is; subsequent translation and rotation use the handle frame.
4. To place that frame on another coordinate system, choose **Target CSYS** and press **Align handles and part to CSYS**. This matches the origin and all three axes, including roll.
5. **OK** confirms and closes. **Apply** confirms and keeps the tool open. **Cancel** discards the pending move while retaining earlier applied moves. Middle-click applies; middle-drag remains navigation.

References include World CSYS, component occurrence origins, construction-plane frames and named joint origins. Repeated component placements have separate reference entries. Missing references must be selected again; they never silently become World CSYS.

Handle placement is a draft tool setting, not a change to the part's authored coordinate system. Alignment records a move at the selected frame's current pose; it does not create an associative mate or a new datum. Existing grounding and assembly restrictions still apply. Component Apply captures only that component's pending placement.

The public interaction reference was the [Siemens discussion of Move Handles Only](https://community.sw.siemens.com/s/question/0D54O000061x09oSAA/copypaste-sketch-lines-to-other-models-sketch-). The implementation uses SolidCraft's own commands, geometry and controls.

## Snap handles to geometry

Click the handle centre (or **Snap origin**) to pick an origin on a vertex, edge or face. **Auto** prefers nearby vertices, half-length edge midpoints and circle centres; **On geometry**, **Edge midpoint** and **Circle / face center** let you choose explicitly. Selecting another snap type starts the picker immediately. Cylinder and cone face centres lie on the surface axis halfway between the face’s axial limits; circular edges use their circle centre. Midpoints use half the exact curve length, including spline edges. A marker and label identify the candidate before you click.

Click an axis arrow (or the X/Y/Z snap control), then choose an edge tangent or face normal to orient that axis. **Reverse axis** flips the selected axis. Axis alignment keeps the origin fixed and uses the shortest rotation; aligning a full CSYS also defines roll. A click arms snapping; dragging an arrow continues to move along it.

For **Virtual intersection**, pick two straight edges. The first pick stays marked, and preview lines extend both selected edges to the proposed corner. Their extended lines must intersect in 3D: parallel or skew lines are rejected. Projected screen crossings do not count as intersections.

With **Move handles only** checked, snaps change only the handle frame. Uncheck it to translate or rotate the body/component with the same snap workflow. Pending body and component transforms are included when resolving geometry.

### Hover inference

Automatic mode reveals clickable vertex, edge-midpoint, circle-centre and cylindrical/conical face-axis-centre markers while hovering. Markers remain visible after leaving their source geometry, including when their point is in empty space. Hover over two straight edges to infer their true 3D intersection and display the supporting extensions; click the resulting virtual-corner marker. Hovering does not move geometry. Click a marker to position the origin, or use Align X/Y/Z to select its direction. Ordinary clicks away from markers retain their existing behavior unless Pick origin is armed.

The most recent twelve inferred targets and three supporting edges are retained while the tool is open. Geometry revisions or pending body transformations clear the retained points. Preferences → Tools → inferred move snap markers can disable hover inference; the explicit snap modes remain available.

### Compact controls and axis selection

Handle origin and angles now appear in a compact upper-right dock when **Move handles only** is enabled. Manual translation/rotation values are under **Manual values**; selecting an arrow exposes its nearby value field.

Axis arrows take priority over inferred markers behind them. Click an arrow, then a face to align to its normal or an edge to align to its direction, regardless of the origin snap-mode setting. Double-click an arrow to reverse that axis. Dragging an arrow cancels any armed geometry pick and translates normally. These controls apply to bodies and components.

Inferred snap markers use the theme's orange Surface ink with dark outlines for visibility over selected geometry. Move Parents and associative geometry alignment are not currently implemented; ordinary downstream feature recomputation is unchanged.

The component manipulator also exposes an editable local distance or angle beside a selected arrow/ring. Unit expressions are accepted. In handles-only mode the numeric offset changes the draft frame, preserving the component; otherwise it previews component placement. Replacing an entered offset does not accumulate a second move. A new geometry snap resets the numeric handle-offset baseline.
