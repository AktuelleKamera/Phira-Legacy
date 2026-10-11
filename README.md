# Phira Legacy Android Port

![Phira](https://phira.5wyxi.com/files/github-showcase.png)

> 将[Phira](https://github.com/TeamFlos/phira)（Rust + macroquad/miniquad）移植到**清朝安卓设备**，最低支持Android 2.0（minSdk 5），适用于ARMv5/VFPv2的MSM7227等早期SoC。本移植目前支持armeabi-vfp, armeabi-v7a, x86。

## 推荐配置

- **最低**：单核ARMv6（VFPv2），`512MB`内存，Android 2.0
- **推荐**：`MSM8260`/`S5PC110`等双核设备，`768MB`内存及以上，Android 2.2+
- **GPU**：支持OpenGL ES 2.0（Adreno 205 / PowerVR SGX540等）
- **存储**：约`200MB`可用空间（推荐`500MB`以上）

> 极老单核设备（如HTC Tattoo、纯ARMv5无VFP）请使用`no-VFP`版本，详见下文。

## 目录结构/依赖

- `phira-main/`（本仓库）：Rust workspace（`phira` + `prpr` + `prpr-l10n`等）。
- `phira-port/`：Android Gradle工程，产物位于`app/src/main/jniLibs/<abi>/libphira.so`。

引擎为本地fork，懒得传了（路径写在`Cargo.toml`的`[patch]`中）：

```
I:\Other\prpr-macroquad-master
I:\Other\prpr-miniquad-master
I:\Other\sasa-main
```

> **注意**：这些fork未随本仓库上传，包含本移植所需的修改（见下文“与上游的差异”）。请自行准备或向我索取罢（

## 构建（3个ABI）

主机环境：**Windows 7**。Rust使用**1.87 GNU**，NDK使用**r26d的clang/lld**编译、**r10e的sysroot/CRT/libc**链接，C依赖使用`L:\Software\w64devkit`编译。

千万不要用Windows 7编译Rust 1.87，否则你会先吃饱VxKex（

`.cargo/config.toml`中包含各目标的linker/rustflags（**含本机绝对路径，换机需自行修改**）。

```

编译完成后，将`target/<triple>/release/libphira.so`拷贝到`Phira-port/app/src/main/jniLibs/{armeabi-v7a,armeabi,x86}/`，然后使用Gradle打包。

### no-VFP版本（极老设备）

`build-novfp.ps1`会通过环境变量覆盖Rust目标特性，编出**软浮点（无VFP）**的armeabi，供纯ARMv5等没有VFP的清朝机器使用，产物输出到`Phira-port/novfp/armeabi/`。

## ABI说明

| ABI | 目标 | 适用设备 |
|-----|------|----------|
| `armeabi-v7a` | `armv7-linux-androideabi` | 主流ARMv7 |
| `armeabi` | `arm-linux-androideabi`（+v6,+vfp2,+d16, arm1136jf-s） | ARMv6-vfp设备 |
| `x86` | `i686-linux-android` | 模拟器/x86机器 |

## 与上游的主要差异/老设备优化

- **渲染分辨率**：离屏FBO+拉伸blit（可关闭）。surface保持原生，渲染到低分辨率离屏目标再放大
- **低配自动判定**：`堆≤64MB或CPU核心≤2或物理内存<768MB`→触发`low_memory()`，自动应用更省内存/更省算力的设置
- **逻辑渲染优化**：合批优化（可开关）、判定线裁剪、非普通音符/视口裁剪、粒子索引缓冲复用、每帧视口/变换缓存
  - **流式播放**（低配默认开启）
  - **OpenSL ES低延迟开关**（Android 2.3+，可开关）
  - **流式音乐磁盘PCM缓存**：首次解码缓存到本地，虽然效果非常意义不明；预览只解15秒
  - EGL先建后销（避免后台/弹窗返回崩溃）
  - waker不再panic
  - 菜单资源释放/音频重声明
  - 图集/贴图大小与格式可调等

## 下载

- [GitHub Release](https://github.com/AktuelleKamera/phira/releases)：Android、Windows、Linux

## 贡献

欢迎提交Issue和Pull Request！建议先看看这些
[good first issue](https://github.com/TeamFlos/phira/issues?q=label%3A%22good+first+issue%22)
和[pull requests](https://github.com/TeamFlos/phira/pulls)。

如果想深入交流，可以发邮件到[contact@phira.cn](mailto:contact@phira.cn)。
我们正在寻找**有节奏游戏经验**的**Rust**程序员。

## 翻译

参见[Weblate](https://hosted.weblate.org/engage/phira/)页面获取最新翻译进度，欢迎在那里贡献翻译。

![翻译进度](https://hosted.weblate.org/widget/phira/svg-badge.svg)

![翻译状态](https://hosted.weblate.org/widget/phira/multi-auto.svg)

## Star历史

[![Stargazers over time](https://starchart.cc/TeamFlos/phira.svg?variant=adaptive)](https://starchart.cc/TeamFlos/phira)

## 许可

上游Phira使用**GPL-3.0-only**许可，本移植沿用同一许可。