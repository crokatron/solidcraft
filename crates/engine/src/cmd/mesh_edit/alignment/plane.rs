use super::*;
use solidcore_record::Plane;
use solidcraft_geom::PlaneGeom;
use solidcraft_doc::PlaneRef;

const CMD: &str = "mesh.align_plane_to_csys";
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::cmd::mesh_edit) struct Args {
    moving: String,
    plane: u64,
    target: String,
    #[serde(default)]
    flip: bool,
    #[serde(default)]
    angle_deg: f64,
    #[serde(default)]
    offset: [f64; 2],
}
#[derive(serde::Serialize, JsonSchema)]
pub(in crate::cmd::mesh_edit) struct Output {
    moving: String,
    plane: u64,
    angle_deg: f64,
    offset: [f64; 2],
    angular_error_deg: f64,
    distance_error_mm: f64,
    align_report: String,
}

pub(in crate::cmd::mesh_edit) fn owner_key(feature: u64) -> String {
    format!("solidcraft.scan_plane.source.{feature}")
}

fn owner(s:&Session, plane:u64)->Option<(u64,usize)> {
    let entry=s.doc.metadata.0.get(&owner_key(plane))?;
    Some((entry.get(0)?.as_u64()?, entry.get(1)?.as_u64()?.try_into().ok()?))
}

fn world_plane(s:&Session, plane:u64)->Result<Plane>{
    crate::view::construction_planes(s).into_iter().find(|row|row.0==plane)
        .map(|row|row.2).ok_or_else(||bad(CMD,"pick an evaluated construction plane"))
}

fn placement(plane:Plane,target:&str,flip:bool,angle_deg:f64,offset:[f64;2])->Result<(Mat,Vec3,Vec3)> {
    let target = match target.to_ascii_lowercase().as_str() {
        "xy" => 2, "xz" => 1, "yz" => 0,
        _ => return Err(bad(CMD,"target must be xy, xz or yz")),
    };
    let n = plane.normal();
    let result = solidcraft_kernel::scan_symmetry::align_to_csys(
        solidcraft_kernel::scan_symmetry::Plane { origin:[plane.origin.x,plane.origin.y,plane.origin.z],normal:[n.x,n.y,n.z] },
        target, flip, angle_deg, offset).map_err(|e|bad(CMD,e.to_string()))?;
    // Core exposes row-major matrices; document placements are column-major.
    let transform = std::array::from_fn(|column| std::array::from_fn(|row| result.transform[row][column]));
    Ok((transform,Vec3::new(result.pivot[0],result.pivot[1],result.pivot[2]),Vec3::new(result.axis[0],result.axis[1],result.axis[2])))
}

impl Session {
    /// The world-space rigid delta, aligned pivot and signed rotation axis, without modifying the document.
    pub fn preview_mesh_plane_alignment(&self,plane:u64,target:&str,flip:bool,angle_deg:f64)->Result<(Mat,Vec3,Vec3)> {
        self.preview_mesh_plane_alignment_offset(plane,target,flip,angle_deg,[0.,0.])
    }

    /// Apply a target-plane offset after alignment and clocking. The returned pivot includes that translation.
    pub fn preview_mesh_plane_alignment_offset(&self,plane:u64,target:&str,flip:bool,angle_deg:f64,offset:[f64;2])->Result<(Mat,Vec3,Vec3)> {
        placement(world_plane(self,plane)?,target,flip,angle_deg,offset)
    }

    /// Resolve the current body name from the original imported mesh feature and ordinal.
    pub fn fitted_plane_scan(&self,plane:u64)->Option<String> {
        let (feature,index)=owner(self,plane)?;
        let import=self.model.result(feature)?;
        let body=import.state.bodies.iter().filter(|b|b.feature==feature).nth(index)?;
        self.body_frames().iter().find(|(_,own,_)|own==&body.name).map(|(name,..)|name.clone())
    }
}

/// Move explicit fitted planes in their own component frame before the scan edit's refresh.
pub(super) fn move_linked(s:&mut Session,feature:u64,index:usize,transform:&Mat)->Result<()> {
    let ids:Vec<_>=s.doc.features.iter().filter(|f|owner(s,f.id)==Some((feature,index))).map(|f|f.id).collect();
    for id in ids {
        let Some(f)=s.doc.feature(id) else {continue};
        if !matches!(&f.kind,FeatureKind::ConstructionPlane{plane:PlaneRef::Custom{..}}){continue}
        let frame=s.doc.component_transform(f.component);
        let inverse=mat_inverse(&frame).ok_or_else(||bad(CMD,"fitted plane component has singular placement"))?;
        let local=mat_mul(&mat_mul(&inverse,transform),&frame);
        if let Some(f)=s.doc_mut().feature_mut(id){f.kind.transform_geometry(&local);}
    }
    Ok(())
}

pub(in crate::cmd::mesh_edit) fn run(s:&mut Session,p:&Value)->Result<Value>{
    let a:Args=parse(p,CMD)?;
    let moving=source(s,CMD,Some(&a.moving))?;
    let source_id=(moving.feature,moving.index);
    let plane=world_plane(s,a.plane)?;
    let (transform,_,axis)=placement(plane,&a.target,a.flip,a.angle_deg,a.offset)?;
    if let Some(known)=owner(s,a.plane){
        if known!=source_id{return Err(bad(CMD,"the selected fitted plane belongs to another scan"))}
    } else {
        let f=s.doc.feature(a.plane).ok_or_else(||bad(CMD,"missing construction plane"))?;
        if !matches!(&f.kind,FeatureKind::ConstructionPlane{plane:PlaneRef::Custom{..}}){
            return Err(bad(CMD,"select a fitted scan plane or an explicit custom plane on the moving scan"))
        }
        s.doc_mut().metadata.0.insert(owner_key(a.plane),json!([source_id.0,source_id.1]));
    }
    apply_world(s,CMD,&a.moving,&transform)?;
    let end=world_plane(s,a.plane)?;
    let n=end.normal();
    let angular_error_deg=n.cross(axis).len().atan2(n.dot(axis)).to_degrees();
    let distance_error_mm=end.origin.dot(axis).abs();
    Ok(json!(Output{moving:a.moving,plane:a.plane,angle_deg:a.angle_deg,offset:a.offset,angular_error_deg,distance_error_mm,
        align_report:format!("Aligned scan plane to {} with {:.2}° rotation and offsets {:.3}, {:.3} mm. Residual {:.6}° and {:.6} mm.",a.target.to_uppercase(),a.angle_deg,a.offset[0],a.offset[1],angular_error_deg,distance_error_mm)}))
}

#[test]
fn clocking_preserves_coincidence_for_each_signed_axis(){
    use crate::geom::prelude::*;
    let plane=Plane::from_normal(Vec3::new(7.,-3.,11.),Vec3::new(1.,2.,-4.)).unwrap();
    for target in ["xy","xz","yz"]{
        for flip in [false,true]{
            let (base,_,_)=placement(plane,target,flip,0.,[0.,0.]).unwrap();
            let (m,pivot,n)=placement(plane,target,flip,90.,[8.5,-12.25]).unwrap();
            assert!(pivot.dot(n).abs()<1e-12);
            assert!((apply_point(&m,plane.origin)-pivot).len()<1e-12);
            assert!((apply_vector(&m,plane.normal())-n).len()<1e-12);
            assert!((apply_vector(&m,plane.x)-n.cross(apply_vector(&base,plane.x))).len()<1e-12);
        }
    }
}
