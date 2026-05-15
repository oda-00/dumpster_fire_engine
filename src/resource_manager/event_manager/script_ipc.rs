//! Wire-format codec for `langcd` ↔ engine IPC.
//!
//! Length-prefixed, little-endian.  Strings = `u32 len + utf-8 bytes`.
//! All payloads land in / out of a reused `ThinVec<u8>` to keep the steady-state
//! allocation-free.
//!
//! Single source of truth for the wire protocol — `langcd` includes this file
//! via `#[path = ...]`.  Each side uses only half the codec (the daemon never
//! reads daemon messages, the engine never writes engine messages), hence the
//! crate-level `#[allow(dead_code)]`.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::sync::Arc;
use thin_vec::ThinVec;

// ── Tag bytes ────────────────────────────────────────────────────────────────

pub const TAG_WATCH:     u8 = 1;
pub const TAG_UNWATCH:   u8 = 2;
pub const TAG_SHUTDOWN:  u8 = 3;
pub const TAG_COMPILE_OK:  u8 = 4;
pub const TAG_COMPILE_ERR: u8 = 5;

// ── Messages ─────────────────────────────────────────────────────────────────

pub enum EngineMsg {
    Watch    { script_id: i64, path: Arc<str> },
    Unwatch  { script_id: i64 },
    Shutdown,
}

pub enum DaemonMsg {
    CompileOk  { script_id: i64, o_path: Arc<str>, state_size: u32, state_version: u32 },
    CompileErr { script_id: i64, diagnostics: ThinVec<Arc<str>> },
}

// ── Encoding ─────────────────────────────────────────────────────────────────

pub fn write_engine_msg<W: Write>(w: &mut W, m: &EngineMsg) -> std::io::Result<()> {
    let mut buf: ThinVec<u8> = ThinVec::new();
    match m {
        EngineMsg::Watch { script_id, path } => {
            buf.push(TAG_WATCH);
            buf.extend_from_slice(&script_id.to_le_bytes());
            put_str(&mut buf, path);
        }
        EngineMsg::Unwatch { script_id } => {
            buf.push(TAG_UNWATCH);
            buf.extend_from_slice(&script_id.to_le_bytes());
        }
        EngineMsg::Shutdown => buf.push(TAG_SHUTDOWN),
    }
    let len = buf.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&buf)?;
    Ok(())
}

pub fn write_daemon_msg<W: Write>(w: &mut W, m: &DaemonMsg) -> std::io::Result<()> {
    let mut buf: ThinVec<u8> = ThinVec::new();
    match m {
        DaemonMsg::CompileOk { script_id, o_path, state_size, state_version } => {
            buf.push(TAG_COMPILE_OK);
            buf.extend_from_slice(&script_id.to_le_bytes());
            put_str(&mut buf, o_path);
            buf.extend_from_slice(&state_size.to_le_bytes());
            buf.extend_from_slice(&state_version.to_le_bytes());
        }
        DaemonMsg::CompileErr { script_id, diagnostics } => {
            buf.push(TAG_COMPILE_ERR);
            buf.extend_from_slice(&script_id.to_le_bytes());
            buf.extend_from_slice(&(diagnostics.len() as u32).to_le_bytes());
            for d in diagnostics { put_str(&mut buf, d); }
        }
    }
    let len = buf.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&buf)?;
    Ok(())
}

pub fn read_engine_msg<R: Read>(r: &mut R) -> std::io::Result<EngineMsg> {
    let buf = read_frame(r)?;
    let mut c = Cursor { buf: &buf, pos: 0 };
    let tag = c.u8()?;
    match tag {
        TAG_WATCH => Ok(EngineMsg::Watch {
            script_id: c.i64()?,
            path:      c.string()?,
        }),
        TAG_UNWATCH => Ok(EngineMsg::Unwatch { script_id: c.i64()? }),
        TAG_SHUTDOWN => Ok(EngineMsg::Shutdown),
        _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("unknown tag {tag}"))),
    }
}

pub fn read_daemon_msg<R: Read>(r: &mut R) -> std::io::Result<DaemonMsg> {
    let buf = read_frame(r)?;
    let mut c = Cursor { buf: &buf, pos: 0 };
    let tag = c.u8()?;
    match tag {
        TAG_COMPILE_OK => Ok(DaemonMsg::CompileOk {
            script_id:     c.i64()?,
            o_path:       c.string()?,
            state_size:    c.u32()?,
            state_version: c.u32()?,
        }),
        TAG_COMPILE_ERR => {
            let script_id = c.i64()?;
            let n = c.u32()? as usize;
            let mut diags = ThinVec::with_capacity(n);
            for _ in 0..n { diags.push(c.string()?); }
            Ok(DaemonMsg::CompileErr { script_id, diagnostics: diags })
        }
        _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("unknown tag {tag}"))),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn put_str(buf: &mut ThinVec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

fn read_frame<R: Read>(r: &mut R) -> std::io::Result<ThinVec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let n = u32::from_le_bytes(len_buf) as usize;
    let mut buf: ThinVec<u8> = ThinVec::with_capacity(n);
    buf.resize(n, 0);
    r.read_exact(&mut buf[..])?;
    Ok(buf)
}

struct Cursor<'a> { buf: &'a [u8], pos: usize }
impl<'a> Cursor<'a> {
    fn u8(&mut self)  -> std::io::Result<u8>  {
        let v = *self.buf.get(self.pos).ok_or_else(short)?;
        self.pos += 1; Ok(v)
    }
    fn u32(&mut self) -> std::io::Result<u32> {
        let s = self.pos; self.pos += 4;
        let bs = self.buf.get(s..s+4).ok_or_else(short)?;
        Ok(u32::from_le_bytes(bs.try_into().unwrap()))
    }
    fn i64(&mut self) -> std::io::Result<i64> {
        let s = self.pos; self.pos += 8;
        let bs = self.buf.get(s..s+8).ok_or_else(short)?;
        Ok(i64::from_le_bytes(bs.try_into().unwrap()))
    }
    fn string(&mut self) -> std::io::Result<Arc<str>> {
        let n = self.u32()? as usize;
        let s = self.pos; self.pos += n;
        let bs = self.buf.get(s..s+n).ok_or_else(short)?;
        Ok(Arc::from(core::str::from_utf8(bs).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 string")
        })?))
    }
}

fn short() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short read")
}
