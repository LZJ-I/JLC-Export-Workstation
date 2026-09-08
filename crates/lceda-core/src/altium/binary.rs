//! Altium 二进制块编码（参数块 / Pascal 短串 / 坐标）。
//! 布局对齐 OriginalCircuit.AltiumSharp 的 BinaryFormatWriter。

use crate::error::{Error, Result};
use encoding_rs::WINDOWS_1252;
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::Path;

pub struct CfbDoc {
    inner: cfb::CompoundFile<File>,
}

impl CfbDoc {
    pub fn create(path: &Path) -> Result<Self> {
        crate::util::ensure_parent(path)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let file = File::create(path)?;
        let inner = cfb::CompoundFile::create(file).map_err(|e| Error::Altium(e.to_string()))?;
        Ok(Self { inner })
    }

    pub fn storage(&mut self, path: &str) -> Result<()> {
        self.inner
            .create_storage(path)
            .map_err(|e| Error::Altium(e.to_string()))
    }

    pub fn stream(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let mut s = self
            .inner
            .create_stream(path)
            .map_err(|e| Error::Altium(e.to_string()))?;
        s.write_all(data)?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.inner.flush().map_err(|e| Error::Altium(e.to_string()))
    }
}

pub struct BinWriter {
    inner: Cursor<Vec<u8>>,
}

impl BinWriter {
    pub fn new() -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
        }
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.inner.into_inner()
    }

    pub fn write_u8(&mut self, v: u8) {
        self.inner.write_all(&[v]).unwrap();
    }
    pub fn write_i16(&mut self, v: i16) {
        self.inner.write_all(&v.to_le_bytes()).unwrap();
    }
    pub fn write_u16(&mut self, v: u16) {
        self.inner.write_all(&v.to_le_bytes()).unwrap();
    }
    pub fn write_i32(&mut self, v: i32) {
        self.inner.write_all(&v.to_le_bytes()).unwrap();
    }
    pub fn write_u32(&mut self, v: u32) {
        self.inner.write_all(&v.to_le_bytes()).unwrap();
    }
    pub fn write_f64(&mut self, v: f64) {
        self.inner.write_all(&v.to_le_bytes()).unwrap();
    }
    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(if v { 1 } else { 0 });
    }
    pub fn write_bytes(&mut self, data: &[u8]) {
        self.inner.write_all(data).unwrap();
    }

    pub fn write_coord(&mut self, raw: i32) {
        self.write_i32(raw);
    }

    pub fn write_coord_point(&mut self, x: i32, y: i32) {
        self.write_coord(x);
        self.write_coord(y);
    }

    pub fn write_pascal_short(&mut self, s: &str) {
        let (bytes, _, _) = WINDOWS_1252.encode(s);
        let len = bytes.len().min(255);
        self.write_u8(len as u8);
        self.write_bytes(&bytes[..len]);
    }

    pub fn write_cstring(&mut self, s: &str) {
        let (bytes, _, _) = WINDOWS_1252.encode(s);
        self.write_bytes(&bytes);
        self.write_u8(0);
    }

    pub fn write_block<F: FnOnce(&mut BinWriter)>(&mut self, flags: u8, f: F) {
        let start = self.inner.position();
        self.write_i32(0);
        f(self);
        let end = self.inner.position();
        let length = (end - start - 4) as i32;
        let size = ((flags as i32) << 24) | (length & 0x00FF_FFFF);
        let buf = self.inner.get_mut();
        buf[start as usize..start as usize + 4].copy_from_slice(&size.to_le_bytes());
    }

    /// Length-prefixed block with raw payload. Pad SubRecord 2/4 must be `[0]`, not empty.
    pub fn write_block_raw(&mut self, flags: u8, data: &[u8]) {
        let size = ((flags as i32) << 24) | (data.len() as i32 & 0x00FF_FFFF);
        self.write_i32(size);
        self.write_bytes(data);
    }

    pub fn write_string_block(&mut self, s: &str) {
        let owned = s.to_string();
        self.write_block(0, |w| w.write_pascal_short(&owned));
    }

    pub fn write_params(&mut self, pairs: &[(&str, String)]) {
        let mut s = String::new();
        for (k, v) in pairs {
            s.push('|');
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        self.write_block(0, |w| w.write_cstring(&s));
    }

    pub fn write_params_raw(&mut self, raw: &str) {
        let owned = raw.to_string();
        self.write_block(0, |w| w.write_cstring(&owned));
    }

    /// UTF-16LE 参数块：`int32 字节数` + `|KEY=VAL|…` + 结尾 NUL。
    /// 与 AltiumSharp `WriteUnicodeParameterBlock` / `PinSymbolLineWidth` 内层一致。
    #[allow(dead_code)]
    pub fn write_unicode_params(&mut self, pairs: &[(&str, String)]) {
        let mut s = String::new();
        for (k, v) in pairs {
            s.push('|');
            s.push_str(k);
            s.push('=');
            s.push_str(v);
        }
        let mut utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        utf16.extend_from_slice(&[0, 0]);
        self.write_i32(utf16.len() as i32);
        self.write_bytes(&utf16);
    }

    /// OLE 压缩条目：`0xD0` + Pascal 名 + zlib 载荷。用于 PinFrac / PinTextData 等。
    #[allow(dead_code)]
    pub fn write_compressed_named(&mut self, name: &str, payload: &[u8]) {
        let compressed = zlib_compress(payload);
        let (name_bytes, _, _) = WINDOWS_1252.encode(name);
        let name_len = name_bytes.len().min(255);
        let block_size = 1 + 1 + name_len + 4 + compressed.len();
        self.write_i32((block_size as i32) | 0x0100_0000);
        self.write_u8(0xD0);
        self.write_u8(name_len as u8);
        self.write_bytes(&name_bytes[..name_len]);
        self.write_i32(compressed.len() as i32);
        self.write_bytes(&compressed);
    }
}

#[allow(dead_code)]
pub fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).expect("zlib compress");
    enc.finish().expect("zlib finish")
}

impl Default for BinWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// 1 mil = 10_000 raw；1 mm ≈ 39370.0787 raw
pub fn from_mm(mm: f64) -> i32 {
    (mm * 10_000.0 / 0.0254).round() as i32
}

pub fn from_mils(mils: f64) -> i32 {
    (mils * 10_000.0).round() as i32
}

/// DXP 单位：1 DXP = 10 mil = 100_000 raw
pub fn add_coord_param(pairs: &mut Vec<(String, String)>, name: &str, mm: f64) {
    let raw = from_mm(mm);
    let dxp = raw / 100_000;
    let frac = raw % 100_000;
    if dxp != 0 {
        pairs.push((name.to_string(), dxp.to_string()));
    }
    if frac != 0 {
        pairs.push((format!("{name}_Frac"), frac.to_string()));
    }
}

pub fn schematic_units(mm: f64) -> i32 {
    from_mm(mm) / 1000
}

pub fn v7_layer_id(layer: u8) -> u32 {
    let layer = layer as u32;
    if layer == 32 {
        return 0x0100_FFFF;
    }
    if (1..=31).contains(&layer) {
        return 0x0100_0000 + layer;
    }
    if (39..=54).contains(&layer) {
        return 0x0101_0000 + (layer - 38);
    }
    if (57..=72).contains(&layer) {
        return 0x0102_0000 + (layer - 56);
    }
    match layer {
        33 => 0x0103_0006,
        34 => 0x0103_0007,
        35 => 0x0103_0008,
        36 => 0x0103_0009,
        37 => 0x0103_000A,
        38 => 0x0103_000B,
        55 => 0x0103_000C,
        56 => 0x0103_000D,
        73 => 0x0103_000E,
        74 => 0x0103_000F,
        _ => 0x0103_000F,
    }
}

pub const FLAG_UNLOCKED: u16 = 0x04;
pub const FLAG_SAVED: u16 = 0x08;

pub fn encode_flags() -> u16 {
    FLAG_UNLOCKED | FLAG_SAVED
}
