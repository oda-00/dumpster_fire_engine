use std::path::PathBuf;
use thin_vec::ThinVec;
use std::sync::Arc;

use super::ingot::Ingot;
use super::master::{ForgeMaster, ForgeResult};
use super::ore::{IngotSpec, Ore, OreKind};

pub struct FramePlan {
    pub name: Arc<str>,
    pub ores: ThinVec<Ore>,
}

impl FramePlan {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            ores: ThinVec::new(),
        }
    }

    pub fn push(&mut self, ore: Ore) {
        self.ores.push(ore);
    }

    pub fn refine(self, master: &mut ForgeMaster) -> ForgeResult<Frame> {
        let mut frame = Frame::new(self.name);
        for ore in self.ores {
            frame.ingots.push(master.refine(ore)?);
        }
        Ok(frame)
    }
}

pub struct Frame {
    pub name: Arc<str>,
    pub ingots: ThinVec<Ingot>,
}

impl Frame {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            ingots: ThinVec::new(),
        }
    }

    pub fn add_ingot(&mut self, ingot: Ingot) {
        self.ingots.push(ingot);
    }

    pub fn manifest(&self) -> FrameManifest {
        FrameManifest {
            name: self.name.clone(),
            entries: self
                .ingots
                .iter()
                .map(|ingot| FrameEntry {
                    kind: ingot.kind,
                    byte_len: ingot.as_bytes().len() as u64,
                    save_path: ingot.save_path.clone(),
                })
                .collect(),
        }
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for ingot in &mut self.ingots {
            unsafe { ingot.destroy(device) };
        }
        self.ingots.clear();
    }
}

#[derive(Debug, Clone)]
pub struct FrameManifest {
    pub name: Arc<str>,
    pub entries: ThinVec<FrameEntry>,
}

#[derive(Debug, Clone)]
pub struct  FrameEntry {
    pub kind: OreKind,
    pub byte_len: u64,
    pub save_path: Option<PathBuf>,
}

pub fn ore_for_buffer(
    kind: OreKind,
    bytes: ThinVec<u8>,
    output_size: ash::vk::DeviceSize,
    workgroups: [u32; 3],
) -> Ore {
    Ore::new(
        kind,
        super::ore::OreInput::Bytes(bytes.to_vec()  ),
        IngotSpec::Buffer {
            size: output_size,
            save_path: None,
        },
        workgroups,
    )
}
