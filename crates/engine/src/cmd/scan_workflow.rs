//! Read-only inventory for the scan reconstruction workspace. No scan traversal.
use super::CommandSpec;
use crate::{
    Result, Session,
    schema::{TypedContract, output_schema_for},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Args {}

#[derive(Serialize, JsonSchema)]
struct Inventory {
    meshes: Vec<String>,
    cad_bodies: Vec<String>,
    revision: u64,
    cad_to_scan_deviation_available: bool,
}

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("scan.workflow", "Reverse Engineering", crate::checked!("scan.workflow", inventory))
        .icon("section").noundo().contract(&TypedContract::<Args>::new())
        .output(output_schema_for::<Inventory>)
        .params("Read the scan reconstruction source inventory without modifying geometry. The desktop opens a guided cleanup, alignment, section and surface workspace."),
];

fn inventory(s: &mut Session, _: &Value) -> Result<Value> {
    let state = s.world_state()?;
    Ok(json!(Inventory {
        meshes: state.bodies.iter().filter(|b| b.body.is_mesh()).map(|b| b.name.clone()).collect(),
        cad_bodies: state.bodies.iter().filter(|b| !b.body.is_mesh()).map(|b| b.name.clone()).collect(),
        revision: s.revision,
        cad_to_scan_deviation_available: false,
    }))
}
