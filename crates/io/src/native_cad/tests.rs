use super::*;

/// A 10 × 20 × 30 mm box called "Block", saved by FreeCAD 1.1.4 (`fixtures/make_block_fcstd.py`).
const BLOCK_FCSTD: &[u8] = include_bytes!("fixtures/block.FCStd");

/// The same box as a Rhino 7 file, written with McNeel's MIT-licensed rhino3dm package
/// (`fixtures/make_block_3dm.py`).
const BLOCK_3DM: &[u8] = include_bytes!("fixtures/block.3dm");

// Original analytical fixtures, generated without a commercial CAD installation.
const CUBE_PRT: &[u8] = include_bytes!("fixtures/cube.prt");
const CUBE_XT: &[u8] = include_bytes!("fixtures/cube10.x_t");
const CUBE_XB: &[u8] = include_bytes!("fixtures/cube10.x_b");

#[test]
fn independent_nx_and_parasolid_cubes_preserve_geometry_and_units() {
    for (bytes, name) in [(CUBE_PRT, "cube.prt"), (CUBE_XT, "cube.x_t"), (CUBE_XB, "cube.x_b")] {
        let imported = crate::step_import_feature(bytes, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(imported.import.bodies.len(), 1, "{name}");
        assert!(imported.import.bodies[0].closed, "{name}: cube is not a closed solid");
        let measured = solidcraft_kernel::measure(&imported.import.bodies[0].body).unwrap();
        assert!((measured.volume - 1000.0).abs() < 1e-6, "{name}: volume {}", measured.volume);
        assert!((measured.area - 600.0).abs() < 1e-6, "{name}: area {}", measured.area);
        assert_eq!(measured.faces, 6, "{name}");
        assert_eq!(measured.edges, 12, "{name}");
        assert_eq!(measured.vertices, 8, "{name}");
    }
}

#[test]
fn incomplete_independent_cad_files_are_refused() {
    for (good, name) in [(CUBE_PRT, "cube.prt"), (CUBE_XT, "cube.x_t"), (CUBE_XB, "cube.x_b")] {
        for length in [0, 1, good.len() / 3, good.len() / 2] {
            assert!(native_cad_to_step(&good[..length], name).is_err(), "{name}: accepted truncated input of {length} bytes");
        }
    }
}

/// STEP text, which these importers refuse.
const STEP_TEXT: &str = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";

/// The file imports as one solid with the box's volume, through the same Import feature as STEP.
fn imports_the_block(bytes: &[u8], file: &str) {
    let step = native_cad_to_step(bytes, file).unwrap().0;
    assert!(step.contains("MANIFOLD_SOLID_BREP"), "{file}: no solid in the restated STEP");
    let sf = crate::step_import_feature(bytes, &format!("dir/{file}")).unwrap();
    assert_eq!(sf.import.bodies.len(), 1, "{file}");
    let solidcraft_doc::FeatureKind::Import { file: held, step, .. } = &sf.kind else { panic!("not an Import feature") };
    assert_eq!(held, file);
    assert!(step.contains("MANIFOLD_SOLID_BREP"), "the feature keeps the restated STEP");
    let v = solidcraft_kernel::measure(&sf.import.bodies[0].body).unwrap().volume;
    assert!((v - 6000.0).abs() < 1e-6, "{file}: volume {v}");
    // FreeCAD's object label does not come through cadmpeg; the product is then named after the file.
    let stem = file.rsplit_once('.').unwrap().0;
    assert!(["Block", stem].contains(&sf.body_names[0].as_str()), "{file}: {:?}", sf.body_names);
    assert!(!step.contains("cadmpeg_model"), "{file}");
}

#[test]
fn extensions() {
    assert!(is_native_cad_path("a/Part.FCStd") && is_native_cad_path("x.3DM") && is_native_cad_path("y.fcstd"));
    for other in ["c.sat", "c.sab", "c.smt", "c.smb", "c.f3d", "c.sldprt", "c.step", "fcstd"] {
        assert!(!is_native_cad_path(other), "{other}");
    }
    assert!(crate::is_step_path("c.FCStd") && crate::is_step_path("c.3dm"), "read as an Import feature like STEP and IGES");
}

#[test]
fn freecad_box_imports_as_a_solid() {
    let bytes = BLOCK_FCSTD;
    assert!(bytes.starts_with(b"PK"), "an FCStd file is a ZIP archive");
    imports_the_block(bytes, "block.FCStd");
}

#[test]
fn rhino_box_imports_as_a_solid() {
    let bytes = BLOCK_3DM;
    assert!(bytes.starts_with(b"3D Geometry File Format"), "a 3dm file starts with its signature");
    imports_the_block(bytes, "block.3dm");
}

/// Only FreeCAD and Rhino contents are read: STEP text, garbage, a bad ZIP and empty input are
/// refused with a message, as are ACIS, ShapeManager and commercial formats whatever their bytes.
#[test]
fn other_contents_are_refused() {
    let step = STEP_TEXT;
    let fcstd = BLOCK_FCSTD;
    let cases: [(&[u8], &str); 9] = [
        (step.as_bytes(), "x.3dm"),
        (step.as_bytes(), "x.fcstd"),
        (b"\0\0garbage", "x.3dm"),
        (b"PK\x03\x04junk", "x.fcstd"),
        (b"", "x.3dm"),
        (b"700 0 1 0\n", "x.sat"),
        (b"ASM BinaryFile4", "x.smb"),
        (fcstd, "x.f3d"),
        (fcstd, "x.sldprt"),
    ];
    for (bytes, name) in cases {
        let e = native_cad_to_step(bytes, name).expect_err(name);
        assert!(e.starts_with(name), "{name}: the message names the file: {e}");
    }
    // A Rhino file named .FCStd is still read as what it is.
    imports_the_block(BLOCK_3DM, "renamed.FCStd");
}

/// Truncated, bit-flipped and garbage-tailed variants of valid FreeCAD and Rhino files give an
/// error or a solid, never a panic.
#[test]
fn hostile_native_cad_never_panics() {
    for (good, name) in
        [(BLOCK_FCSTD, "hostile.FCStd"), (BLOCK_3DM, "hostile.3dm"), (CUBE_PRT, "hostile.prt"), (CUBE_XT, "hostile.x_t"), (CUBE_XB, "hostile.x_b")]
    {
        let mut cases: Vec<Vec<u8>> = (0..good.len()).step_by((good.len() / 60).max(1)).map(|n| good[..n].to_vec()).collect();
        for i in (0..good.len()).step_by((good.len() / 80).max(1)) {
            let mut b = good.to_vec();
            b[i] ^= 0x5a;
            cases.push(b);
            let mut b = good.to_vec();
            b[i] = 0xff;
            cases.push(b);
        }
        let mut tail = good.to_vec();
        tail.extend((0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8));
        cases.push(tail);
        cases.push(good.iter().rev().copied().collect());
        for (i, bytes) in cases.iter().enumerate() {
            let r = std::panic::catch_unwind(|| crate::step_import_feature(bytes, name));
            assert!(r.is_ok(), "{name}: case {i} panicked");
        }
    }
}

/// cadmpeg decodes under its tight `service` caps, with our STEP size limit on the input.
/// cadmpeg (since 0.6.3) has no work-unit or collection-item budgets, and its service profile already
/// admits the 8 million entities a detailed part needed (#1783).
#[test]
fn decoding_is_capped() {
    let limits = convert::policy().limits;
    let service = cadmpeg_core::decode::ResourceLimits::service();
    assert_eq!(limits.max_input_bytes, crate::MAX_STEP_BYTES as u64);
    let same = cadmpeg_core::decode::ResourceLimits { max_input_bytes: service.max_input_bytes, ..limits };
    assert_eq!(same, service);
}

/// A FreeCAD archive whose Document.xml inflates past the per-entry cap (a ZIP bomb, 300 MB of
/// zeros in a few hundred KB) is refused.
#[test]
fn a_zip_bomb_is_refused() {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("Document.xml", zip::write::SimpleFileOptions::default().large_file(true)).unwrap();
    let zeros = vec![0u8; 1 << 20];
    for _ in 0..300 {
        zip.write_all(&zeros).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    assert!(bytes.len() < 2 << 20, "{} bytes", bytes.len());
    assert!(native_cad_to_step(&bytes, "bomb.FCStd").is_err());
}

// Independent XT writer: analytic dimensions and topology, not commercial-CAD output.
#[test]
fn parasolid_box_export() {
    let body = solidcraft_kernel::box_solid(solidcore_record::Vec3::new(7., -3., 11.), solidcore_record::Vec3::new(17., 17., 41.)).unwrap();
    let body = solidcraft_doc::ModelBody::new("Box".into(), body, 1);
    let data = crate::parasolid_out::export(&[&body]).unwrap();

    let imported = crate::step_import_feature(&data, "box.x_t").unwrap();
    assert!(imported.import.bodies[0].closed);
    let m = solidcraft_kernel::measure(&imported.import.bodies[0].body).unwrap();
    assert_eq!(m.faces, 6);
    assert_eq!(m.edges, 12);
    assert_eq!(m.vertices, 8);
    assert!((m.volume - 6000.).abs() < 1e-6, "{}", m.volume);
    assert!((m.area - 2200.).abs() < 1e-6);
}

#[test]
fn parasolid_hole_export() {
    use solidcore_record::Vec3;
    use solidcraft_kernel as k;
    let outer = k::box_solid(Vec3::ZERO, Vec3::new(20., 20., 10.)).unwrap();
    let tool = k::box_solid(Vec3::new(5., 5., -1.), Vec3::new(15., 15., 11.)).unwrap();
    let body = k::boolean(&outer, &tool, k::BoolOp::Cut).unwrap().unwrap();
    let body = solidcraft_doc::ModelBody::new("Frame".into(), body, 1);
    let data = crate::parasolid_out::export(&[&body]).unwrap();
    let imported = crate::step_import_feature(&data, "frame.x_t").unwrap();
    assert!(imported.import.bodies[0].closed);
    let m = k::measure(&imported.import.bodies[0].body).unwrap();
    assert!((m.volume - 3000.).abs() < 1e-6, "{}", m.volume);
    assert!((m.area - 1800.).abs() < 1e-6, "{}", m.area);
}
#[test]
fn parasolid_curved_multiple_bodies() {
    use solidcore_record::Vec3;
    use solidcraft_kernel as k;
    let cylinder=k::cylinder(Vec3::ZERO,Vec3::Z,5.,10.).unwrap();
    let body=solidcraft_doc::ModelBody::new("Cylinder".into(),cylinder,1);
    let shifted=k::transform(&body.body,Vec3::new(30.,0.,0.),Vec3::ZERO,Vec3::Z,0.).unwrap();
    let shifted=solidcraft_doc::ModelBody::new("Other cylinder".into(),shifted,2);
    let bytes=crate::parasolid_out::export(&[&body,&shifted]).unwrap();
    let imported=crate::step_import_feature(&bytes,"cylinders.x_t").unwrap();
    assert_eq!(imported.import.bodies.len(),2);
    for b in &imported.import.bodies {
        assert!(b.closed);
        let measured=k::measure(&b.body).unwrap();
        assert!((measured.volume-250.*std::f64::consts::PI).abs()<1e-3,"{}",measured.volume);
    }
}

#[test]
fn parasolid_rotated_placement_and_mesh_refusal() {
    use solidcore_record::Vec3;
    use solidcraft_kernel as k;
    let box_body = k::box_solid(Vec3::ZERO, Vec3::new(10., 20., 30.)).unwrap();
    let moved = k::transform(&box_body, Vec3::new(5., 7., 11.), Vec3::ZERO, Vec3::Z, std::f64::consts::FRAC_PI_2).unwrap();
    let body = solidcraft_doc::ModelBody::new("Rotated".into(), moved, 1);
    let data = crate::parasolid_out::export(&[&body]).unwrap();
    assert!(data.split(|b| *b == b'\n').all(|line| line.len() <= 80));
    let imported = crate::step_import_feature(&data, "rotated.x_t").unwrap();
    let record = imported.import.bodies[0].body.record();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for vertex in &record.vertices {
        for k in 0..3 {
            low[k] = low[k].min(vertex.point[k]);
            high[k] = high[k].max(vertex.point[k]);
        }
    }
    for k in 0..3 {
        assert!((low[k] - [-15., 7., 11.][k]).abs() < 1e-7);
        assert!((high[k] - [5., 17., 41.][k]).abs() < 1e-7);
    }
    let mesh = k::mesh_body(&[Vec3::ZERO, Vec3::X, Vec3::Y], &[[0, 1, 2]]).unwrap();
    let mesh = solidcraft_doc::ModelBody::new("Scan".into(), mesh, 2);
    assert!(crate::parasolid_out::export(&[&mesh]).unwrap_err().to_string().contains("scan mesh"));
}
