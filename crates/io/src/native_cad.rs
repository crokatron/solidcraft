//! Independent CAD readers using [cadmpeg](https://github.com/cadmpeg/cadmpeg): FreeCAD,
//! Rhino, Parasolid and supported NX part representations. The decoder builds a neutral model and writes that as
//! STEP, which the Import feature then reads and keeps like any STEP file.
//!
//! Only explicitly enabled codecs are compiled in. No commercial CAD installation is called.
//! Incomplete NX geometry is refused; source history is not recreated as native features.
//!
//! This module is the importer seam: the one place cadmpeg is called, returning STEP text plus
//! warnings. A cadmpeg-based STEP reader could be added here later and chosen by a setting, to
//! compare with our own STEP reader, without changing that reader.

/// File extensions read through cadmpeg (lower case).
pub const NATIVE_CAD_EXTS: [&str; 5] = ["fcstd", "3dm", "prt", "x_t", "x_b"];

/// Is this a supported native CAD file name (any case)?
pub fn is_native_cad_path(path: &str) -> bool {
    std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| NATIVE_CAD_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Restate a supported native CAD file (`file` is its name) as STEP text, with the losses
/// and warnings cadmpeg reported. Panics inside cadmpeg become errors.
pub fn native_cad_to_step(bytes: &[u8], file: &str) -> Result<(String, Vec<String>), String> {
    let name = std::path::Path::new(file).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| file.to_string());
    solidcraft_kernel::guard("cadmpeg import", || convert::to_step(&name, bytes).map_err(solidcraft_kernel::KernelError::Invalid)).map_err(
        |e| match e {
            solidcraft_kernel::KernelError::Invalid(m) => m,
            e => e.to_string(),
        },
    )
}

mod convert {
    use std::io::Cursor;

    use cadmpeg_core::decode::{DecodeArena, DecodeContext, DecodeDepth, DecodeMode, DecodePolicy, ResourceLimits, View};
    use cadmpeg_ir::codec::write::{EncodeInput, target::TargetRequest};
    use cadmpeg_ir::codec::{DecodeFailure, DecodeOptions, DecodeResult};
    use cadmpeg_ir::validate::validate_neutral_for_decode;
    use cadmpeg_registry::{DETECTION_PREFIX_LEN, Format, InputCatalog, ResolveSourceError, ResolvedSource, build_encoder, forced_input};

    /// The codecs we accept (cadmpeg codec ids), whatever else the registry recognises.
    const ACCEPTED: [&str; 3] = ["fcstd", "rhino", "nx"];

    /// Caps for hostile input: cadmpeg's tight `service` profile, with our STEP size limit on
    /// the input. The service profile already admits a detailed part's 8 million entities:
    /// cadmpeg 0.6.3 dropped the collection-item budget that refused one at a million (#1783)
    /// and the work-unit budget that went with it.
    pub(super) fn policy() -> DecodePolicy {
        DecodePolicy {
            mode: DecodeMode::Salvage,
            limits: ResourceLimits { max_input_bytes: crate::limits::Kind::Step.limit(), ..ResourceLimits::service() },
        }
    }

    /// Decode `bytes` (a file called `name`, extension `ext`) with the codec cadmpeg detects from
    /// the bytes, else the one the extension names, if it is one of `accepted`. Returns the codec id
    /// and the decoded model.
    pub(super) fn decode(name: &str, ext: &str, bytes: &[u8], accepted: &[&str]) -> Result<(String, DecodeResult), String> {
        let policy = policy();
        let catalog = InputCatalog::with_builtins();
        let codec = {
            let arena = DecodeArena::new();
            let (ctx, _) = DecodeContext::from_root_bytes(&[], &arena, &policy).map_err(|e| e.to_string())?;
            let prefix = bytes.get(..DETECTION_PREFIX_LEN).unwrap_or(bytes);
            let resolved = catalog.resolve_seekable_source(&ctx, View::over_retained(prefix), &mut Cursor::new(bytes), None);
            let resolved = match resolved {
                Ok(ResolvedSource::Unrecognized | ResolvedSource::Cadir) | Err(ResolveSourceError::Ambiguous(_)) => {
                    match forced_input(ext).map_err(|e| e.to_string())? {
                        Some(forced) => catalog.resolve_source(&ctx, View::over_retained(prefix), Some(forced)),
                        None => Ok(ResolvedSource::Unrecognized),
                    }
                }
                other => other,
            };
            ctx.finish_session().map_err(|e| e.to_string())?;
            match resolved.map_err(|e| format!("{name}: {e}"))? {
                ResolvedSource::Native { codec, .. } => codec,
                _ => return Err(format!("{name}: unrecognised file contents")),
            }
        };
        let format = codec.id().as_str().to_string();
        if !accepted.contains(&format.as_str()) {
            return Err(format!("{name}: {format} contents are not supported by this CAD reader"));
        }

        // Decode in memory under the caps, as deep as the STEP writer reads: solids and products,
        // not the design history, which the import does not keep.
        let options = DecodeOptions { depth: DecodeDepth::Geometry, policy };
        let decoded = codec.decode(&mut Cursor::new(bytes), &options).map_err(|e| match e {
            DecodeFailure::Codec(e) => format!("{name}: could not read this {format} file: {e}"),
            other => format!("{name}: {other}"),
        })?;
        Ok((format, decoded))
    }

    pub(super) fn to_step(name: &str, bytes: &[u8]) -> Result<(String, Vec<String>), String> {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if !super::NATIVE_CAD_EXTS.contains(&ext.as_str()) {
            return Err(format!("{name}: unsupported CAD file extension"));
        }
        if matches!(ext.as_str(), "x_t" | "x_b") {
            let ir =
                cadmpeg_codec_sldprt::standalone_parasolid::decode(bytes).map_err(|e| format!("{name}: could not read this Parasolid file: {e}"))?;
            return ir_to_step(name, bytes.len(), "parasolid", &ir, None, Vec::new());
        }
        let (format, decoded) = decode(name, &ext, bytes, &ACCEPTED)?;
        decoded_to_step(name, bytes.len(), &format, decoded)
    }

    pub(super) fn decoded_to_step(name: &str, input_len: usize, format: &str, decoded: DecodeResult) -> Result<(String, Vec<String>), String> {
        let (ir, report, fidelity) = decoded.into_parts();
        if format == "nx" {
            let omitted: Vec<_> =
                report.losses.iter().filter(|loss| geometry_loss(loss)).take(6).map(|loss| format!("{}: {}", loss.code, loss.message)).collect();
            if !omitted.is_empty() {
                return Err(format!(
                    "{name}: this NX part contains geometry or evaluated state the independent reader cannot yet reconstruct. {}",
                    omitted.join("; ")
                ));
            }
        }
        if !report.geometry_transferred() {
            return Err(format!("{name}: no geometry could be read from this {format} file"));
        }
        let warnings: Vec<String> = report.losses.iter().map(|l| format!("{}: {}", l.code, l.message)).collect();
        ir_to_step(name, input_len, format, &ir, Some(&fidelity), warnings)
    }

    pub(super) fn ir_to_step(
        name: &str,
        input_len: usize,
        format: &str,
        ir: &cadmpeg_ir::CadIr,
        fidelity: Option<&cadmpeg_ir::source_fidelity::SourceFidelity>,
        mut warnings: Vec<String>,
    ) -> Result<(String, Vec<String>), String> {
        let policy = policy();

        // Validate the neutral model, as cadmpeg's own conversion does.
        let arena = DecodeArena::new();
        let ctx = DecodeContext::for_loaded_input(&arena, &policy, input_len as u64).map_err(|e| e.to_string())?;
        let validation = validate_neutral_for_decode(&ctx, ir, Vec::new()).map_err(|e| e.to_string())?;
        ctx.finish_session().map_err(|e| e.to_string())?;
        warnings.extend(validation.findings.iter().map(|f| format!("{}: {}", f.severity, f.message)));
        if !validation.is_ok() {
            return Err(format!("{name}: the {format} model is invalid ({} errors)", validation.error_count()));
        }

        // Write it as STEP AP214.
        let plan = build_encoder(Format::Step)
            .plan(EncodeInput::new(ir, fidelity), TargetRequest::Explicit("ap214"))
            .map_err(|e| format!("{name}: could not restate as STEP: {e}"))?;
        let losses = &plan.report().losses;
        if matches!(format, "nx" | "parasolid")
            && let Some(loss) = losses.iter().find(|loss| geometry_loss(loss))
        {
            return Err(format!("{name}: cannot transfer all geometry to SolidCraft ({}: {})", loss.code, loss.message));
        }
        if losses.iter().any(|l| l.code.to_string() == "step/export.no-exportable-solids") {
            return Err(format!("{name}: this {format} file has no solids that can be imported"));
        }
        warnings.extend(losses.iter().map(|l| format!("{}: {}", l.code, l.message)));
        let mut out = Vec::new();
        plan.write_to(&mut out).map_err(|e| e.to_string())?;
        let step = String::from_utf8(out).map_err(|e| e.to_string())?;
        Ok((super::name_unnamed_products(step, name), warnings))
    }

    /// A successful container decode is not sufficient evidence of a complete part.
    fn geometry_loss(loss: &cadmpeg_ir::report::loss::LossNote) -> bool {
        use cadmpeg_ir::report::{Severity, loss::LossTaxonomy};
        loss.severity >= Severity::Error
            || matches!(
                loss.code.taxonomy(),
                LossTaxonomy::TopologyNotTransferred | LossTaxonomy::GeometryNotTransferred | LossTaxonomy::ObjectRecordsUntransferred
            )
            || matches!(
                loss.code.local_code(),
                "deltas.selection-unresolved"
                    | "history.sub-body-composition-unresolved"
                    | "configuration.state-unresolved"
                    | "feature.suppression-unresolved"
                    | "source.scan-bounded"
            )
    }
}

/// cadmpeg's STEP writer calls a product it has no name for `cadmpeg_model`; name it after the
/// file instead (as plain STEP text: letters, digits, spaces, `-`, `_` and `.` only).
fn name_unnamed_products(step: String, file: &str) -> String {
    const PLACEHOLDER: &str = "'cadmpeg_model'";
    let stem: String = std::path::Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.')).collect())
        .unwrap_or_default();
    if stem.trim().is_empty() || !step.contains(PLACEHOLDER) { step } else { step.replace(PLACEHOLDER, &format!("'{stem}'")) }
}

#[cfg(test)]
mod tests;
