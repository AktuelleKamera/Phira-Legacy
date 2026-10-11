# 构建 Tegra 2（NVIDIA Tegra 250，Cortex-A9 无 NEON）专用 armeabi-v7a 版。
# Tegra 2 的 A9 核没有 NEON，带 NEON 的普通 v7a 构建会 SIGILL。此版显式 -neon + target-cpu=cortex-a9。
#
# 用法：powershell -ExecutionPolicy Bypass -File build-tegra2.ps1
# 产物：L:\Other\Phira-port\tegra2\armeabi-v7a\libphira.so（打包进对应 APK 的 lib/armeabi-v7a/）

$ErrorActionPreference = 'Stop'
$NewNdk  = "I:\Other\android-ndk-r26d"
$OldNdk  = "L:\Other\android-ndk-r10e"
$RustBin = "L:\Program Files\Rust stable GNU 1.87\bin"
$W64     = "L:\Software\w64devkit\bin"
$Clang   = "$NewNdk\toolchains\llvm\prebuilt\windows-x86_64\bin\clang.exe"
$LlvmAr  = "$NewNdk\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-ar.exe"
$Rsp     = "L:/Other/Phira-port/rust-poc/build/link.rsp"
$OutDir  = "L:\Other\Phira-port\tegra2\armeabi-v7a"

$env:PATH = "$W64;$RustBin;$env:PATH"
Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue

# 关键：覆盖 v7a 的 rustflags —— 禁用 NEON，按 cortex-a9 调度
$env:CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_RUSTFLAGS = "-C link-arg=@$Rsp -C target-cpu=cortex-a9 -C target-feature=-neon"
$env:CC_armv7_linux_androideabi  = $Clang
$env:AR_armv7_linux_androideabi  = $LlvmAr
$env:CFLAGS_armv7_linux_androideabi = "--target=armv7a-linux-androideabi8 -march=armv7-a -mfpu=vfpv3-d16 -mfloat-abi=softfp --sysroot=$OldNdk\platforms\android-8\arch-arm -DSIZE_MAX=((size_t)-1)"

Set-Location "L:\Other\phira-main"
& "$RustBin\cargo.exe" build -p phira --no-default-features --release --target armv7-linux-androideabi
if ($LASTEXITCODE -ne 0) { Write-Host "[ERR] cargo build 失败"; exit 1 }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Copy-Item -Force "L:\Other\phira-main\target\armv7-linux-androideabi\release\libphira.so" "$OutDir\libphira.so"
Write-Host ("[OK] Tegra2 (no-NEON) armeabi-v7a -> " + $OutDir + "\libphira.so (" + [int]((Get-Item "$OutDir\libphira.so").Length/1MB) + "MB)")
