// SPDX-License-Identifier: Apache-2.0
//! Standalone neutral-binary Parasolid decoding. Local SolidCraft adapter.
use cadmpeg_core::decode::{DecodeArena, DecodeContext, DecodePolicy, View};
use cadmpeg_core::CodecError;
use cadmpeg_ir::CadIr;
use cadmpeg_parasolid::records::PrimaryRecords;

/// Decode a complete neutral-binary XT stream, with an optional standard text header.
/// Unsupported schemas and incomplete node sequences are errors, never empty models.
pub fn decode_binary(bytes: &[u8]) -> Result<CadIr, CodecError> {
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(CodecError::NotImplemented(
            "Parasolid input exceeds the 256 MiB local limit".into(),
        ));
    }
    let payload = if bytes.starts_with(b"PS") {
        bytes
    } else {
        let marker = b"**END_OF_HEADER";
        let start = bytes
            .get(..bytes.len().min(65536))
            .ok_or_else(|| CodecError::Malformed("invalid header window".into()))?
            .windows(marker.len())
            .position(|w| w == marker)
            .ok_or_else(|| {
                CodecError::Malformed("missing Parasolid neutral-binary signature".into())
            })?;
        let tail = bytes
            .get(start..)
            .ok_or_else(|| CodecError::Malformed("invalid XT header".into()))?;
        let newline = tail
            .get(..tail.len().min(65536))
            .ok_or_else(|| CodecError::Malformed("invalid XT header window".into()))?
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| CodecError::Malformed("unterminated XT header".into()))?;
        tail.get(newline + 1..)
            .ok_or_else(|| CodecError::Malformed("missing XT body".into()))?
    };
    let records = PrimaryRecords::read(payload)?.ok_or_else(|| {
        CodecError::NotImplemented(
            "unsupported Parasolid header/schema (neutral binary base 11004 or 13006 required)"
                .into(),
        )
    })?;
    if let Some(offset) = records.stop_offset() {
        return Err(CodecError::NotImplemented(format!(
            "unsupported or incomplete Parasolid node sequence at byte {offset}"
        )));
    }
    if records.records().len() > 1_000_000 {
        return Err(CodecError::NotImplemented(
            "Parasolid node count exceeds the local limit".into(),
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    if records.nodes().any(|node| !identities.insert(node.index())) {
        return Err(CodecError::Malformed(
            "duplicate Parasolid node identity".into(),
        ));
    }
    let source_faces = records.nodes().filter(|n| n.kind() == 14).count();
    let source_bodies = records.nodes().filter(|n| n.kind() == 12).count();
    let mut view = View::over_retained(payload);
    view.take(2)
        .ok_or_else(|| CodecError::Malformed("missing XT signature".into()))?;
    let description_len = view
        .u32_be()
        .ok_or_else(|| CodecError::Malformed("missing description length".into()))?
        as usize;
    let description = view
        .take(description_len)
        .ok_or_else(|| CodecError::Malformed("incomplete description".into()))?;
    let schema_len =
        view.u32_be()
            .ok_or_else(|| CodecError::Malformed("missing schema length".into()))? as usize;
    let schema = view
        .take(schema_len)
        .ok_or_else(|| CodecError::Malformed("incomplete schema".into()))?;
    let schema =
        std::str::from_utf8(schema).map_err(|_| CodecError::Malformed("invalid schema".into()))?;
    let header = crate::parasolid::StreamHeader {
        description: String::from_utf8_lossy(description).into_owned(),
        schema: cadmpeg_parasolid::OwnedSchemaToken::try_from(schema)
            .map_err(|_| CodecError::Malformed("invalid schema".into()))?,
        body_offset: view.position(),
    };
    if !crate::brep::graph::standalone_node_graph_supported(payload, header.body_offset) {
        return Err(CodecError::NotImplemented(
            "unsupported Parasolid topology schema fields".into(),
        ));
    }
    let arena = DecodeArena::new();
    let (ctx, _) = DecodeContext::from_root_bytes(payload, &arena, &DecodePolicy::service())?;
    let brep = crate::brep::graph::decode_bodies(
        &ctx,
        &[(payload, &header)],
        &cadmpeg_ir::stream_name!("parasolid"),
    )?;
    crate::brep::graph::standalone_topology_complete(&records, &brep)?;
    if brep.faces.is_empty() || brep.bodies.is_empty() {
        return Err(CodecError::NotImplemented(
            "Parasolid contains no supported bounded bodies".into(),
        ));
    }
    if brep.faces.len() != source_faces
        || brep.bodies.len() != source_bodies
        || !brep.unknowns.is_empty()
    {
        return Err(CodecError::NotImplemented(format!(
            "Parasolid geometry is incomplete: transferred {}/{} faces and {}/{} bodies",
            brep.faces.len(),
            source_faces,
            brep.bodies.len(),
            source_bodies
        )));
    }
    if let Some(loss) = brep.losses.first() {
        return Err(CodecError::NotImplemented(format!(
            "Parasolid geometry decoding reported unsupported data: {}",
            loss.message
        )));
    }
    let mut ir = CadIr::empty();
    ir.model.bodies = brep.bodies;
    ir.model.regions = brep.regions;
    ir.model.shells = brep.shells;
    ir.model.faces = brep.faces;
    ir.model.loops = brep.loops;
    ir.model.coedges = brep.coedges;
    ir.model.edges = brep.edges;
    ir.model.vertices = brep.vertices;
    ir.model.points = brep.points;
    ir.model.surfaces = brep.surfaces;
    ir.model.procedural_surfaces = brep.procedural_surfaces;
    ir.model.curves = brep.curves;
    ir.model.procedural_curves = brep.procedural_curves;
    ir.model.pcurves = brep.pcurves;
    let validation = cadmpeg_ir::validate::validate_neutral_for_decode(&ctx, &ir, Vec::new())?;
    if !validation.is_ok() {
        let detail = validation
            .findings
            .iter()
            .find(|f| f.severity >= cadmpeg_ir::report::Severity::Error)
            .map_or("invalid topology", |f| f.message.as_str());
        return Err(CodecError::Malformed(format!(
            "Parasolid topology validation failed: {detail}"
        )));
    }
    ctx.finish_session()?;
    Ok(ir)
}

/// Decode a text or neutral-binary standalone XT file.
pub fn decode(bytes: &[u8]) -> Result<CadIr, CodecError> {
    let prefix = bytes
        .get(..bytes.len().min(65536))
        .ok_or_else(|| CodecError::Malformed("invalid XT input".into()))?;
    if bytes.starts_with(b"T") || prefix.windows(11).any(|w| w == b"FORMAT=text") {
        let binary = cadmpeg_parasolid::records::text::to_neutral_binary(bytes)?;
        decode_binary(&binary)
    } else {
        decode_binary(bytes)
    }
}
