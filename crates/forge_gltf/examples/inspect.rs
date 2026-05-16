use forge_gltf::*;
fn main() {
    for name in &["BrainStem.glb", "AnimatedColorsCube.glb", "DiffuseTransmissionPlant.glb", "TransmissionTest.glb", "ScatteringSkull.glb"] {
        let path = std::path::Path::new("assets/models").join(name);
        match GltfAsset::load(&path) {
            Ok(a) => {
                println!("\n=== {name} ===");
                println!("  nodes={} meshes={} skins={} anims={} materials={} lights={}",
                    a.nodes.len(), a.meshes.len(), a.skins.len(), a.animations.len(),
                    a.materials.len(), a.lights.len());
                if let Some(anim) = a.animations.first() {
                    let target_nodes: Vec<_> = anim.channels.iter().map(|c| c.target_node).collect();
                    let mesh_nodes: Vec<usize> = a.nodes.iter().enumerate().filter_map(|(i,n)| n.mesh.map(|_| i)).collect();
                    println!("  anim '{}' dur={:.2}s, targets {}", anim.name.as_deref().unwrap_or(""), anim.duration(), target_nodes.len());
                    println!("  mesh-bearing nodes: {mesh_nodes:?}");
                    let common: Vec<u32> = target_nodes.iter().filter(|t| mesh_nodes.contains(&(**t as usize))).copied().collect();
                    println!("  animation overlaps mesh-bearing nodes: {common:?}");
                }
                for (i, m) in a.materials.iter().enumerate().take(5) {
                    println!("  mat{i} '{}': transmission={} thickness={} attn={} ior={} emissive_strength={}",
                        m.name.as_deref().unwrap_or(""),
                        m.transmission.factor,
                        m.volume.thickness_factor,
                        m.volume.attenuation_distance,
                        m.ior,
                        m.emissive_strength);
                }
            }
            Err(e) => println!("{name}: FAIL {e:?}"),
        }
    }
}
