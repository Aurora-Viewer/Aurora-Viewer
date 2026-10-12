//! Offline import fixtures: the same drafts, previews, fee confirmation and
//! upload queue as real files, with the application's local demo server.

use super::*;
pub fn start(imports: &mut Imports, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
    let Ok(mode) = std::env::var("AURORA_DEMO_IMPORTS") else {
        return;
    };
    imports.open = true;
    imports.choice = match mode.as_str() {
        "texture" => Choice::Texture,
        "material" => Choice::Material,
        "mesh" => Choice::Mesh,
        "animation" => Choice::Animation,
        "sound" => Choice::Sound,
        "local" => Choice::Local,
        _ => Choice::Bulk,
    };
    let choice = imports.choice;
    let request = Uuid::new_v4();
    imports.pending_files = Some(request);
    let tx = imports.tx.clone();
    let wake = ctx.clone();
    jobs.execute(move|| {
        let mut pixels=Vec::new();
        for y in 0..256 {for x in 0..512 {pixels.extend(if (x/32+y/32)%2==0{[120,65,220,255]}else{[55,220,190,190]});}}
        let image=Arc::new(DecodedImage{width:512,height:256,components:4,data:pixels,discard:0});
        let mut entries=vec![Entry::new(".scratch/import-demo.png".into(),Ok(Source::Texture(image.clone())),Default::default())];
        let mut local=files::local_image((*image).clone(),"import-demo".into());local.name="Base color".into();
        entries.push(Entry::new(".scratch/material-demo.gltf".into(),Ok(Source::Material {value:PbrMaterial{roughness_factor:0.25,metallic_factor:0.65,..Default::default()},images:Box::new([Some(local),None,None,None])}),Default::default()));
        let positions=[[-0.5,-0.5,-0.5],[0.5,-0.5,-0.5],[0.5,0.5,-0.5],[-0.5,0.5,-0.5],[-0.5,-0.5,0.5],[0.5,-0.5,0.5],[0.5,0.5,0.5],[-0.5,0.5,0.5]];
        let face=aurora_assets::import_mesh::Face{material:0,indices:vec![0,2,1,0,3,2,4,5,6,4,6,7,0,1,5,0,5,4,3,7,6,3,6,2,0,4,7,0,7,3,1,2,6,1,6,5],
            vertices:positions.into_iter().map(|position|aurora_assets::import_mesh::Vertex{position,normal:glam::Vec3::from_array(position).normalize().to_array(),uv:[position[0]+0.5,position[2]+0.5],joints:[0;4],weights:[0.0;4]}).collect()};
        let scene=aurora_assets::import_mesh::Scene{models:vec![aurora_assets::import_mesh::Model{name:"Cube".into(),faces:vec![face],skin:None,center:glam::Vec3::ZERO,size:glam::Vec3::ONE}],
            instances:vec![aurora_assets::import_mesh::Instance{model:0,transform:glam::Mat4::IDENTITY,name:"Cube".into(),materials:vec![0]}],
            materials:vec![aurora_assets::import_mesh::Material{name:"Demo".into(),value:PbrMaterial{metallic_factor:0.0,..Default::default()},images:[None;4]}],..Default::default()};
        entries.push(Entry::new(".scratch/mesh-demo.dae".into(),Ok(Source::Mesh(Box::new(mesh::Draft::new(scene,HashMap::new())))),Default::default()));
        let skeleton=aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml"));
        let bvh=b"HIERARCHY ROOT hip { OFFSET 0 0 0 CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation End Site { OFFSET 0 1 0 } } MOTION Frames: 6 Frame Time: 0.5 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 15 0 0 0 0 0 -15 0 0 0 0 0 15 0 0 0 0 0 0 0 0";
        let animation=skeleton.and_then(|sk|aurora_assets::import_animation::parse_bvh(bvh,&sk)).map(|bvh|Source::Animation{bytes:Arc::new(Vec::new()),bvh:Some(BvhInfo{frames:bvh.frames,frame_time:bvh.frame_time}),animation:Arc::new(bvh.animation)}).map_err(|e|e.to_string());
        entries.push(Entry::new(".scratch/animation-demo.bvh".into(),animation,Default::default()));
        let sound=aurora_assets::import_sound::Sound{source_channels:2,source_bits:16,mono:(0..44100).map(|i|(i as f32*440.0*std::f32::consts::TAU/44100.0).sin()*0.15).collect()};
        entries.push(Entry::new(".scratch/sound-demo.wav".into(),Ok(Source::Sound(Arc::new(sound))),Default::default()));
        if choice==Choice::Local {entries.retain(|e|matches!(e.source,Some(Source::Texture(_)|Source::Material{..})));for e in &mut entries{e.local=true;}}
        let _=tx.send(Worker::Loaded(request,entries));wake.request_repaint();
    });
}
