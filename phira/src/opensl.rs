//! 原生 OpenSL ES 低延迟音频输出（Android 2.3+ / API 9+）。
//!
//! 通过 dlopen 加载 libOpenSLES.so（避免 minSdk=5 时因 DT_NEEDED 缺库导致 .so 加载失败），
//! 用 buffer queue 播放；回调线程里直接渲染 sasa 混音器（`pull_audio_i16`，无 JNI 往返）。
//! 任一步失败返回 false，调用方回退到 Java AudioTrack。

#![allow(non_snake_case, non_camel_case_types, clippy::missing_safety_doc)]

use std::ffi::{c_char, c_int, c_void};

type SLresult = u32;
type SLboolean = u32;
type SLuint32 = u32;

type SLObjectItf = *mut *const SLObjectItf_;
type SLInterfaceID = *const c_void;
type SLEngineItf = *mut *const SLEngineItf_;
type SLPlayItf = *mut *const SLPlayItf_;
type SLAndroidSimpleBufferQueueItf = *mut *const SLAndroidSimpleBufferQueueItf_;

#[repr(C)]
struct SLObjectItf_ {
    realize: unsafe extern "C" fn(SLObjectItf, SLboolean) -> SLresult,
    resume: *const c_void,
    get_state: *const c_void,
    get_interface: unsafe extern "C" fn(SLObjectItf, SLInterfaceID, *mut c_void) -> SLresult,
    register_callback: *const c_void,
    abort_async_operation: *const c_void,
    destroy: *const c_void,
    set_priority: *const c_void,
    get_priority: *const c_void,
    set_loss_of_control_interfaces: *const c_void,
}

#[repr(C)]
struct SLEngineItf_ {
    create_led_device: *const c_void,
    create_vibra_device: *const c_void,
    create_audio_player: unsafe extern "C" fn(
        SLEngineItf,
        *mut SLObjectItf,
        *const SLDataSource,
        *const SLDataSink,
        SLuint32,
        *const SLInterfaceID,
        *const SLboolean,
    ) -> SLresult,
    create_audio_recorder: *const c_void,
    create_midi_player: *const c_void,
    create_listener: *const c_void,
    create_3d_group: *const c_void,
    create_output_mix: unsafe extern "C" fn(
        SLEngineItf,
        *mut SLObjectItf,
        SLuint32,
        *const SLInterfaceID,
        *const SLboolean,
    ) -> SLresult,
    create_metadata_extractor: *const c_void,
    create_extension_object: *const c_void,
    query_num_supported_interfaces: *const c_void,
    query_supported_interfaces: *const c_void,
    query_num_supported_extensions: *const c_void,
    query_supported_extension: *const c_void,
    is_extension_supported: *const c_void,
}

#[repr(C)]
struct SLPlayItf_ {
    set_play_state: unsafe extern "C" fn(SLPlayItf, SLuint32) -> SLresult,
    get_play_state: *const c_void,
    get_duration: *const c_void,
    get_position: *const c_void,
    register_callback: *const c_void,
    set_callback_events_mask: *const c_void,
    get_callback_events_mask: *const c_void,
    set_marker_position: *const c_void,
    clear_marker_position: *const c_void,
    get_marker_position: *const c_void,
    set_position_update_period: *const c_void,
    get_position_update_period: *const c_void,
}

#[repr(C)]
struct SLAndroidSimpleBufferQueueItf_ {
    enqueue: unsafe extern "C" fn(SLAndroidSimpleBufferQueueItf, *const c_void, SLuint32) -> SLresult,
    clear: *const c_void,
    get_state: *const c_void,
    register_callback: unsafe extern "C" fn(
        SLAndroidSimpleBufferQueueItf,
        unsafe extern "C" fn(SLAndroidSimpleBufferQueueItf, *mut c_void),
        *mut c_void,
    ) -> SLresult,
    set_callback_events_mask: *const c_void,
    get_callback_events_mask: *const c_void,
}

#[repr(C)]
struct SLDataLocator_AndroidSimpleBufferQueue {
    locator_type: SLuint32,
    num_buffers: SLuint32,
}
#[repr(C)]
struct SLDataFormat_PCM {
    format_type: SLuint32,
    num_channels: SLuint32,
    samples_per_sec: SLuint32,
    bits_per_sample: SLuint32,
    container_size: SLuint32,
    channel_mask: SLuint32,
    endianness: SLuint32,
}
#[repr(C)]
struct SLDataLocator_OutputMix {
    locator_type: SLuint32,
    output_mix: SLObjectItf,
}
#[repr(C)]
struct SLDataSource {
    p_locator: *const c_void,
    p_format: *const c_void,
}
#[repr(C)]
struct SLDataSink {
    p_locator: *const c_void,
    p_format: *const c_void,
}

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;
const SL_RESULT_SUCCESS: SLresult = 0;
const SL_BOOLEAN_FALSE: SLboolean = 0;
const SL_PLAYSTATE_PLAYING: SLuint32 = 3;
const SL_DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE: SLuint32 = 0x8000_07AE;
const SL_DATALOCATOR_OUTPUTMIX: SLuint32 = 0x8000_0100;
const SL_DATAFORMAT_PCM: SLuint32 = 2;
const SL_SAMPLINGRATE_44_1: SLuint32 = 44_100_000;
const SL_PCMSAMPLEFORMAT_FIXED_16: SLuint32 = 16;
const SL_SPEAKER_FRONT_LEFT: SLuint32 = 0x1;
const SL_SPEAKER_FRONT_RIGHT: SLuint32 = 0x2;
const SL_BYTEORDER_LITTLEENDIAN: SLuint32 = 0;

/// 每缓冲帧数（i16 立体声 → 2048 个 i16）。1024 帧 ≈ 23ms。
const FRAMES: usize = 1024;
const BUF_SAMPLES: usize = FRAMES * 2;
const BUF_BYTES: SLuint32 = (BUF_SAMPLES * 2) as SLuint32;

static mut ENGINE_OBJ: SLObjectItf = std::ptr::null_mut();
static mut MIX: SLObjectItf = std::ptr::null_mut();
static mut PLAYER: SLObjectItf = std::ptr::null_mut();
static mut BQ: SLAndroidSimpleBufferQueueItf = std::ptr::null_mut();
static mut BUF0: *mut i16 = std::ptr::null_mut();
static mut BUF1: *mut i16 = std::ptr::null_mut();
static mut WHICH: usize = 0;

unsafe extern "C" fn on_buffer(_bq: SLAndroidSimpleBufferQueueItf, _ctx: *mut c_void) {
    let buf = if WHICH == 0 { BUF0 } else { BUF1 };
    WHICH ^= 1;
    if buf.is_null() || BQ.is_null() {
        return;
    }
    let slice = std::slice::from_raw_parts_mut(buf, BUF_SAMPLES);
    sasa::backend::android::pull_audio_i16(slice);
    let _ = ((**BQ).enqueue)(BQ, buf as *const c_void, BUF_BYTES);
}

/// 尝试启动原生 OpenSL ES 输出。成功返回 true（调用方不要再建 Java AudioTrack）。
pub fn try_start() -> bool {
    unsafe {
        let lib = dlopen(b"libOpenSLES.so\0".as_ptr() as *const c_char, RTLD_NOW);
        if lib.is_null() {
            return false;
        }
        let create = dlsym(lib, b"slCreateEngine\0".as_ptr() as *const c_char);
        let iid_engine = dlsym(lib, b"SL_IID_ENGINE\0".as_ptr() as *const c_char);
        let iid_play = dlsym(lib, b"SL_IID_PLAY\0".as_ptr() as *const c_char);
        let iid_bq = dlsym(lib, b"SL_IID_ANDROIDSIMPLEBUFFERQUEUE\0".as_ptr() as *const c_char);
        if create.is_null() || iid_engine.is_null() || iid_play.is_null() || iid_bq.is_null() {
            return false;
        }
        let sl_create_engine: unsafe extern "C" fn(
            *mut SLObjectItf,
            u32,
            *const c_void,
            u32,
            *const SLInterfaceID,
            *const SLboolean,
        ) -> SLresult = std::mem::transmute(create);
        // SL_IID_* 是导出的全局变量，dlsym 得到其地址 → 解引用得到 SLInterfaceID 值
        let id_engine: SLInterfaceID = *(iid_engine as *const SLInterfaceID);
        let id_play: SLInterfaceID = *(iid_play as *const SLInterfaceID);
        let id_bq: SLInterfaceID = *(iid_bq as *const SLInterfaceID);

        if sl_create_engine(
            &mut ENGINE_OBJ,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        ) != SL_RESULT_SUCCESS
            || ENGINE_OBJ.is_null()
        {
            return false;
        }
        if ((**ENGINE_OBJ).realize)(ENGINE_OBJ, SL_BOOLEAN_FALSE) != SL_RESULT_SUCCESS {
            return false;
        }
        let mut engine: SLEngineItf = std::ptr::null_mut();
        if ((**ENGINE_OBJ).get_interface)(ENGINE_OBJ, id_engine, &mut engine as *mut _ as *mut c_void)
            != SL_RESULT_SUCCESS
            || engine.is_null()
        {
            return false;
        }

        if ((**engine).create_output_mix)(engine, &mut MIX, 0, std::ptr::null(), std::ptr::null())
            != SL_RESULT_SUCCESS
            || MIX.is_null()
        {
            return false;
        }
        if ((**MIX).realize)(MIX, SL_BOOLEAN_FALSE) != SL_RESULT_SUCCESS {
            return false;
        }

        let loc_bufq = SLDataLocator_AndroidSimpleBufferQueue {
            locator_type: SL_DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE,
            num_buffers: 2,
        };
        let format = SLDataFormat_PCM {
            format_type: SL_DATAFORMAT_PCM,
            num_channels: 2,
            samples_per_sec: SL_SAMPLINGRATE_44_1,
            bits_per_sample: SL_PCMSAMPLEFORMAT_FIXED_16,
            container_size: SL_PCMSAMPLEFORMAT_FIXED_16,
            channel_mask: SL_SPEAKER_FRONT_LEFT | SL_SPEAKER_FRONT_RIGHT,
            endianness: SL_BYTEORDER_LITTLEENDIAN,
        };
        let src = SLDataSource {
            p_locator: &loc_bufq as *const _ as *const c_void,
            p_format: &format as *const _ as *const c_void,
        };
        let loc_outmix = SLDataLocator_OutputMix {
            locator_type: SL_DATALOCATOR_OUTPUTMIX,
            output_mix: MIX,
        };
        let sink = SLDataSink {
            p_locator: &loc_outmix as *const _ as *const c_void,
            p_format: std::ptr::null(),
        };
        let ids: [SLInterfaceID; 1] = [id_bq];
        let req: [SLboolean; 1] = [1];
        if ((**engine).create_audio_player)(
            engine,
            &mut PLAYER,
            &src,
            &sink,
            1,
            ids.as_ptr(),
            req.as_ptr(),
        ) != SL_RESULT_SUCCESS
            || PLAYER.is_null()
        {
            return false;
        }
        if ((**PLAYER).realize)(PLAYER, SL_BOOLEAN_FALSE) != SL_RESULT_SUCCESS {
            return false;
        }
        let mut play: SLPlayItf = std::ptr::null_mut();
        if ((**PLAYER).get_interface)(PLAYER, id_play, &mut play as *mut _ as *mut c_void)
            != SL_RESULT_SUCCESS
            || play.is_null()
        {
            return false;
        }
        if ((**PLAYER).get_interface)(PLAYER, id_bq, &mut BQ as *mut _ as *mut c_void)
            != SL_RESULT_SUCCESS
            || BQ.is_null()
        {
            return false;
        }

        BUF0 = vec![0i16; BUF_SAMPLES].leak().as_mut_ptr();
        BUF1 = vec![0i16; BUF_SAMPLES].leak().as_mut_ptr();
        if ((**BQ).register_callback)(BQ, on_buffer, std::ptr::null_mut()) != SL_RESULT_SUCCESS {
            return false;
        }

        let _ = ((**BQ).enqueue)(BQ, BUF0 as *const c_void, BUF_BYTES);
        let _ = ((**BQ).enqueue)(BQ, BUF1 as *const c_void, BUF_BYTES);
        if ((**play).set_play_state)(play, SL_PLAYSTATE_PLAYING) != SL_RESULT_SUCCESS {
            return false;
        }
        true
    }
}
