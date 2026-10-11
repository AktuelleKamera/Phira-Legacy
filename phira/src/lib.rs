prpr_l10n::tl_file!("common" ttl crate::);

#[rustfmt::skip]
#[cfg(closed)]
mod inner;

mod anim;
mod censor;
mod codec;
mod charts_view;
mod client;
mod data;
pub mod deeplink;
mod icons;
mod images;
mod login;
mod mp;
mod page;
mod popup;
mod rate;
mod resource;
mod scene;
mod tabs;
mod tags;
mod threed;
mod uml;

use anyhow::Result;
use data::Data;
use macroquad::prelude::*;
use prpr::{
    build_conf,
    core::{init_assets, PGR_FONT},
    ext::SafeTexture,
    log,
    scene::show_error,
    time::TimeManager,
    ui::{cleanup_audio, FontArc, TextPainter},
    Main,
};
use prpr_l10n::set_prefered_locale;
#[cfg(not(feature = "hykb"))]
use prpr_l10n::{GLOBAL, LANGS};
use scene::MainScene;
use std::{
    collections::VecDeque,
    sync::{mpsc, Mutex},
};
use tracing::{error, info};

#[cfg(target_os = "android")]
use jni::{
    objects::{JClass, JShortArray, JString},
    sys::jint,
    EnvUnowned,
};

static MESSAGES_TX: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(None);
static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 老机渲染分辨率：目标高度（0 = 原生）。宽度按 5:3 计算。
pub static RENDER_HEIGHT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn render_res_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/render_h"))
}

#[cfg(target_os = "android")]
pub fn apply_render_size(h: u32) {
    // 内部渲染尺寸：高=h，宽按当前（原生）屏幕宽高比。0=原生。
    // 不再改 SurfaceView surface 尺寸（2.2 上不会被系统缩放会跑到角落）；
    // 改为：App 渲染到该尺寸的离屏 FBO，再由引擎拉伸 blit 到全屏。
    let (rw, rh) = if h == 0 {
        (0i32, 0i32)
    } else {
        let sw = macroquad::window::screen_width();
        let sh = macroquad::window::screen_height();
        let w = if sw > 0.0 && sh > 0.0 {
            (h as f32 * sw / sh).round() as i32
        } else {
            h as i32
        };
        (w, h as i32)
    };
    // 引擎按「渲染尺寸」渲染到离屏 FBO，再拉伸 blit 到全屏（真离屏）。
    miniquad::native::set_render_target_size(rw, rh);
    unsafe {
        // 通知 Java 内部渲染尺寸（宽+高），供触摸坐标按比例映射（surface 尺寸保持原生）
        let hh = if h == 0 { 0i32 } else { h as i32 };
        let env = miniquad::native::attach_jni_env();
        let ctx = ndk_context::android_context().context();
        let class = (**env).GetObjectClass.unwrap()(env, ctx);
        let method = (**env).GetMethodID.unwrap()(
            env,
            class,
            c"setRenderSize".as_ptr() as _,
            c"(II)V".as_ptr() as _,
        );
        (**env).CallVoidMethod.unwrap()(env, ctx, method, rw, hh);
    }
}

#[cfg(not(target_os = "android"))]
pub fn apply_render_size(_h: u32) {}

/// 请求宿主用隐藏 EditText 弹出软键盘（游戏内渲染输入框）。
#[cfg(target_os = "android")]
pub fn ime_start(text: &str, password: bool) {
    unsafe {
        let env = miniquad::native::attach_jni_env();
        let ctx = ndk_context::android_context().context();
        let class = (**env).GetObjectClass.unwrap()(env, ctx);
        let method = (**env).GetMethodID.unwrap()(env, class, c"startIme".as_ptr() as _, c"(Ljava/lang/String;Z)V".as_ptr() as _);
        let s = std::ffi::CString::new(text.replace('\0', "")).unwrap();
        let js = (**env).NewStringUTF.unwrap()(env, s.as_ptr());
        (**env).CallVoidMethod.unwrap()(env, ctx, method, js, if password { 1i32 } else { 0i32 });
    }
}

#[cfg(target_os = "android")]
pub fn ime_stop() {
    unsafe {
        let env = miniquad::native::attach_jni_env();
        let ctx = ndk_context::android_context().context();
        let class = (**env).GetObjectClass.unwrap()(env, ctx);
        let method = (**env).GetMethodID.unwrap()(env, class, c"stopIme".as_ptr() as _, c"()V".as_ptr() as _);
        (**env).CallVoidMethod.unwrap()(env, ctx, method);
    }
}

#[cfg(not(target_os = "android"))]
pub fn ime_start(_text: &str, _password: bool) {}
#[cfg(not(target_os = "android"))]
pub fn ime_stop() {}

/// 游戏已无法再返回上一页：让宿主退到后台。
#[cfg(target_os = "android")]
pub fn move_to_back() {
    unsafe {
        let env = miniquad::native::attach_jni_env();
        let ctx = ndk_context::android_context().context();
        let class = (**env).GetObjectClass.unwrap()(env, ctx);
        let method = (**env).GetMethodID.unwrap()(env, class, c"moveToBack".as_ptr() as _, c"()V".as_ptr() as _);
        (**env).CallVoidMethod.unwrap()(env, ctx, method);
    }
}
#[cfg(not(target_os = "android"))]
pub fn move_to_back() {}

/// 修改并持久化渲染分辨率（立即生效）。
pub fn set_render_height(h: u32) {
    RENDER_HEIGHT.store(h, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = render_res_path() {
        let _ = std::fs::write(&p, h.to_string());
    }
    apply_render_size(h);
}

/// 启动时读取渲染分辨率（0=原生时不调用 Java，避免启动即暂停渲染）。
pub fn init_render_height() {
    if let Some(p) = render_res_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(h) = s.trim().parse::<u32>() {
                RENDER_HEIGHT.store(h, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    if RENDER_HEIGHT.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        apply_render_size(RENDER_HEIGHT.load(std::sync::atomic::Ordering::Relaxed));
    }
}

/// 打击特效渲染方式：0=自动, 1=GPU实例化, 2=CPU模拟。
pub static PARTICLE_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn particle_mode_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/particle_mode"))
}

pub fn set_particle_mode(m: u8) {
    PARTICLE_MODE.store(m, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = particle_mode_path() {
        let _ = std::fs::write(&p, m.to_string());
    }
    prpr::particle::set_particle_mode(m);
}

pub fn init_particle_mode() {
    let mut m = 0u8;
    if let Some(p) = particle_mode_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            m = s.trim().parse().unwrap_or(0);
        }
    }
    PARTICLE_MODE.store(m, std::sync::atomic::Ordering::Relaxed);
    prpr::particle::set_particle_mode(m);
}

/// Android `ActivityManager.getMemoryClass()`（MB）。0=未知。
pub static MEMORY_CLASS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// 低内存设备：限制封面整图/大缩略图等，避免老机(OMAP 等) OOM。
/// 未知(0)时按普通设备处理，不影响普通设备体验。
/// 原生 OpenSL ES 低延迟音频（android only）。
#[cfg(target_os = "android")]
mod opensl;

/// 是否启用原生低延迟音频（设置项，Android 2.3+）。默认关，验证 OK 后再改默认开。
pub static NATIVE_AUDIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn native_audio_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/native_audio"))
}

pub fn native_audio() -> bool {
    NATIVE_AUDIO.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_native_audio(b: bool) {
    NATIVE_AUDIO.store(b, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = native_audio_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_native_audio() {
    // 默认开启；不支持的设备（API<9 等）由 startNativeAudio 的 dlopen/try_start 失败自动回退。
    let mut b = true;
    if let Some(p) = native_audio_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    NATIVE_AUDIO.store(b, std::sync::atomic::Ordering::Relaxed);
}

/// 谱面音乐流式的磁盘 PCM 缓存（首次解码落盘，之后直读；弱机避免播放时解码抢 CPU）。
pub static DISK_CACHE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn disk_cache_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/disk_cache"))
}

pub fn disk_cache() -> bool {
    DISK_CACHE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_disk_cache(b: bool) {
    DISK_CACHE.store(b, std::sync::atomic::Ordering::Relaxed);
    prpr::core::DISK_CACHE.store(b, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = disk_cache_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_disk_cache() {
    let mut b = false;
    if let Some(p) = disk_cache_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    DISK_CACHE.store(b, std::sync::atomic::Ordering::Relaxed);
    prpr::core::DISK_CACHE.store(b, std::sync::atomic::Ordering::Relaxed);
    *prpr::core::MUSIC_CACHE_DIR.lock().unwrap() =
        dir::root().ok().map(|r| std::path::PathBuf::from(format!("{r}/cache/music")));
}

/// 进谱面时是否销毁主菜单资源（省内存）。关闭则保留——返回主菜单更快（尤其弱机重建 BGM/贴图很慢）。
pub static MENU_DESTROY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn menu_destroy_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/menu_destroy"))
}

pub fn menu_destroy() -> bool {
    MENU_DESTROY.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_menu_destroy(b: bool) {
    MENU_DESTROY.store(b, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = menu_destroy_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_menu_destroy() {
    let mut b = true;
    if let Some(p) = menu_destroy_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    MENU_DESTROY.store(b, std::sync::atomic::Ordering::Relaxed);
}

/// 系统 API 等级（Java 传入），用于设置项可见性（安卓2.3+ 才显示原生音频开关）。
pub static API_LEVEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

pub fn api_level() -> i32 {
    API_LEVEL.load(std::sync::atomic::Ordering::Relaxed)
}

/// 低配标志：由 Java 按「堆≤64MB 或 单/双核 或 内存<768MB」判定（Java 堆不准，故只用 cpu/内存为主）。
pub static LOW_END: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn low_memory() -> bool {
    if LOW_END.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    let mb = MEMORY_CLASS.load(std::sync::atomic::Ordering::Relaxed);
    mb > 0 && mb <= 64
}

/// BGM 是否全流式解码（不驻留整曲 PCM）。
pub static MUSIC_STREAMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn music_streaming_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/music_streaming"))
}

fn apply_music_streaming(b: bool) {
    MUSIC_STREAMING.store(b, std::sync::atomic::Ordering::Relaxed);
    prpr::core::MUSIC_STREAMING.store(b, std::sync::atomic::Ordering::Relaxed);
}

pub fn set_music_streaming(b: bool) {
    apply_music_streaming(b);
    if let Some(p) = music_streaming_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_music_streaming() {
    // 默认开：避免整曲在主线程解码造成数秒卡死与数十 MB 内存尖峰
    // 默认：低配设备（low_memory）开流式——避免解码整曲 PCM 把小内存机撑爆（被系统杀）。
    let mut b = low_memory();
    if let Some(p) = music_streaming_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    apply_music_streaming(b);
}

/// 文字图集上限（0=自动）。改动会触发主 painter 重建。
pub static TEXT_ATLAS_LIMIT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static NEED_RECREATE_TEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn text_atlas_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/text_atlas"))
}

pub fn set_text_atlas_limit(v: u32) {
    TEXT_ATLAS_LIMIT.store(v, std::sync::atomic::Ordering::Relaxed);
    prpr::ui::set_atlas_limit(v);
    NEED_RECREATE_TEXT.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = text_atlas_path() {
        let _ = std::fs::write(&p, v.to_string());
    }
}

pub fn init_text_atlas_limit() {
    let mut v = 0u32;
    if let Some(p) = text_atlas_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            v = s.trim().parse().unwrap_or(0);
        }
    }
    TEXT_ATLAS_LIMIT.store(v, std::sync::atomic::Ordering::Relaxed);
    prpr::ui::set_atlas_limit(v);
}

/// 贴图最大边长（0=默认 1024）。
pub static TEXTURE_MAX_DIM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn texture_max_dim_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/tex_max_dim"))
}

pub fn set_texture_max_dim(v: u32) {
    TEXTURE_MAX_DIM.store(v, std::sync::atomic::Ordering::Relaxed);
    macroquad::texture::set_texture_max_dim(v);
    if let Some(p) = texture_max_dim_path() {
        let _ = std::fs::write(&p, v.to_string());
    }
}

pub fn init_texture_max_dim() {
    let mut v = 0u32;
    if let Some(p) = texture_max_dim_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            v = s.trim().parse().unwrap_or(0);
        }
    }
    TEXTURE_MAX_DIM.store(v, std::sync::atomic::Ordering::Relaxed);
    macroquad::texture::set_texture_max_dim(v);
}

pub fn take_need_recreate_text() -> bool {
    NEED_RECREATE_TEXT.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// 图片格式：true = RGB565（省一半显存，无 alpha；仅用于不透明图）。
pub static TEXTURE_RGB565: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn texture_rgb565_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/tex_rgb565"))
}

pub fn set_texture_rgb565(b: bool) {
    TEXTURE_RGB565.store(b, std::sync::atomic::Ordering::Relaxed);
    macroquad::texture::set_texture_rgb565(b);
    if let Some(p) = texture_rgb565_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_texture_rgb565() {
    let mut b = false;
    if let Some(p) = texture_rgb565_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    TEXTURE_RGB565.store(b, std::sync::atomic::Ordering::Relaxed);
    macroquad::texture::set_texture_rgb565(b);
}

/// 主页动态背景（整屏条纹着色器）。弱 GPU 上极耗填充率，低端默认关。
pub static BG_STRIPE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn bg_stripe_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/bg_stripe"))
}

pub fn bg_stripe() -> bool {
    BG_STRIPE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_bg_stripe(b: bool) {
    BG_STRIPE.store(b, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = bg_stripe_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_bg_stripe() {
    // 默认开：效果保留（着色器已把条纹坐标搬到顶点，逐像素成本很低）
    let mut b = true;
    if let Some(p) = bg_stripe_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    BG_STRIPE.store(b, std::sync::atomic::Ordering::Relaxed);
}

/// 页面转场动画：0=完整, 1=快速, 2=关闭。低端默认快速。
pub static PAGE_ANIM: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn page_anim_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/page_anim"))
}

pub fn set_page_anim(v: u8) {
    PAGE_ANIM.store(v, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = page_anim_path() {
        let _ = std::fs::write(&p, v.to_string());
    }
}

pub fn init_page_anim() {
    let mut v = if low_memory() { 1 } else { 0 };
    if let Some(p) = page_anim_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            v = s.trim().parse().unwrap_or(v);
        }
    }
    PAGE_ANIM.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// 低端：跳过屏幕外的判定线。
pub static LINE_CULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn line_cull_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/line_cull"))
}

pub fn line_cull() -> bool {
    LINE_CULL.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_line_cull(b: bool) {
    LINE_CULL.store(b, std::sync::atomic::Ordering::Relaxed);
    prpr::core::LINE_CULL.store(b, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = line_cull_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_line_cull() {
    let mut b = low_memory();
    if let Some(p) = line_cull_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    LINE_CULL.store(b, std::sync::atomic::Ordering::Relaxed);
    prpr::core::LINE_CULL.store(b, std::sync::atomic::Ordering::Relaxed);
}

/// 谱面渲染时把 model 烘焙进顶点以减少 draw call（部分设备/材质下可能显示异常，故可关）。
pub static BAKE_MODEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn bake_model_path() -> Option<String> {
    dir::root().ok().map(|r| format!("{r}/bake_model"))
}

pub fn bake_model() -> bool {
    BAKE_MODEL.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_bake_model(b: bool) {
    BAKE_MODEL.store(b, std::sync::atomic::Ordering::Relaxed);
    prpr::core::BAKE_MODEL.store(b, std::sync::atomic::Ordering::Relaxed);
    if let Some(p) = bake_model_path() {
        let _ = std::fs::write(&p, if b { "1" } else { "0" });
    }
}

pub fn init_bake_model() {
    let mut b = low_memory();
    if let Some(p) = bake_model_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            b = s.trim() == "1";
        }
    }
    BAKE_MODEL.store(b, std::sync::atomic::Ordering::Relaxed);
    prpr::core::BAKE_MODEL.store(b, std::sync::atomic::Ordering::Relaxed);
}
static DATA_PATH: Mutex<Option<String>> = Mutex::new(None);
static CACHE_DIR: Mutex<Option<String>> = Mutex::new(None);
pub static mut DATA: Option<Data> = None;

#[cfg(target_env = "ohos")]
use napi_derive_ohos::napi;

#[cfg(closed)]
pub async fn load_res(name: &str) -> Vec<u8> {
    let bytes = load_file(name).await.unwrap();
    inner::resolve_data(bytes)
}

#[allow(unused)]
pub async fn load_res_tex(name: &str) -> SafeTexture {
    #[cfg(closed)]
    {
        let bytes = load_res(name).await;
        let image = image::load_from_memory(&bytes).unwrap();
        image.into()
    }
    #[cfg(not(closed))]
    prpr::ext::BLACK_TEXTURE.clone()
}

pub fn sync_data() {
    if get_data().language.is_none() {
        #[cfg(feature = "hykb")]
        let default_lang = "zh-CN".to_owned();
        #[cfg(not(feature = "hykb"))]
        let default_lang = LANGS[GLOBAL.order.lock().unwrap()[0]].to_owned();
        get_data_mut().language = Some(default_lang);
    }
    set_prefered_locale(get_data().language.as_ref().and_then(|it| it.parse().ok()));
    let _ = client::set_access_token_sync(get_data().tokens.as_ref().map(|it| &*it.0));
}

pub fn set_data(data: Data) {
    unsafe {
        DATA = Some(data);
    }
}

#[allow(static_mut_refs)]
pub fn get_data() -> &'static Data {
    unsafe { DATA.as_ref().unwrap() }
}

#[allow(static_mut_refs)]
pub fn get_data_mut() -> &'static mut Data {
    unsafe { DATA.as_mut().unwrap() }
}

pub fn save_data() -> Result<()> {
    std::fs::write(format!("{}/data.json", dir::root()?), serde_json::to_string(get_data())?)?;
    Ok(())
}

mod dir {
    use anyhow::Result;

    use crate::{CACHE_DIR, DATA_PATH};

    fn ensure(s: &str) -> Result<String> {
        let s = format!("{}/{}", DATA_PATH.lock().unwrap().as_ref().map(|it| it.as_str()).unwrap_or("."), s);
        let path = std::path::Path::new(&s);
        if !path.exists() {
            std::fs::create_dir_all(path)?;
        }
        Ok(s)
    }

    pub fn cache() -> Result<String> {
        if let Some(cache) = &*CACHE_DIR.lock().unwrap() {
            ensure(cache)
        } else {
            ensure("cache")
        }
    }

    pub fn bold_font_path() -> Result<String> {
        Ok(format!("{}/bold.ttf", root()?))
    }

    pub fn cache_image_local() -> Result<String> {
        ensure(&format!("{}/image", cache()?))
    }

    pub fn root() -> Result<String> {
        ensure("data")
    }

    pub fn charts() -> Result<String> {
        ensure("data/charts")
    }

    pub fn collections() -> Result<String> {
        ensure("data/collections")
    }

    pub fn custom_charts() -> Result<String> {
        ensure("data/charts/custom")
    }

    pub fn downloaded_charts() -> Result<String> {
        ensure("data/charts/download")
    }

    pub fn respacks() -> Result<String> {
        ensure("data/respack")
    }
}

async fn the_main() -> Result<()> {
    log::register();
    let diag_dp = DATA_PATH.lock().unwrap().clone();
    let diag_cwd = std::env::current_dir();
    error!("DIAG cwd={:?} data_path={:?}", diag_cwd, diag_dp);
    #[cfg(target_env = "ohos")]
    {
        *DATA_PATH.lock().unwrap() = Some("/data/storage/el2/base".to_owned());
        *CACHE_DIR.lock().unwrap() = Some("/data/storage/el2/base/cache".to_owned());
        prpr::core::DPI_VALUE.store(250, std::sync::atomic::Ordering::Relaxed);
    };

    init_assets();
    error!("DIAG after init_assets");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();
    error!("DIAG rt ready");

    #[cfg(target_os = "ios")]
    {
        use objc2_foundation::{NSSearchPathDirectory, NSSearchPathDomainMask, NSSearchPathForDirectoriesInDomains};

        let directories = NSSearchPathForDirectoriesInDomains(NSSearchPathDirectory::LibraryDirectory, NSSearchPathDomainMask::UserDomainMask, true);
        let path = directories.firstObject().unwrap().to_string();
        *DATA_PATH.lock().unwrap() = Some(path);
        *CACHE_DIR.lock().unwrap() = Some("Caches".to_owned());
    }

    let dir = dir::root()?;
    init_render_height();
    // 先交出这一帧：确保「内部渲染尺寸」在任何绘制之前就已生效，避免首帧仍按原生
    // 尺寸渲染导致的冷启动过渡。已配合 macroquad exec.rs 把 waker 的 panic 去掉，可安全 await。
    macroquad::window::next_frame().await;
    init_particle_mode();
    init_music_streaming();
    init_text_atlas_limit();
    init_texture_max_dim();
    init_texture_rgb565();
    init_bg_stripe();
    init_page_anim();
    init_line_cull();
    init_bake_model();
    init_native_audio();
    init_disk_cache();
    init_menu_destroy();
    error!("DIAG root={:?}", dir);
    let mut data: Data = std::fs::read_to_string(format!("{dir}/data.json"))
        .map_err(anyhow::Error::new)
        .and_then(|s| Ok(serde_json::from_str(&s)?))
        .unwrap_or_default();
    data.init().await?;
    set_data(data);
    sync_data();
    save_data()?;
    error!("DIAG after save_data");
    // 低内存设备：关掉谱面特效 -> 不建 chart_target FBO、不额外 blit（省一整屏 pass）
    if low_memory() {
        get_data_mut().config.disable_effect = true;
    }

    // Warm up the offline banned-word automaton so local edits can check
    // synchronously. No-op without the `aa` feature.
    tokio::spawn(censor::preload());

    let rx = {
        let (tx, rx) = mpsc::channel();
        *MESSAGES_TX.lock().unwrap() = Some(tx);
        rx
    };

    unsafe { get_internal_gl() }
        .quad_context
        .display_mut()
        .set_pause_resume_listener(on_pause_resume);

    let pgr_font = FontArc::try_from_vec(load_file("phigros.ttf").await?)?;
    error!("DIAG after pgr_font");
    PGR_FONT.with(move |it| *it.borrow_mut() = Some(TextPainter::new(pgr_font, None)));

    let font = {
        let bytes = match load_file("font.ttf").await {
            Ok(b) if !b.is_empty() => b,
            _ => {
                // APK 里不放 12MB 字体（老机内部存储太小），回退到系统字体
                let mut v: Vec<u8> = Vec::new();
                for p in [
                    "/system/fonts/DroidSansFallback.ttf",
                    "/system/fonts/DroidSans.ttf",
                    "/system/fonts/Roboto-Regular.ttf",
                ] {
                    if let Ok(b) = std::fs::read(p) {
                        if !b.is_empty() {
                            v = b;
                            break;
                        }
                    }
                }
                v
            }
        };
        FontArc::try_from_vec(bytes)?
    };
    error!("DIAG after font");
    let mut painter = TextPainter::new(font.clone(), None);

    let mut main = Main::new(Box::new(MainScene::new(font.clone()).await?), TimeManager::default(), None).await?;
    error!("DIAG after Main::new");
    READY.store(true, std::sync::atomic::Ordering::Relaxed);

    let tm = TimeManager::default();
    let mut fps_time = -1;

    const FPS_BUF_SIZE: usize = 60;
    let mut fps_times = VecDeque::<f32>::with_capacity(FPS_BUF_SIZE);
    let mut last_frame_start = f32::NAN;
    let mut fps_time_sum = 0.;
    // 分相位计时（秒的累加）用于定位瓶颈：update(逻辑) / render(提交) / 总量
    let mut acc_upd = 0.0f64;
    let mut acc_draw = 0.0f64;
    let mut acc_n = 0u32;

    'app: loop {
        if main.paused() {
            match rx.recv() {
                Ok(false) => {
                    main.resume()?;
                }
                Ok(true) => {}
                Err(_) => break 'app,
            }
        }

        let frame_start = tm.real_time();
        if !last_frame_start.is_nan() {
            if fps_times.len() == FPS_BUF_SIZE {
                fps_time_sum -= fps_times.pop_front().unwrap();
            }
            let frame_time = frame_start as f32 - last_frame_start;
            fps_times.push_back(frame_time);
            fps_time_sum += frame_time;
        }
        last_frame_start = frame_start as f32;
        if take_need_recreate_text() {
            painter = TextPainter::new(font.clone(), None);
        }
        // 游戏内输入框：轮询软键盘显示/隐藏请求
        if let Some((text, pw)) = prpr::scene::take_ime_start() {
            ime_start(&text, pw);
        }
        if prpr::scene::take_ime_stop() {
            ime_stop();
        }
        if prpr::scene::WANT_EXIT.swap(false, std::sync::atomic::Ordering::Relaxed) {
            move_to_back();
        }
        let res = || -> Result<()> {
            let a = tm.real_time();
            main.update()?;
            let b = tm.real_time();
            main.render(&mut painter)?;
            let c = tm.real_time();
            acc_upd += (b - a) as f64;
            acc_draw += (c - b) as f64;
            acc_n += 1;
            if let Ok(paused) = rx.try_recv() {
                if paused {
                    main.pause()?;
                } else {
                    main.resume()?;
                }
            }
            prpr::ext::flush_pending_texture_deletions();
            Ok(())
        }();
        if let Err(err) = res {
            error!("uncaught error: {err:?}");
            show_error(err);
        }
        if main.should_exit() {
            break 'app;
        }

        let t = tm.real_time();

        let fps_now = t as i32;
        if fps_now != fps_time {
            fps_time = fps_now;
            if fps_times.len() == FPS_BUF_SIZE {
                let actual_fps = 1. / (fps_time_sum / FPS_BUF_SIZE as f32);
                let current_fps = 1. / (t - frame_start);
                let _ = (current_fps, actual_fps);
                acc_upd = 0.;
                acc_draw = 0.;
                acc_n = 0;
            }
        }

        // While backgrounded the scene is paused; the blocking `recv_timeout`
        // above already parks this thread, so nothing extra is needed here.
        next_frame().await;
    }
    Ok(())
}

fn build_global_window_conf() -> Conf {
    let mut conf = build_conf();
    conf.window_title = "Phira".to_owned();
    conf.icon = Some(miniquad::conf::Icon {
        small: *include_bytes!("../icon/small"),
        medium: *include_bytes!("../icon/medium"),
        big: *include_bytes!("../icon/big"),
    });

    #[cfg(target_os = "windows")]
    {
        conf.fullscreen = dir::root()
            .ok()
            .and_then(|r| std::fs::read_to_string(std::path::Path::new(&r).join("data.json")).ok())
            .and_then(|s| serde_json::from_str::<Data>(&s).ok())
            .is_some_and(|d| d.config.fullscreen_mode);
    }

    conf
}

#[no_mangle]
pub extern "C" fn quad_main() {
    macroquad::Window::from_config(build_global_window_conf(), async {
        if let Err(err) = the_main().await {
            error!(?err, "global error");
        }
    });
    cleanup_audio();
}

static LAST_PAUSED: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

fn send_pause_state(pause: bool) {
    use std::sync::atomic::Ordering;
    let v = if pause { 1i8 } else { 0i8 };
    if LAST_PAUSED.swap(v, Ordering::SeqCst) == v {
        return;
    }
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(pause);
    }
}

fn on_pause_resume(pause: bool) {
    send_pause_state(pause);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_initializeEnvironment(env: EnvUnowned, _class: JClass) {
    unsafe {
        inputbox::backend::Android::initialize_raw(env.as_raw()).unwrap();
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_prprActivityOnPause(_env: EnvUnowned, _class: JClass) {
    send_pause_state(true);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_prprActivityOnResume(_env: EnvUnowned, _class: JClass) {
    send_pause_state(false);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_prprActivityOnDestroy(_env: EnvUnowned, _class: JClass) {
    std::process::exit(0);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setDataPath(mut env: EnvUnowned, _class: JClass, path: JString) {
    env.with_env(|env| -> jni::errors::Result<()> {
        *DATA_PATH.lock().unwrap() = Some(path.try_to_string(env)?);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setTempDir(mut env: EnvUnowned, _class: JClass, path: JString) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let path = path.try_to_string(env)?;
        std::env::set_var("TMPDIR", path.clone());
        *CACHE_DIR.lock().unwrap() = Some(path);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setDpi(_env: EnvUnowned, _class: JClass, dpi: jint) {
    prpr::core::DPI_VALUE.store(dpi as _, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setMemoryClass(_env: EnvUnowned, _class: JClass, mb: jint) {
    MEMORY_CLASS.store(mb as _, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setLowEnd(_env: EnvUnowned, _class: JClass, b: jni::sys::jboolean) {
    LOW_END.store(b, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setApiLevel(_env: EnvUnowned, _class: JClass, api: jint) {
    API_LEVEL.store(api, std::sync::atomic::Ordering::SeqCst);
}

/// 由 Java AudioTrack 启动前调用：成功启用原生 OpenSL 输出则返回 true（Java 不再建 AudioTrack）。
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_startNativeAudio(
    _env: EnvUnowned,
    _class: JClass,
) -> jni::sys::jboolean {
    if !NATIVE_AUDIO.load(std::sync::atomic::Ordering::Relaxed) {
        return false as jni::sys::jboolean;
    }
    opensl::try_start() as jni::sys::jboolean
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_fillAudio(
    mut env: EnvUnowned,
    _class: JClass,
    arr: JShortArray,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let len = arr.len(env)?;
        let mut buf = vec![0i16; len];
        sasa::backend::android::pull_audio_i16(&mut buf);
        arr.set_region(env, 0, &buf)?;
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_isReady(
    _env: EnvUnowned,
    _class: JClass,
) -> jni::sys::jboolean {
    READY.load(std::sync::atomic::Ordering::Relaxed) as jni::sys::jboolean
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setChosenFile(mut env: EnvUnowned, _class: JClass, file: JString) {
    use prpr::scene::CHOSEN_FILE;
    env.with_env(|env| -> jni::errors::Result<()> {
        CHOSEN_FILE.lock().unwrap().1 = Some(file.try_to_string(env)?);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setDeepLink(mut env: EnvUnowned, _class: JClass, url: JString) {
    env.with_env(|env| -> jni::errors::Result<()> {
        deeplink::set_deeplink(url.try_to_string(env)?);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_markImport(_env: EnvUnowned, _class: JClass) {
    use prpr::scene::CHOSEN_FILE;

    CHOSEN_FILE.lock().unwrap().0 = Some("_import".to_owned());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_markImportRespack(_env: EnvUnowned, _class: JClass) {
    use prpr::scene::CHOSEN_FILE;

    CHOSEN_FILE.lock().unwrap().0 = Some("_import_respack".to_owned());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_setInputText(_env: EnvUnowned, _class: JClass, text: JString) {
    use prpr::scene::INPUT_TEXT;
    INPUT_TEXT.lock().unwrap().1 = Some(text.to_string());
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_imeText(mut env: EnvUnowned, _class: JClass, text: JString) {
    env.with_env(|env| -> jni::errors::Result<()> {
        prpr::scene::ingame_set_text(text.try_to_string(env)?);
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_imeSubmit(_env: EnvUnowned, _class: JClass) {
    prpr::scene::ingame_submit();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_imeCancel(_env: EnvUnowned, _class: JClass) {
    prpr::scene::ingame_cancel();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_onBackKey(_env: EnvUnowned, _class: JClass) {
    prpr::scene::BACK_PRESSED.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_onSearchKey(_env: EnvUnowned, _class: JClass) {
    prpr::scene::SEARCH_PRESSED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Credentials obtained from the native HYKB (好游快爆) login SDK.
pub struct HykbCredential {
    /// SDK result code: 0 on success, otherwise an error / user cancellation.
    pub code: i32,
    pub uid: i64,
    pub nick: String,
    pub access_token: String,
}

impl HykbCredential {
    /// Map the SDK result code to an error, or yield the credential on success.
    /// Centralizes the code → user-facing message translation shared by every
    /// HYKB login/bind entry point.
    #[cfg(feature = "hykb")]
    pub fn ok_or_err(self) -> Result<Self> {
        if self.code == 0 {
            Ok(self)
        } else {
            // A non-zero code is any failure the HYKB SDK reports: 2001 auth
            // failed, 2002 login failed, 2003 cancelled, 2004 exception, 2005
            // developer-requested exit / account logout. A HYKB build mandates a
            // valid, matching HYKB session, so every one of these must tear the
            // in-game session down — otherwise cancelling the HYKB prompt during
            // a silent re-verify would leave the player signed in and bypass the
            // gate entirely.
            force_logout();
            anyhow::bail!("{}", crate::ttl!("hykb-login-cancelled"))
        }
    }
}

/// Slot for the pending HYKB login result. The native callback fulfills it.
static HYKB_TX: Mutex<Option<tokio::sync::oneshot::Sender<HykbCredential>>> = Mutex::new(None);

/// Call a no-arg `void` method on the Android host activity (the HYKB shell).
#[cfg(all(target_os = "android", feature = "hykb"))]
fn call_activity_void(method: &'static jni::strings::JNIStr) {
    use jni::{jni_sig, objects::JObject, vm::JavaVM};

    JavaVM::singleton()
        .unwrap()
        .attach_current_thread(|env| -> jni::errors::Result<()> {
            let ctx = unsafe { JObject::from_raw(env, ndk_context::android_context().context() as _) };
            env.call_method(ctx, method, jni_sig!("()V"), &[])?;
            Ok(())
        })
        .unwrap();
}

/// Ask the Android shell to pop the HYKB account picker (`MainActivity.hykbSwitchAccount`).
/// Used by the explicit login / switch-account flow.
#[cfg(all(target_os = "android", feature = "hykb"))]
fn request_hykb_login() {
    call_activity_void(jni::jni_str!("hykbSwitchAccount"));
}

#[cfg(not(all(target_os = "android", feature = "hykb")))]
fn request_hykb_login() {}

/// Ask the Android shell to sign in using the cached HYKB account without
/// popping the picker (`MainActivity.hykbLogin`). The credentials the SDK
/// reports flow back through `HYKB_TX`, so the caller can verify them against
/// the restored Phira session. Used by the silent startup restore.
#[cfg(all(target_os = "android", feature = "hykb"))]
fn request_hykb_login_silent() {
    call_activity_void(jni::jni_str!("hykbLogin"));
}

#[cfg(not(all(target_os = "android", feature = "hykb")))]
fn request_hykb_login_silent() {}

/// Tell the native HYKB SDK to sign out (`MainActivity.hykbLogout`). Called when the
/// player logs out from their profile.
#[cfg(all(target_os = "android", feature = "hykb"))]
pub fn hykb_logout() {
    call_activity_void(jni::jni_str!("hykbLogout"));
}

#[cfg(not(all(target_os = "android", feature = "hykb")))]
pub fn hykb_logout() {}

/// Tear down the local session: sign out of the native HYKB SDK, clear the
/// stored account and tokens, then re-sync. Shared by every path that must
/// reject a login — a failed/cancelled HYKB verification, a uid mismatch, or
/// the player logging out from their profile.
pub fn force_logout() {
    hykb_logout();
    get_data_mut().me = None;
    get_data_mut().tokens = None;
    let _ = save_data();
    sync_data();
}

/// Trigger the native HYKB login and await its credentials.
#[allow(unused)]
pub async fn obtain_hykb_credential() -> Result<HykbCredential> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    *HYKB_TX.lock().unwrap() = Some(tx);
    request_hykb_login();
    let cred = rx.await.map_err(|_| anyhow::anyhow!("hykb login cancelled"))?;
    Ok(cred)
}

/// Silently restore the HYKB session from the cached account and await its
/// credentials. Unlike [`obtain_hykb_credential`], this does not pop the account
/// picker; used by the blocking startup check to verify the restored session.
#[allow(unused)]
pub async fn obtain_hykb_credential_silent() -> Result<HykbCredential> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    *HYKB_TX.lock().unwrap() = Some(tx);
    request_hykb_login_silent();
    let cred = rx.await.map_err(|_| anyhow::anyhow!("hykb login cancelled"))?;
    Ok(cred)
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_quad_1native_QuadNative_hykbLoginCallback(
    _env: EnvUnowned,
    _class: JClass,
    code: jint,
    uid: jni::sys::jlong,
    nick: JString,
    access_token: JString,
) {
    let nick = if nick.is_null() { String::new() } else { nick.to_string() };
    let access_token = if access_token.is_null() {
        String::new()
    } else {
        access_token.to_string()
    };
    if let Some(tx) = HYKB_TX.lock().unwrap().take() {
        let _ = tx.send(HykbCredential {
            code: code as i32,
            uid: uid as i64,
            nick,
            access_token,
        });
    } else if code == 2005 {
        // No login is in flight, so this is the SDK's asynchronous
        // anti-addiction "exit game" action: the player hit a play-time limit
        // and chose to quit from the SDK's own dialog. Honor it by exiting.
        // Other async codes (e.g. 2008 "continue playing") are handled inside
        // the SDK and need no response here. A request-less success (code 0, the
        // SDK switching accounts on its own) is likewise ignored: any signed-in
        // HYKB account is accepted, so a switch no longer tears the session down.
    }
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn set_input_text(text: String) {
    use prpr::scene::INPUT_TEXT;
    INPUT_TEXT.lock().unwrap().1 = Some(text);
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn set_chosen_file(file: String) {
    use prpr::scene::CHOSEN_FILE;
    CHOSEN_FILE.lock().unwrap().1 = Some(file);
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn mark_auto_import() {
    use prpr::scene::CHOSEN_FILE;
    CHOSEN_FILE.lock().unwrap().0 = Some("_import_auto".to_owned());
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn on_foreground() {
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(false);
    }
}

#[cfg(target_env = "ohos")]
#[napi]
pub fn on_background() {
    if let Some(tx) = MESSAGES_TX.lock().unwrap().as_mut() {
        let _ = tx.send(true);
    }
}
