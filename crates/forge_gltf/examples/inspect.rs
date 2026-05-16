use forge_gltf::*;

fn main() {
    for name in &[
        "BrainStem.glb",
        "AnimatedColorsCube.glb",
        "DiffuseTransmissionPlant.glb",
        "TransmissionTest.glb",
        "ScatteringSkull.glb",
        "BoomBox.glb",
        "ToyCar.glb",
        "AnimationPointerUVs.glb",
    ] {
        let path = std::path::Path::new("assets/models").join(name);
        println!("\n=== {name} ===");
        match GltfAsset::load(&path) {
            Ok(a) => {
                println!(
                    "  nodes={} meshes={} skins={} anims={} materials={} lights={}",
                    a.nodes.len(), a.meshes.len(), a.skins.len(),
                    a.animations.len(), a.materials.len(), a.lights.len()
                );
                for (i, anim) in a.animations.iter().enumerate() {
                    println!(
                        "  anim{i} '{}' dur={:.2}s channels={} pointer_channels={}",
                        anim.name.as_deref().unwrap_or(""),
                        anim.duration(),
                        anim.channels.len(),
                        anim.pointer_channels.len(),
                    );
                    for pc in anim.pointer_channels.iter().take(3) {
                        println!("    pointer: {} (sampler {})", pc.pointer, pc.sampler);
                    }
                }
                for (i, m) in a.materials.iter().enumerate().take(4) {
                    let exts: Vec<&str> = [
                        ("clearcoat",   m.clearcoat.is_some()),
                        ("sheen",       m.sheen.is_some()),
                        ("specular",    m.specular.is_some()),
                        ("iridescence", m.iridescence.is_some()),
                        ("anisotropy",  m.anisotropy.is_some()),
                        ("diff_trans",  m.diffuse_transmission.is_some()),
                        ("dispersion",  m.dispersion > 0.0),
                    ].iter().filter(|(_, on)| *on).map(|(n, _)| *n).collect();
                    println!(
                        "  mat{i} '{}': trans={:.2} thick={:.2} ior={} exts={exts:?}",
                        m.name.as_deref().unwrap_or(""),
                        m.transmission.factor,
                        m.volume.thickness_factor,
                        m.ior,
                    );
                }
            }
            Err(e) => println!("  LOAD FAIL: {e:?}"),
        }
    }
}
