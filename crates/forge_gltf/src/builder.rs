//! Minimal GLB 2.0 byte-blob builder. Reused by both this crate's tests and
//! the engine's mesh-pipeline benches — no on-disk IO required.

/// Build a minimal GLB 2.0 byte blob from raw mesh data.
pub fn build_test_glb(
    positions: &[[f32; 3]],
    normals:   Option<&[[f32; 3]]>,
    uvs:       Option<&[[f32; 2]]>,
    indices:   Option<&[u32]>,
) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();

    fn append_f32s(bin: &mut Vec<u8>, floats: &[f32]) -> (usize, usize) {
        while bin.len() % 4 != 0 { bin.push(0); }
        let off = bin.len();
        for &f in floats { bin.extend_from_slice(&f.to_le_bytes()); }
        (off, bin.len() - off)
    }
    fn append_u32s(bin: &mut Vec<u8>, ints: &[u32]) -> (usize, usize) {
        while bin.len() % 4 != 0 { bin.push(0); }
        let off = bin.len();
        for &v in ints { bin.extend_from_slice(&v.to_le_bytes()); }
        (off, bin.len() - off)
    }

    let pos_flat: Vec<f32> = positions.iter().flat_map(|p| p.iter().copied()).collect();
    let (pos_off, pos_len) = append_f32s(&mut bin, &pos_flat);

    let norm_bv = normals.map(|ns| {
        let flat: Vec<f32> = ns.iter().flat_map(|n| n.iter().copied()).collect();
        append_f32s(&mut bin, &flat)
    });
    let uv_bv = uvs.map(|us| {
        let flat: Vec<f32> = us.iter().flat_map(|u| u.iter().copied()).collect();
        append_f32s(&mut bin, &flat)
    });
    let idx_bv = indices.map(|ids| append_u32s(&mut bin, ids));

    while bin.len() % 4 != 0 { bin.push(0); }
    let bin_total = bin.len();

    let mut bvs:  Vec<String> = Vec::new();
    let mut accs: Vec<String> = Vec::new();

    let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in positions {
        for i in 0..3 {
            mn[i] = mn[i].min(p[i]);
            mx[i] = mx[i].max(p[i]);
        }
    }
    bvs.push(format!(r#"{{"buffer":0,"byteOffset":{pos_off},"byteLength":{pos_len}}}"#));
    accs.push(format!(
        r#"{{"bufferView":0,"componentType":5126,"count":{cnt},"type":"VEC3","min":[{},{},{}],"max":[{},{},{}]}}"#,
        mn[0], mn[1], mn[2], mx[0], mx[1], mx[2], cnt = positions.len()
    ));

    let norm_acc: Option<usize> = norm_bv.map(|(off, len)| {
        let bv = bvs.len();
        bvs.push(format!(r#"{{"buffer":0,"byteOffset":{off},"byteLength":{len}}}"#));
        let ac = accs.len();
        accs.push(format!(r#"{{"bufferView":{bv},"componentType":5126,"count":{cnt},"type":"VEC3"}}"#,
            cnt = normals.unwrap().len()));
        ac
    });

    let uv_acc: Option<usize> = uv_bv.map(|(off, len)| {
        let bv = bvs.len();
        bvs.push(format!(r#"{{"buffer":0,"byteOffset":{off},"byteLength":{len}}}"#));
        let ac = accs.len();
        accs.push(format!(r#"{{"bufferView":{bv},"componentType":5126,"count":{cnt},"type":"VEC2"}}"#,
            cnt = uvs.unwrap().len()));
        ac
    });

    let idx_acc: Option<usize> = idx_bv.map(|(off, len)| {
        let bv = bvs.len();
        bvs.push(format!(r#"{{"buffer":0,"byteOffset":{off},"byteLength":{len}}}"#));
        let ac = accs.len();
        accs.push(format!(r#"{{"bufferView":{bv},"componentType":5125,"count":{cnt},"type":"SCALAR"}}"#,
            cnt = indices.unwrap().len()));
        ac
    });

    let mut attrs = String::from(r#""POSITION":0"#);
    if let Some(i) = norm_acc { attrs.push_str(&format!(r#","NORMAL":{i}"#)); }
    if let Some(i) = uv_acc   { attrs.push_str(&format!(r#","TEXCOORD_0":{i}"#)); }
    let prim_idx = match idx_acc {
        Some(i) => format!(r#","indices":{i}"#),
        None    => String::new(),
    };

    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"meshes":[{{"primitives":[{{"attributes":{{{attrs}}}{prim_idx}}}]}}],"accessors":[{accs}],"bufferViews":[{bvs}],"buffers":[{{"byteLength":{bin_total}}}]}}"#,
        accs = accs.join(","), bvs = bvs.join(",")
    );
    let mut json_bytes = json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }

    let total = 12 + 8 + json_bytes.len() + 8 + bin_total;
    let mut glb: Vec<u8> = Vec::with_capacity(total);
    let p = |g: &mut Vec<u8>, v: u32| g.extend_from_slice(&v.to_le_bytes());
    p(&mut glb, 0x46546C67);
    p(&mut glb, 2);
    p(&mut glb, total as u32);
    p(&mut glb, json_bytes.len() as u32);
    p(&mut glb, 0x4E4F534A);
    glb.extend_from_slice(&json_bytes);
    p(&mut glb, bin_total as u32);
    p(&mut glb, 0x004E4942);
    glb.extend_from_slice(&bin);
    glb
}

/// Build a GLB blob with **no** meshes — useful for "empty document" tests.
pub fn build_empty_glb() -> Vec<u8> {
    let json = br#"{"asset":{"version":"2.0"}}"#;
    let mut jp = json.to_vec();
    while jp.len() % 4 != 0 { jp.push(b' '); }
    let total = (12 + 8 + jp.len()) as u32;
    let mut glb: Vec<u8> = Vec::new();
    for v in [0x46546C67u32, 2, total, jp.len() as u32, 0x4E4F534A] {
        glb.extend_from_slice(&v.to_le_bytes());
    }
    glb.extend_from_slice(&jp);
    glb
}
