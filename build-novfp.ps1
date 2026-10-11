# 构建 no-VFP（软浮点）armeabi 版，供无 VFP 的极老设备（如 HTC Tattoo / 纯 ARMv5）使用。
#
# 与正常 armeabi 的区别：
#   - Rust 目标不启用 VFP / ARMv6（用环境变量 CARGO_TARGET_ARM_LINUX_ANDROIDEABI_RUSTFLAGS 覆盖
#     .cargo/config.toml 里的 +v6,+vfp2）——软浮点，全兼容，但慢一些；
#   - C 依赖用 --target=armv5te-linux-androideabi8（无 VFP）。
#
# 用法：powershell -ExecutionPolicy Bypass -File build-novfp.ps1
# 产物：L:\Other\Phira-port\novfp\armeabi\libphira.so（打包时放进对应 APK 的 lib/armeabi/）

$ErrorActionPreference = 'Stop'
$NewNdk  = "I:\Other\android-ndk-r26d"
$OldNdk  = "L:\Other\android-ndk-r10e"
$RustBin = "L:\Program Files\Rust stable GNU 1.87\bin"
$W64     = "L:\Software\w64devkit\bin"
$Clang   = "$NewNdk\toolchains\llvm\prebuilt\windows-x86_64\bin\clang.exe"
$LlvmAr  = "$NewNdk\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-ar.exe"
$Rsp     = "L:/Other/Phira-port/rust-poc/build/link_arm.rsp"
$OutDir  = "L:\Other\Phira-port\novfp\armeabi"

$env:PATH = "$W64;$RustBin;$env:PATH"
Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue

# 关键：覆盖目标 rustflags —— 不带任何 target-feature（软浮点，无 VFP/无 v6 指令）
$env:CARGO_TARGET_ARM_LINUX_ANDROIDEABI_RUSTFLAGS = "-C link-arg=@$Rsp"
$env:CC_arm_linux_androideabi  = $Clang
$env:AR_arm_linux_androideabi  = $LlvmAr
$env:CFLAGS_arm_linux_androideabi = "--target=armv5te-linux-androideabi8 --sysroot=$OldNdk\platforms\android-8\arch-arm -DSIZE_MAX=((size_t)-1)"

Set-Location "L:\Other\phira-main"
& "$RustBin\cargo.exe" build -p phira --no-default-features --release --target arm-linux-androideabi
if ($LASTEXITCODE -ne 0) { Write-Host "[ERR] cargo build 失败"; exit 1 }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Copy-Item -Force "L:\Other\phira-main\target\arm-linux-androideabi\release\libphira.so" "$OutDir\libphira.so"
Write-Host ("[OK] no-VFP armeabi -> " + $OutDir + "\libphira.so (" + [int]((Get-Item "$OutDir\libphira.so").Length/1MB) + "MB)")
