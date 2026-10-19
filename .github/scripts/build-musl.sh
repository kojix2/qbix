#!/usr/bin/env bash
set -euo pipefail

target=x86_64-unknown-linux-musl
prefix=${RUNNER_TEMP}
pkg_dirs="${prefix}/htslib/lib/pkgconfig:${prefix}/libdeflate/lib/pkgconfig:${prefix}/zlib/lib/pkgconfig"
export CC=musl-gcc
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc
# Rust uses -nodefaultlibs; htslib's GCC CPU dispatch needs libgcc.
export RUSTFLAGS='-C target-feature=+crt-static -C link-arg=-lgcc'
export ZLIB_PREFIX="${prefix}/zlib"
export LIBDEFLATE_PREFIX="${prefix}/libdeflate"
export HTSDIR="${prefix}/htslib"
export PKG_CONFIG_PATH="${pkg_dirs}"
export PKG_CONFIG_LIBDIR="${pkg_dirs}"

if [[ ! -f "${ZLIB_PREFIX}/lib/libz.a" ]]; then
  curl -fsSL "https://zlib.net/fossils/zlib-${ZLIB_VERSION}.tar.gz" -o "${prefix}/zlib.tar.gz"
  tar -xzf "${prefix}/zlib.tar.gz" -C "${prefix}"
  (
    cd "${prefix}/zlib-${ZLIB_VERSION}"
    ./configure --static --prefix="${ZLIB_PREFIX}"
    make -j"$(nproc)"
    make install
  )
fi

if [[ ! -f "${LIBDEFLATE_PREFIX}/lib/libdeflate.a" ]]; then
  curl -fsSL "https://github.com/ebiggers/libdeflate/releases/download/v${LIBDEFLATE_VERSION}/libdeflate-${LIBDEFLATE_VERSION}.tar.gz" -o "${prefix}/libdeflate.tar.gz"
  tar -xzf "${prefix}/libdeflate.tar.gz" -C "${prefix}"
  cmake -S "${prefix}/libdeflate-${LIBDEFLATE_VERSION}" -B "${prefix}/libdeflate-build" \
    -DCMAKE_C_COMPILER=musl-gcc \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="${LIBDEFLATE_PREFIX}" \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DLIBDEFLATE_BUILD_SHARED_LIB=OFF \
    -DLIBDEFLATE_BUILD_GZIP=OFF \
    -DLIBDEFLATE_BUILD_TESTS=OFF
  cmake --build "${prefix}/libdeflate-build" --parallel "$(nproc)"
  cmake --install "${prefix}/libdeflate-build"
fi

if [[ ! -f "${HTSDIR}/lib/libhts.a" ]]; then
  curl -fsSL "https://github.com/samtools/htslib/releases/download/${HTSLIB_VERSION}/htslib-${HTSLIB_VERSION}.tar.bz2" -o "${prefix}/htslib.tar.bz2"
  tar -xjf "${prefix}/htslib.tar.bz2" -C "${prefix}"
  (
    cd "${prefix}/htslib-${HTSLIB_VERSION}"
    CFLAGS='-fPIC' \
    CPPFLAGS="-I${LIBDEFLATE_PREFIX}/include -I${ZLIB_PREFIX}/include" \
    LDFLAGS="-L${LIBDEFLATE_PREFIX}/lib -L${ZLIB_PREFIX}/lib" \
      ./configure --host=x86_64-linux-musl \
      --prefix="${HTSDIR}" \
      --disable-bz2 \
      --disable-gcs \
      --disable-libcurl \
      --disable-lzma \
      --disable-plugins \
      --disable-ref-cache \
      --disable-s3 \
      --with-libdeflate
    make -j"$(nproc)"
    make install
  )
  rm -f "${HTSDIR}"/lib/libhts.so*
fi

test -f "${ZLIB_PREFIX}/lib/libz.a"
test -f "${LIBDEFLATE_PREFIX}/lib/libdeflate.a"
test -f "${HTSDIR}/lib/libhts.a"

for name in CC CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER RUSTFLAGS ZLIB_PREFIX LIBDEFLATE_PREFIX HTSDIR PKG_CONFIG_PATH PKG_CONFIG_LIBDIR; do
  printf '%s=%s\n' "${name}" "${!name}" >> "${GITHUB_ENV}"
done

cargo build --release --locked --target "${target}" --bin qbix
binary="target/${target}/release/qbix"
file "${binary}"
if readelf -lW "${binary}" | grep -q INTERP; then
  echo 'musl binary has a dynamic interpreter' >&2
  exit 1
fi
if readelf -dW "${binary}" 2>/dev/null | grep -q NEEDED; then
  echo 'musl binary has dynamic library dependencies' >&2
  exit 1
fi
"${binary}" --help >/dev/null
