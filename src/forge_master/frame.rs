use std::path::PathBuf;

use super::ingot::Ingot;
use super::master::{ForgeMaster, ForgeResult};
use super::ore::{IngotSpec, Ore, OreKind};

pub struct ForgeFramePlan {
    pub name: String,
    pub ores: Vec<Ore>,
}

impl ForgeFramePlan {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ores: Vec::new(),
        }
    }

    pub fn push(&mut self, ore: Ore) {
        self.ores.push(ore);
    }

    pub fn refine(self, master: &mut ForgeMaster) -> ForgeResult<ForgeFrame> {
        let mut frame = ForgeFrame::new(self.name);
        for ore in self.ores {
            frame.ingots.push(master.refine(ore)?);
        }
        Ok(frame)
    }
}

pub struct ForgeFrame {
    pub name: String,
    pub ingots: Vec<Ingot>,
}

impl ForgeFrame {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ingots: Vec::new(),
        }
    }

    pub fn add_ingot(&mut self, ingot: Ingot) {
        self.ingots.push(ingot);
    }

    pub fn manifest(&self) -> ForgeFrameManifest {
        ForgeFrameManifest {
            name: self.name.clone(),
            entries: self
                .ingots
                .iter()
                .map(|ingot| ForgeFrameEntry {
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
pub struct ForgeFrameManifest {
    pub name: String,
    pub entries: Vec<ForgeFrameEntry>,
}

#[derive(Debug, Clone)]
pub struct ForgeFrameEntry {
    pub kind: OreKind,
    pub byte_len: u64,
    pub save_path: Option<PathBuf>,
}

pub fn ore_for_buffer(
    kind: OreKind,
    bytes: Vec<u8>,
    output_size: ash::vk::DeviceSize,
    workgroups: [u32; 3],
) -> Ore {
    Ore::new(
        kind,
        super::ore::OreInput::Bytes(bytes),
        IngotSpec::Buffer {
            size: output_size,
            save_path: None,
        },
        workgroups,
    )
}
