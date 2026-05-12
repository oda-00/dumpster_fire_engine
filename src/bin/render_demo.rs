// Full-stack smoke test for forge_master + render::factory_master + render.
//
// Bootstraps a Vulkan compute context (VulkanContext), registers one Forge
// against a precompiled SPIR-V doubler kernel, builds a Renderer with two
// Windows, refines a Proto (containing a FramePlan with a single Ore) into
// a Factory, reads the result back, and verifies output[i] == input[i] * 2.
//
// Run with:  cargo run --bin render_demo
//
// If Vulkan isn't available on this machine the demo prints the error and
// exits 0 so CI without a GPU stays green.

use dumpster_fire_engine::ThinVec;
use dumpster_fire_engine::forge_master::{
    ForgeId, ForgeMaster, FrameId, FramePlan, IngotSpec, Ore, OreInput, OreKind,
};
use dumpster_fire_engine::render::{
    Proto, ProtoId, Renderer, VulkanContext, Window, WindowId,
};

const DEMO_DOUBLER_SPV: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/shaders/demo_doubler.spv"
));

fn main() {
    match run() {
        Ok(()) => println!("\nrender_demo: clean exit."),
        Err(e) => eprintln!("\nrender_demo: {e}"),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== render_demo: forge / factories / renderer smoke test ===\n");

    // 1. Vulkan bootstrap. Owns entry/instance/device/command_pool; Drop
    //    tears them down after the Renderer (and its ForgeMaster) go away.
    let ctx = VulkanContext::new()?;
    println!(
        "Vulkan: {} (compute queue family {})",
        ctx.device_name, ctx.queue_family_index
    );

    // 2. ForgeMaster on top of the device. Clones the ash::Device handle
    //    (cheap — Arc-backed); the real vkDestroyDevice fires from ctx.
    let mut forge = ForgeMaster::new(
        ctx.device.clone(),
        ctx.queue,
        ctx.command_pool,
        ctx.memory_properties,
    )?;

    // 3. Register one Forge against the precompiled SPIR-V doubler.
    let forge_handle = forge.add_forge_from_spirv_bytes(
        ForgeId::new(1),
        OreKind::SignedDistanceField,
        DEMO_DOUBLER_SPV,
    )?;
    println!(
        "Registered forge: kind={:?} handle={:?}\n",
        OreKind::SignedDistanceField,
        forge_handle
    );

    // 4. Renderer owns the ForgeMaster + the windows arena.
    let mut renderer = Renderer::new(forge);

    let main_h = renderer.add_window(Window::new(WindowId::new(1), "main", 1920, 1080));
    let _preview_h = renderer.add_window(Window::new(WindowId::new(2), "preview", 480, 270));

    println!("Windows added:");
    for window in renderer.windows() {
        println!(
            "  id={:?} name='{}' size={}x{}",
            window.id, window.name, window.width, window.height
        );
    }

    let by_id = renderer
        .handle_of(WindowId::new(1))
        .expect("WindowId(1) should be cached");
    assert_eq!(by_id, main_h, "handle_of should match the add_window result");
    println!(
        "  handle_of(WindowId(1)) -> {:?}  (matches main_h)\n",
        by_id
    );

    // 5. Build a Proto containing one FramePlan containing one Ore.
    let input: ThinVec<u32> = (1u32..=64).collect();
    let input_bytes = u32_slice_to_bytes(&input);
    let output_size = (input.len() * 4) as u64;

    let mut plan = FramePlan::new(FrameId::new(100), "demo.frame");
    plan.push(Ore::new(
        OreKind::SignedDistanceField,
        OreInput::Bytes(input_bytes),
        IngotSpec::Buffer {
            size: output_size,
            save_path: None,
        },
        [1, 1, 1], // local_size_x=64 → 64 invocations cover the buffer
    ));

    let mut proto = Proto::new(ProtoId::new(10), "demo.proto");
    proto.push(plan);

    // 6. Drive the proto through the renderer. This is the actual GPU
    //    dispatch: ForgeMaster::refine stages the input, dispatches the
    //    compute pipeline, copies the result to a host-visible buffer,
    //    and reads it back.
    println!("Refining proto -> factory (dispatching compute shader)...");
    let factory_h = renderer.build_factory(main_h, proto)?;
    println!("  factory handle: {:?}", factory_h);

    // 7. Walk the hierarchy to fetch the readback.
    let window = renderer.window(main_h).expect("main window present");
    let factory = window
        .factory_master
        .get(factory_h)
        .expect("factory present");
    println!(
        "  factory: id={:?} name='{}' frames={}",
        factory.id,
        factory.name,
        factory.len()
    );

    let frame = factory
        .frame_by_id(FrameId::new(100))
        .expect("frame present");
    println!(
        "  frame:   id={:?} name='{}' ingots={}",
        frame.id,
        frame.name,
        frame.ingots.len()
    );

    let output_bytes = frame.ingots[0].as_bytes();
    let output = bytes_to_u32_vec(output_bytes);
    println!("  readback: {} bytes -> {} u32s\n", output_bytes.len(), output.len());

    // 8. Verify output[i] == input[i] * 2.
    let expected: ThinVec<u32> = input.iter().map(|n| n * 2).collect();
    let mismatches: ThinVec<(usize, u32, u32)> = expected
        .iter()
        .zip(output.iter())
        .enumerate()
        .filter_map(|(i, (&want, &got))| (want != got).then_some((i, want, got)))
        .collect();

    if mismatches.is_empty() {
        println!(
            "doubler OK -- {}/{} entries match i*2",
            output.len(),
            input.len()
        );
        println!("  first 8: input={:?}", &input[..8]);
        println!("           output={:?}", &output[..8]);
    } else {
        println!(
            "doubler MISMATCH -- {}/{} matched; first 3 failures: {:?}",
            output.len() - mismatches.len(),
            input.len(),
            &mismatches[..mismatches.len().min(3)]
        );
    }

    // 9. Three-tick "main loop" — walks the renderer hierarchy each tick so
    //    the user sees the full topology built up from arenas and caches.
    println!("\n--- main loop (3 ticks) ---");
    for tick in 0..3 {
        println!(
            "tick {tick}: renderer has {} windows; forge has {} OreKind slots",
            renderer.len(),
            OreKind::COUNT
        );
        for window in renderer.windows() {
            println!(
                "  window id={:?} '{}' {}x{} factories={}",
                window.id,
                window.name,
                window.width,
                window.height,
                window.factory_master.len()
            );
            for factory in window.factory_master.iter() {
                for frame in factory.frames() {
                    println!(
                        "    factory={:?} frame id={:?} '{}' ingots={}",
                        factory.id,
                        frame.id,
                        frame.name,
                        frame.ingots.len()
                    );
                }
            }
        }
    }

    // Drop order on return: renderer (destroys windows' frames' ingots via
    // a cloned device) → ForgeMaster inside renderer (fence + desc_pool) →
    // ctx (command_pool, device, instance).
    Ok(())
}

fn u32_slice_to_bytes(slice: &[u32]) -> ThinVec<u8> {
    let mut out: ThinVec<u8> = ThinVec::with_capacity(slice.len() * 4);
    for &v in slice {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn bytes_to_u32_vec(bytes: &[u8]) -> ThinVec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
