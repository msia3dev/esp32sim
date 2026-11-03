# Compile on x64 and run ARM64 binaries under user-mode QEMU. This needs Docker,
# but no privileged container or host binfmt registration.
FROM rust:1-trixie
RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc-aarch64-linux-gnu libc6-dev-arm64-cross qemu-user ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN rustup toolchain install stable --profile minimal --component clippy \
    --target aarch64-unknown-linux-gnu
ENV CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64 -L /usr/aarch64-linux-gnu"
WORKDIR /work
COPY . .
RUN tools/fetch-rom-elfs.sh roms && tools/fetch-p4-firmware.sh
RUN cargo +stable clippy --target aarch64-unknown-linux-gnu --workspace --all-targets -- -D warnings
RUN cargo +stable test --no-run --release --target aarch64-unknown-linux-gnu -p xtensa-lx7 -p emu-core
RUN cargo +stable test --release --target aarch64-unknown-linux-gnu -p xtensa-lx7 -p emu-core
# Goldens spawn the CLI binaries directly: Cargo's runner covers the test harness,
# but not those subprocesses. Shell launchers keep this independent of host binfmt.
RUN cargo +stable test --no-run --release --target aarch64-unknown-linux-gnu -p esp32sim --test goldens
RUN for name in esp32sim esp32sim-c3 esp32sim-c6 esp32sim-p4; do \
      bin="target/aarch64-unknown-linux-gnu/release/$name"; \
      mv "$bin" "$bin.arm64"; \
      printf '#!/bin/sh\nexec qemu-aarch64 -L /usr/aarch64-linux-gnu "$0.arm64" "$@"\n' > "$bin"; \
      chmod +x "$bin"; \
    done
# Run the built harness directly: another Cargo invocation can rematerialize its
# CLI artifacts and replace the launchers with the original ARM executables.
RUN set -eu; found=0; \
    for harness in target/aarch64-unknown-linux-gnu/release/deps/goldens-*; do \
      [ -f "$harness" ] && [ -x "$harness" ] || continue; \
      found=1; \
      ESP32SIM_ROM_DIR=/work/roms qemu-aarch64 -L /usr/aarch64-linux-gnu \
        "$harness" --include-ignored --skip external_; \
    done; \
    test "$found" -eq 1
