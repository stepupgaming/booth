#!/usr/bin/env bash
# Builds Booth's voice engine for Android phones (arm64-v8a), CPU only:
#
#   zonos3-server  -> gen/android/app/src/main/jniLibs/arm64-v8a/libzonos3server.so
#   booth-decode   -> gen/android/app/src/main/jniLibs/arm64-v8a/libboothdecode.so
#   LACI heads     -> apps/native/src-tauri/android-engine/laci_heads/ (built into the app)
#
# Android only lets an app run programs from its native library folder, so both programs ship
# named like libraries and the app runs them from there (booth-core paths::server_exe).
#
#   scripts/build-android-engine.sh <ZONOS3-Mini checkout> [build folder]
#
# The checkout should be at ZONOS3_REF from .github/workflows/release.yml, with its submodules
# servers/ggml/ggml and third_party/zyphra-textnorm. Needs the Android NDK (ANDROID_NDK_HOME,
# else the newest under the SDK), CMake, Ninja, and Rust with the aarch64-linux-android target.
set -euo pipefail
export MSYS_NO_PATHCONV=1 # Git Bash: keep /data/... and -D flags as they are

# Absolute paths the native tools understand (Git Bash: C:/... rather than /c/... or /tmp/...).
abspath() { (cd "$1" && { pwd -W 2>/dev/null || pwd; }); }
ZONOS3=$(abspath "${1:?usage: $0 <ZONOS3-Mini checkout> [build folder]}")
ROOT=$(abspath "$(dirname "$0")/..")
BUILD=${2:-$ROOT/target/android-engine}
mkdir -p "$BUILD"
BUILD=$(abspath "$BUILD") # a Git Bash path like /c/tmp/x would have cmake build in C:\c\tmp\x
API=28
TRIPLE=aarch64-linux-android
# dotprod and fp16: every phone CPU since ~2019 (Cortex-A55/A75 and later).
ARM_ARCH=armv8.2-a+dotprod+fp16

SDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-${LOCALAPPDATA:-$HOME}/Android/Sdk}}
NDK=${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/* | sort -V | tail -1)}
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) HOST=windows-x86_64; CMD=.cmd; EXE=.exe ;;
  Darwin) HOST=darwin-x86_64; CMD=; EXE= ;;
  *) HOST=linux-x86_64; CMD=; EXE= ;;
esac
TC=$NDK/toolchains/llvm/prebuilt/$HOST/bin
[ -d "$TC" ] || { echo "no NDK toolchain at $TC" >&2; exit 1; }
CC=$TC/$TRIPLE$API-clang$CMD
export CC_aarch64_linux_android=$CC
export CXX_aarch64_linux_android=$TC/$TRIPLE$API-clang++$CMD
export AR_aarch64_linux_android=$TC/llvm-ar$EXE
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$CC

JNI=$ROOT/apps/native/src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a
HEADS=$ROOT/apps/native/src-tauri/android-engine/laci_heads
mkdir -p "$BUILD" "$JNI" "$HEADS"
echo "NDK $NDK"
echo "ZONOS3-Mini $(git -C "$ZONOS3" rev-parse --short HEAD 2>/dev/null || echo '?')"

cmake -S "$ZONOS3/servers/ggml" -B "$BUILD" -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-$API -DCMAKE_BUILD_TYPE=Release \
  -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF -DGGML_CCACHE=OFF -DGGML_CPU_ARM_ARCH=$ARM_ARCH \
  -DBUILD_SHARED_LIBS=OFF

# The engine's CMake builds its Rust text normaliser for the host, which the phone can't link.
# Let that rule run once so ninja records it, then put the arm64 build in its place: ninja leaves
# an output alone once it is newer than its sources. Skipping the first step instead would mean
# ninja runs the rule on its own later (a cold build log makes every rule dirty) and overwrites
# the arm64 library, which ends in "is incompatible with aarch64linux" at link time.
cmake --build "$BUILD" --target zyphra-textnorm-build
cargo rustc --release --locked --lib -p zyphra-textnorm --features capi --crate-type staticlib \
  --manifest-path "$ZONOS3/third_party/zyphra-textnorm/Cargo.toml" \
  --target $TRIPLE --target-dir "$BUILD/textnorm-android"
cp "$BUILD/textnorm-android/$TRIPLE/release/libzyphra_textnorm.a" "$BUILD/textnorm/release/libzyphra_textnorm.a"

cmake --build "$BUILD" --target zonos3-server
"$TC/llvm-strip$EXE" -o "$JNI/libzonos3server.so" "$BUILD/zonos3-server"

cargo build --release -p booth-decode --target $TRIPLE --manifest-path "$ROOT/Cargo.toml"
"$TC/llvm-strip$EXE" -o "$JNI/libboothdecode.so" "$ROOT/target/$TRIPLE/release/booth-decode"

rm -f "$HEADS"/*.json
cp "$ZONOS3"/assets/laci_heads/*.json "$HEADS/"
cp "$ZONOS3/LICENSE" "$HEADS/../LICENSE-ZONOS3.txt"

ls -l "$JNI"
echo "$(ls "$HEADS" | wc -l) LACI heads in $HEADS"
