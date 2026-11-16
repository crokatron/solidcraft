# Independent native NX export: research status

Native NX `.prt` export is **not implemented or validated**. STEP and Parasolid assembly exchange are separate deliverables; neither satisfies the native `.prt` request. A renamed exchange file or a synthetic SPLMSSTR wrapper must not appear as an NX export option.

## What the available code establishes

The vendored `cadmpeg-codec-nx` package is Apache-2.0 and implements detection, inspection, decoding and validation. `src/lib.rs` has no native encoder implementation. Its current public [support matrix](https://github.com/cadmpeg/cadmpeg/blob/main/docs/format-support.md#siemens-nx-prt) explicitly lists **Write: none** for both modern SPLMSSTR and legacy CFB dialects.

`src/test_support/test_prt.rs::prt_with_named_payloads` builds synthetic container fixtures. `single_part_prt` puts compressed Parasolid into one canonical payload. These prove that the project's parser accepts its test inputs. They do not demonstrate that Siemens NX can open, save or edit those files. Reusing those fixtures as a customer-facing writer would overstate the evidence.

The public [cadmpeg NX format notes](https://github.com/cadmpeg/cadmpeg/blob/main/docs/formats/siemens_nx.md), licensed CC-BY-4.0 and attributed to cadmpeg, document container framing and many native records. They provide a useful research foundation, not a complete normative native-write contract. The code preserves the terminal footer fingerprint rather than computing or validating its meaning. The README also identifies unresolved assembly-placement bindings and translation units. No complete minimal NX-loadable document recipe was established in this review.

## First genuine milestone

Target **one new part containing one exact, nonparametric imported solid**, with fixed units, stable identity and a standard coordinate system. Exclude assembly references, editable feature history, sketches, expressions and display assets initially. This is a proposed smallest useful target, not a claim that the necessary record set is already known.

Completion requires a newly generated `.prt` to open as native geometry in NX, preserve its dimensions and topology, save successfully, and reopen. The writer itself must remain independent of NX. Under the current prohibition on NX automation, that acceptance evidence would need a separate permitted/manual validation step; local parser success alone cannot establish it.

## Missing work and evidence

1. Establish the minimum required native part/object records and their defaults for one deliberately chosen NX version. Determine which unit, coordinate-system, object-manager, identity and active-body records are mandatory, optional or derivable. Current read coverage does not establish those write requirements.
2. Serialize a coherent `UG_PART` object graph linking the part and body to a complete exact Parasolid image. Understand and regenerate the relevant identities, section indices, references and active-body membership instead of copying unknown bytes from a template.
3. Resolve container fields that the reader treats as opaque, particularly fingerprint/file-tag semantics, or establish a genuinely accepted alternative layout. The parser's no-footer branch is not evidence of NX compatibility.
4. Extend to multibody parts only after the single-solid milestone passes. Native assemblies additionally need external child `.prt` files, coherent references, occurrence identities, transforms and units. Recovering placement matrices is insufficient without proven child binding.
5. Design feature-history export as a later independent project. A geometric imported body is useful native output but does not reproduce SolidCraft's editable history.

## Tests available without NX

- Deterministic container serialization and strict offset, length, directory, compression and reference checks.
- Re-decode generated files; compare body count, units, placements, bounds, volume, face/edge/vertex connectivity and orientation against the source.
- Validate one-to-one object identities and all cross-record links; reject unresolved dependencies and partial geometry.
- Property/fuzz tests for malformed counts, truncated streams, duplicate identities and round-trip stability.
- Compare against independently authored, authorized reference parts and controlled byte-level differences. Preserve provenance of every inferred field.

These tests establish structural and semantic consistency against the known subset. They **cannot certify native NX acceptance**, because the reader may ignore records that NX requires or share an incorrect interpretation with the writer.

Only public information, permitted file observation and compatible open-source code were used. Do not use proprietary SDKs, commercial application internals, decompilation or incompatible code; the upstream [provenance policy](https://github.com/cadmpeg/cadmpeg/blob/main/LEGAL.md) supplies the relevant boundaries.
