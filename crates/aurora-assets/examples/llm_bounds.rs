fn main() {
    let dir = std::env::args().nth(1).unwrap_or_default();
    for name in [
        "avatar_upper_body.llm",
        "avatar_lower_body.llm",
        "avatar_head.llm",
        "avatar_eye.llm",
        "avatar_hair.llm",
    ] {
        let d = std::fs::read(format!("{dir}/{name}")).unwrap();
        let m = aurora_assets::parse_llm(&d).unwrap();
        let mut mn = [f32::MAX; 3];
        let mut mx = [f32::MIN; 3];
        for p in &m.positions {
            for i in 0..3 {
                mn[i] = mn[i].min(p[i]);
                mx[i] = mx[i].max(p[i]);
            }
        }
        println!(
            "{name}: verts {} faces {} pos {:?} rot {:?} min {:?} max {:?} joints {:?} w0 {:?}",
            m.positions.len(),
            m.faces.len(),
            m.position,
            m.rotation_euler,
            mn,
            mx,
            &m.joint_names[..m.joint_names.len().min(6)],
            &m.weights[..3.min(m.weights.len())]
        );
    }
    let sk = aurora_assets::Skeleton::parse(&std::fs::read(format!("{dir}/avatar_skeleton.xml")).unwrap()).unwrap();
    for n in [
        "mPelvis",
        "mTorso",
        "mChest",
        "mNeck",
        "mHead",
        "mEyeLeft",
        "mShoulderLeft",
        "mHandLeft",
        "mFootLeft",
        "mHipLeft",
        "mSkull",
    ] {
        if let Some(i) = sk.find(n) {
            println!("{n}: {:?}", sk.joints[i].world_position());
        }
    }
}
