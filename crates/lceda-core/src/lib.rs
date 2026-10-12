//! 立创 / LCSC 器件下载核心库。
//!
//! 搜索、拉取 STEP/OBJ、解析 EasyEDA 源，并在进程内写出 Altium / KiCad / PADS 库。

pub mod altium;
pub mod ad_bridge;
pub mod ad_plugin;
pub mod bound_ad;
pub mod bound_kicad;
pub mod client;
pub mod desc;
pub mod easyeda;
pub mod error;
pub mod export;
pub mod ir;
pub mod kicad;
mod library_lock;
pub mod mesh;
pub mod managed_ad;
pub mod models;
pub mod native_library;
pub mod parallel;
pub mod supplement;
pub mod pads;
pub mod util;

pub use client::LcedaClient;
pub use error::{Error, Result};
pub use models::SearchItem;
