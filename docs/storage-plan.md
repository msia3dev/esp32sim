# Persistent flash, NVS and eFuse plan

Date: 2026-09-17

Status: implemented on `feat/esp32s3-storage-support`. For user-facing setup,
flags, formats and browser controls, see [storage.md](storage.md). This document
retains the implementation rationale and test plan.

## Why this work exists

esp32sim already executes ESP-IDF's SPI flash and NVS code, and its ESP32-S3
eFuse model already implements basic one-way programming. The missing behavior
is durability across emulator processes. Without it, an NVS commit or eFuse
burn survives a software reset but disappears when esp32sim exits.

NVS does not need a host-side NVS implementation. It is an ESP-IDF data format
inside a normal flash partition. Correct persistent SPI flash behavior therefore
provides persistent NVS, OTA metadata, PHY calibration and other flash-backed
state without interpreting any of their contents in the emulator.

## Scope boundary

This public plan contains only generic emulator capabilities.

In scope:

- persistent native flash state;
- persistent ESP32-S3 eFuse physical blocks;
- software-reset and process-restart semantics;
- browser persistence and state import/export;
- generic unit, integration and restart tests;
- documentation of the state formats and security properties.

Out of scope:

- product-specific partitions, namespaces, keys or values;
- parsing or editing NVS records in esp32sim;
- real device credentials, signing material or flash-encryption keys;
- UART/TCP download mode, esptool or espefuse integration;
- full-process snapshots of SRAM, CPU registers or peripheral state.

## User-facing model

Seed inputs remain immutable build artifacts:

```text
--flash-image PATH
--bootloader PATH
--ptable PATH
--app PATH
--flash-at OFFSET=PATH
--efuse-regs PATH
```

Mutable state is selected independently:

```text
--flash-state PATH
--efuse-state PATH
```

The lifecycle is:

1. If a state file exists, validate and load it. Do not apply seed inputs over
   that state.
2. If it does not exist, create erased storage, apply the seed inputs once, and
   atomically publish the resulting initial state.
3. Persist guest flash program/erase operations and successful eFuse burns.
4. Preserve both devices across guest software reset.
5. Reopen the same state on the next emulator process.

Passing both existing state and seed inputs is allowed so a stable runner
command can be reused. The seed inputs are ignored after the state has been
created, with one concise diagnostic explaining that existing state won.

The emulator must never overwrite the file supplied through `--flash-image`,
nor any bootloader, partition-table, application or `--flash-at` source.

## State formats

### Flash

The native flash-state file is the exact length of the configured flash and
contains the same logical, CPU-visible bytes stored by `SocBus.flash`.

This is intentionally not claimed to be a physical encrypted-flash dump.
esp32sim currently keeps decrypted logical bytes when exercising encrypted
flash operations. The format must document that limitation rather than imply
interchangeability with a physical device or QEMU ciphertext image.

Validation requirements:

- reject a state file whose length differs from configured flash capacity;
- reject an image or overlay extending beyond flash;
- do not silently resize an existing state file;
- include the expected and actual lengths in errors;
- create new files with user-only permissions where the host supports them.

### eFuse

The canonical in-memory representation is the physical eFuse blocks, not the
program staging registers or derived read-shadow registers. Serialization uses
little-endian words in physical block order.

The native format is esp32sim's 336-byte ESP32-S3 physical payload. A fixture
verifies the same block boundaries as Espressif QEMU's `ESPEfuseBlocks`, and the
loader also accepts QEMU's commonly used 1 KiB backing file while preserving
the bytes beyond the compatible payload.

The loader must reject truncated, oversized or unsupported layouts. It must
never print the contents of key-purpose blocks or other fuse data. Diagnostics
may contain only the path, format version/size and block numbers affected.

## Internal architecture

### S1 — storage abstraction

Add a small chip-independent storage layer, probably under `esp-soc`, with:

- immutable initialization from erased bytes plus overlays;
- an optional native file backend;
- dirty-range notification;
- explicit `flush()` and finalization;
- errors carrying operation and path without dumping data;
- an in-memory backend for tests and runs without state flags.

Do not put filesystem calls in `SpiMem` or the eFuse register device. Device
models report completed mutations; the machine/front end owns persistence.

### S2 — flash mutation receipts

Use the existing SPI flash mutation points in `esp-periph/src/spi_mem.rs` as
the source of truth. A receipt must describe the actual range changed by:

- page program, including encrypted-program logical data;
- sector erase;
- block erase;
- chip erase;
- any supported alternate program/erase opcode.

The receipt is emitted only after the in-memory operation completes. Merge
overlapping or adjacent ranges before writing them to the backend. Seed writes
performed before boot initialize state but are not guest mutations.

Native persistence writes the changed range to its offset in the raw state
file. Initial creation uses a temporary sibling file, flush, and atomic rename.
Runtime mutations are written through after each completed flash command;
`flush()` is also called before a guest reboot, on a normal stop and on clean
process termination. Document that process-crash durability is provided after
the write reaches the host OS, while host-power-loss guarantees depend on the
filesystem.

### S3 — physical eFuse model

Refactor `esp-periph/src/efuse.rs` so it owns explicit physical blocks and
derives read-shadow registers from them. Keep programming registers transient.

A successful program command must:

1. validate the selected block;
2. apply write-protection rules;
3. OR permitted staged bits into the physical block;
4. rebuild affected read shadows;
5. clear/complete the command state according to the modeled controller;
6. emit a mutation receipt containing block and changed-word mask.

At minimum, persistence acceptance requires one-way programming and write
protection. Read-protected blocks must return their hardware-defined hidden
value through normal guest reads. Operation latency and completion interrupts
may be added in a subsequent fidelity patch, but must not change the state-file
format.

eFuse mutations are written and flushed immediately after every successful
burn. There is no opt-in save-on-exit mode because fuses are one-time
programmable. Loading `--efuse-regs` remains a diagnostic seed mechanism; once
an `--efuse-state` file exists, persistent physical state takes precedence.

### S4 — reset behavior

The current ESP32-S3 reboot path already preserves `flash` and `efuse` while
recreating digital peripherals. Add regression tests proving that:

- flash and physical eFuse blocks survive software reset;
- staging registers and active eFuse commands do not survive reset;
- the post-reset read shadows are rebuilt from physical blocks;
- reset flushes pending native writes before ROM execution restarts.

### S5 — native CLI integration

Both state flags are parsed by the common CLI and enabled only for the
ESP32-S3, whose physical eFuse mapping is implemented. C3/C6 combinations fail
before execution rather than silently running ephemerally.

CLI tests cover:

- missing state creates it from seeds;
- existing state suppresses reseeding;
- exact-size validation;
- unwritable paths and failed atomic initialization;
- no state flags retain today's ephemeral behavior;
- source firmware images remain byte-identical after a run.

### S6 — browser persistence

The WASM machine remains filesystem-independent. Export narrow functions that
copy flash/eFuse state and report dirty generations. The page/worker owns an
IndexedDB backend.

Browser behavior:

- state identity is explicit, based on a user-visible profile name plus chip;
- a new profile is initialized from the selected firmware manifest/files;
- dirty state is saved after a short debounce and on `pagehide`/
  `visibilitychange` where possible;
- Run/Reset never silently discards persistent state;
- Export downloads flash and eFuse state separately;
- Import validates chip, type and exact length before replacing a profile;
- Clear state requires an explicit confirmation and affects only that profile.

Do not use `localStorage` for binary state. IndexedDB is required because flash
images are large and writes are asynchronous.

## Test plan

### Unit tests

- dirty-range merge and clipping;
- atomic first-state creation;
- flash size validation;
- program preserves NOR `1 -> 0` behavior;
- erase restores `0xff` and persists it;
- eFuse serialization round trip;
- repeated burns can only add bits;
- protected words cannot be changed;
- read-protected words are hidden from guest reads;
- no log/error string contains eFuse payload bytes.

### Native restart tests

Each restart test launches two separate emulator processes:

1. create state from a small public test firmware/image;
2. make the guest write flash or burn a non-sensitive test fuse;
3. stop the first process;
4. start a second process with the same state;
5. verify the guest observes the changed value.

Required scenarios:

- NVS-like page writes survive restart;
- sector erase survives restart;
- repeated start does not reapply the original seed;
- eFuse burn survives restart;
- attempted eFuse bit clearing does not change state;
- guest software reset preserves both devices;
- a failed/aborted state initialization leaves no apparently valid partial
  state file;
- firmware seed files hash identically before and after every test.

### Browser tests

- create, reload and resume one profile;
- profiles do not share state;
- export, clear and import restores the same state;
- malformed or wrong-sized imports are rejected without replacing good state;
- rapid flash mutations coalesce without losing the final bytes.

## Delivery sequence

1. S1 storage abstraction and raw flash-state initialization.
2. S2 complete flash dirty receipts and native write-through.
3. Native cross-process flash/NVS tests.
4. S3 physical eFuse refactor, format fixture and immediate persistence.
5. Native eFuse protection and restart tests.
6. S4 reset regressions and failure-path hardening.
7. S5 CLI documentation and examples.
8. S6 IndexedDB profiles, import/export and browser tests.

Each step should be independently reviewable. Do not combine the physical
eFuse refactor, CLI persistence and browser UI in one patch.

## Completion criteria

This workstream is complete when:

- ordinary ESP-IDF NVS writes survive a separate esp32sim process;
- flash erase/program operations retain NOR behavior and survive restart;
- ESP32-S3 eFuse burns survive restart and enforce one-way/protection rules;
- guest software reset preserves durable state without preserving transient
  controller registers;
- immutable firmware/seed inputs are never modified;
- native and browser state have documented reset, clear and import behavior;
- no product-specific data or secret material exists in the public repository;
- UART/TCP provisioning has not been introduced as part of this work.
