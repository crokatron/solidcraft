//! Bounded scan surfaces use existing Import B-rep features, with a metadata entry
//! `scan_surface_fit:<feature>` recording version, source feature/index, selected-facet
//! fingerprint and fit options/ranges. Geometry is embedded and survives without the scan.
//! Older designs have no recipes. The last release opens a newer design with a warning
//! and preserves unknown data; fitted surfaces remain ordinary editable base surfaces.
