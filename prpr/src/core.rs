//! Core module for prpr, submodules:
//!   - [crate::core::anim]
//!   - [crate::core::chart]
//!   - [crate::core::effect]
//!   - [crate::core::line]
//!   - [crate::core::note]
//!   - [crate::core::object]
//!   - [crate::core::render]
//!   - [crate::core::resource]
//!   - [crate::core::smooth]
//!   - [crate::core::tween]

pub use macroquad::color::Color;

pub const NOTE_WIDTH_RATIO_BASE: f64 = 0.13175016;
pub const HEIGHT_RATIO: f64 = 0.83175;

pub const EPS: f64 = 1e-5;

pub type Point = nalgebra::Point2<f32>;
pub type Vector = nalgebra::Vector2<f32>;
pub type Matrix = nalgebra::Matrix3<f32>;

mod anim;
pub use anim::{Anim, AnimFloat, AnimVector, Keyframe};

mod chart;
pub use chart::{Chart, ChartExtra, ChartSettings, HitSoundMap};

mod effect;
pub use effect::{Effect, Uniform};

mod line;
pub use line::{GifFrames, JudgeLine, JudgeLineCache, JudgeLineKind, UIElement};

mod note;
use macroquad::prelude::set_pc_assets_folder;
pub use note::{BadNote, HitSound, Note, NoteKind, RenderConfig};

mod object;
pub use object::{CtrlObject, Object};

mod render;
pub use render::{copy_fbo, internal_id, MSRenderTarget};

mod resource;
pub use resource::{NoteStyle, ParticleEmitter, ResPackInfo, Resource, ResourcePack, BUFFER_SIZE, DPI_VALUE, MUSIC_STREAMING, LAST_NOTE_VERTS};

/// 低端优化：整条判定线（含可达范围）都在屏幕外时跳过渲染。由设置控制，默认关。
pub static LINE_CULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 谱面渲染时是否把 model 烘焙进顶点以减少 draw call（部分设备/材质下可能显示异常，故可关）。
pub static BAKE_MODEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 流式谱面音乐的磁盘 PCM 缓存（首次解码落盘，之后直读）。
pub static DISK_CACHE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 磁盘缓存的目录（由宿主设置；None 表示不可用）。
pub static MUSIC_CACHE_DIR: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// 本帧判定线：实际渲染数 / 总数（诊断用）。
pub static LINES_RENDERED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static LINES_TOTAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// 判定线明细：音符循环累计微秒（诊断用）。
pub static LINE_NOTES_US: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

mod smooth;
pub use smooth::Smooth;

mod tween;
pub use tween::{
    easing_from, BezierTween, ClampedTween, GeneralIntTween, IntClampedTween, IntStaticTween, StaticTween, TweenFunction, TweenId, TweenMajor,
    TweenMinor, Tweenable, TWEEN_FUNCTIONS,
};

#[cfg(feature = "video")]
mod video;
#[cfg(feature = "video")]
pub use prpr_avc::demux_audio;
#[cfg(feature = "video")]
pub use video::{Video, VideoAttach};

use crate::ui::TextPainter;
use std::cell::RefCell;

thread_local! {
    pub static PGR_FONT: RefCell<Option<TextPainter>> = RefCell::default();
    pub static BOLD_FONT: RefCell<Option<TextPainter>> = RefCell::default();
}

pub fn init_assets() {
    #[cfg(not(target_env = "ohos"))]
    if let Ok(mut exe) = std::env::current_exe() {
        while exe.pop() {
            if exe.join("assets").exists() {
                std::env::set_current_dir(exe).unwrap();
                break;
            }
        }
    }
    #[cfg(target_env = "ohos")]
    let _ = std::env::set_current_dir("/data/storage/el1/bundle/entry/resources/resfile/");
    set_pc_assets_folder("assets");
}

#[derive(serde::Deserialize)]
/// `(i, n, d)`: `i + n / d`
pub struct Triple(i32, u32, u32);
impl Default for Triple {
    fn default() -> Self {
        Self(0, 0, 1)
    }
}

impl Triple {
    pub fn beats(&self) -> f64 {
        self.0 as f64 + self.1 as f64 / self.2 as f64
    }
}

#[derive(Default)] // the default is a dummy
pub struct BpmList {
    /// (beats, time, bpm)
    /// time in seconds
    elements: Vec<(f64, f64, f64)>,
    /// cursor for searching, value is the index of `elements`
    cursor: usize,
}

impl BpmList {
    /// Create a new BpmList from a list of (beats, bpm) pairs
    ///
    /// Basically just calculate the time for each pair(key frame)
    pub fn new(ranges: Vec<(f64, f64)>) -> Self {
        let mut elements = Vec::new();
        let mut time = 0.0;
        let mut last_beats = 0.0;
        let mut last_bpm: Option<f64> = None;
        for (now_beats, bpm) in ranges {
            if let Some(bpm) = last_bpm {
                time += (now_beats - last_beats) * (60. / bpm);
            }
            last_beats = now_beats;
            last_bpm = Some(bpm);
            elements.push((now_beats, time, bpm));
        }
        BpmList { elements, cursor: 0 }
    }

    /// Get the time in seconds for a given beats
    pub fn time_beats(&mut self, beats: f64) -> f64 {
        while let Some(kf) = self.elements.get(self.cursor + 1) {
            if kf.0 > beats {
                break;
            }
            self.cursor += 1;
        }
        while self.cursor != 0 && self.elements[self.cursor].0 > beats {
            self.cursor -= 1;
        }
        let (start_beats, time, bpm) = &self.elements[self.cursor];
        time + (beats - start_beats) * (60. / bpm)
    }

    /// Get the time in seconds for a given `i + n / d`
    pub fn time(&mut self, triple: &Triple) -> f64 {
        self.time_beats(triple.beats())
    }

    /// Get the beat coordinate for a given time in seconds
    pub fn beat(&mut self, time: f64) -> f64 {
        while let Some(kf) = self.elements.get(self.cursor + 1) {
            if kf.1 > time {
                break;
            }
            self.cursor += 1;
        }
        while self.cursor != 0 && self.elements[self.cursor].1 > time {
            self.cursor -= 1;
        }
        let (beats, start_time, bpm) = &self.elements[self.cursor];
        beats + (time - start_time) / (60. / bpm)
    }
}
