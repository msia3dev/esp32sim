# Compile on x64 and run ARM64 binaries under user-mode QEMU. This needs Docker,
# but no privileged container or host binfmt registration.
FROM rust:1-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc-aarch64-linux-gnu libc6-dev-arm64-cross qemu-user ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN rustup toolchain install stable --profile minimal --component clippy \
    --target aarch64-unknown-linux-gnu
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64 -L /usr/aarch64-linux-gnu"
WORKDIR /work
COPY . .
RUN tools/fetch-rom-elfs.sh roms && tools/fetch-p4-firmware.sh
RUN cargo +stable clippy --target aarch64-unknown-linux-gnu --workspace --all-targets -- -D warnings
RUN cargo +stable test --release --target aarch64-unknown-linux-gnu -p xtensa-lx7 -p emu-core
RUN ESP32SIM_ROM_DIR=/work/roms cargo +stable test --release --target aarch64-unknown-linux-gnu \
    -p esp32sim --test goldens -- --include-ignored --skip external_
