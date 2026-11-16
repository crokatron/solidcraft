//! Mirror datums use existing custom construction planes and scan ownership metadata.
//! `scan_mirror_plane:<feature>` preserves the original world-space definition and paired-landmark
//! residuals for reproducibility; these are original observations, not live transformed picks.
//! Older designs have no mirror recipe. Earlier releases warn and preserve unknown metadata.

//! SurfaceMatch optionally stores an explicit source trim edge, cross-boundary scale,
//! and target orientation for exact bounded polynomial matching. Older features use
//! an empty source edge list and retain their existing matching algorithm. Older releases
//! warn on the newer design format and preserve unrecognized fields.
