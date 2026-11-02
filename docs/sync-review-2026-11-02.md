# Upstream synchronization review — 2026-11-02

## Scope and result

Fetched both configured remotes with `git fetch --all --prune` and reviewed the
current upstream against the published fork. This is a source/history assessment,
not a performance experiment or a validated integration build.

| Reference | Reviewed revision |
| --- | --- |
| Local `main` before sync | `d5446b4aa9a1b035aff74eba082611822676f285` |
| `origin/main` | `9fdeb323837d090b038a1036d2f344566546d466` |
| `fork/main` | `52fb1826c79a804d748ae34bf917e15c6b02c2ea` |
| Active `feat/esp32s3-esptool-support` | `49b13e7b4d0a9e96af7aaa0545899644dcbb04e2` |
| Fork/upstream merge base | `4ab7e900fee998d0137c2c57d59de9d05339d093` |

Before sync, local main was an ancestor of upstream with **0 unique / 1,477
behind** commits. It has now been fast-forwarded to the reviewed `origin/main`;
`codex/pre-sync-main-20261102` preserves its previous tip. This updates the local
tracking branch only. No remote branch was pushed.

The published fork still diverges: **43 fork-only / 680 upstream-only** commits,
including merges (41 / 526 excluding merges). All 41 fork-only non-merge patches
are marked `+` by `git cherry origin/main fork/main`: none is an exact patch-ID
match upstream. Semantic duplication still exists, as detailed below.

The active esptool branch differs in commit identities from fork main, but its
tracked tree is identical: `git diff --quiet HEAD fork/main` succeeds, and all
11 active-branch-only commits are patch-equivalent to commits in fork main.
Do not replay that branch as an additional feature series.

Existing local documentation edits and untracked files were left untouched.

## Overlap and implementation choices

“Keep” below means preserve behavior during integration, subject to regression
validation. It does not claim that the old implementation can be copied unchanged
into the new architecture.

| Area and our commits | Current upstream implementation | Preferred integration |
| --- | --- | --- |
| Offline S3 PHY/FE calibration: `6db30505`, `c6f309a2` | `3dfc2474` and cleanup `7bb16623` independently implement the same completion bit, independent of AP presence. Current `FeIq::read` has the same `word \| (1 << 16)` behavior as ours. Upstream adds no-AP/AP-removal tests and qualification evidence. | **Use upstream.** Our two fixes are semantically superseded; retain relevant regression coverage without reintroducing the old state or AP gate. Upstream records this follow-up under EX210. |
| Timestamped GPIO callbacks from GPIO audio: `406e5c86` | Upstream EX201 (`edd985d6` and follow-ups) supplies `gpio_output_at(cycle, changes, enabled, output)`, physical routes, enable transitions, and later waveform callbacks. Our `gpio_changes_at` supplies less context. | **Use upstream transport API.** Adapt ULP edge delivery and audio consumers to it, preserving completion timestamps. The actual `GpioAudio` PCM/high-pass helper and board audio fallback remain unique and should be ported if needed. Upstream I2S/PCM input support does not implement this GPIO-audio contract. |
| ULP-FSM and ULP RISC-V execution: `14a4a004` through `e041c2ca`, plus later cadence/cache fixes | No upstream ULP controller/engine or `ulp-fsm` crate found in the reviewed tree. P4/C6 coprocessor support is a different device. | **Keep our feature**, adapted to current RTC, analog and scheduling APIs. Preserve `5c21ca5e` RTC-write decode invalidation and the later deferred-tick fixes. |
| RISC-V read-only cycle/instret aliases used by our ULP engine: `d9aad1ca` | Upstream P4 work implements the same CSR addresses behind `Cpu::user_counters`, with default C3/C6 behavior preserved. | **Use upstream CSR capability design**, enable it for the ULP wrapper, and verify counter/reset semantics. Do not replace the expanded P4-aware CPU state with our older struct or expose the aliases globally. |
| Flash/eFuse persistence, QEMU eFuse import, NVS across processes: `dc198da5`, `22715e32`, subsequent S3/C3/C6 commits | No `--flash-state` / `--efuse-state`, equivalent persistence backend, or storage export/import ABI found upstream. Upstream SD write-through storage is a separate feature. | **Keep ours** for S3/C3/C6, integrating current reset and flash paths. Preserve native/QEMU formats, seed precedence, one-way burns and write-through error handling. |
| Secure first boot: `0c792e5e`, hardened by storage work | Upstream shared S3 eFuse still ignores the program command; S3 has no equivalent EXT_MEM_ENC handshake, and its narrow peripheral reads do not include the eFuse range. | **Keep our behavior**, preserving upstream octal-flash and memory-access changes. This is a logical decrypted-flash model, not physical ciphertext fidelity or a new security claim. |
| UART TCP download mode, ROM autobaud, modern esptool stubs and C3/C6 lifecycles: `3668a834` through `52fb1826` | Upstream EX203 provides pin-routed UART board endpoints, not our raw UART TCP bridge/download-mode workflow. No equivalent download/autobaud feature found. | **Keep our feature** on top of upstream UART clocks/routes. Revalidate stub execution, flashing, reset and persisted eFuse behavior on all three chips. |
| Configurable partition/app offsets: `eb481d68` | Upstream still writes `--ptable` at `0x8000`, `--app` at `0x10000`, and boots app at `0x10000`. Generic `--flash-at` can place data elsewhere but does not replace the app boot offset. Upstream adds chip-specific bootloader offsets and image validation. | **Combine:** keep our offset flags and app-boot offset while retaining upstream `S::BOOTLOADER_OFFSET`, chip/revision checks and `--flash-at` validation. |
| DCS RGB666/18-bit streams: `14e5d20c`, reset follow-up `5fe6de83` | Upstream stores COLMOD but still consumes pixel data in pairs; it has no equivalent three-byte RGB666 decoder. Both retain RAMCTRL byte-order handling. | **Keep RGB666 decoding**, retain upstream controller behavior, and review reset defaults explicitly. Our ST7789 constructor sets `0x55`, while upstream uses `0x66`; our reset path recreates `Self::new`, so constructor and reset behavior need separate coverage. Neither default should be chosen solely to make a fixture pass. |
| Custom framebuffer reveal, board rotation hints, accessible panel controls: `d20bb591`, `71d35d60`, `8fb0ef7f`, `2b276aff` | Current upstream board trait lacks our rotation/control metadata; no equivalent generic board-control rendering was found. Existing URL/manifest rotation is only a partial alternative. | **Keep the generic capabilities**, adapted to the current web configuration, board APIs and WASM exports. Preserve keyboard activation. |
| Selected RTC calibration clock: `e369d6df` | Upstream TIMG calibration still computes every source from `RTC_SLOW_HZ`. Its real timer stepping, prescaler and alarm logic has advanced substantially. | **Keep our calibration-source fix only**, retaining upstream timer implementation and its fixes. |
| Xtensa coprocessor reset: `52fb1826` | Upstream `Cpu::reset` still sets `cpenable = 0`; ours sets `0xff` and includes a regression. | **Carry for qualification**, checking both S3 and the newly supported classic LX6. There is no upstream replacement, but a shared-core reset policy needs chip-specific validation before calling ours universally better. |
| Checked unsigned division lint: `1ee3f8ae` | Upstream retains an explicit zero-divisor branch with a documented lint allowance; both produce `u32::MAX` for divide-by-zero. | **Keep upstream form.** This is equivalent behavior, not a unique feature to re-port. |

Upstream's broader reboot preservation (`319b58d5` and related work) must also
survive integration: host network/camera settings, analog streams, external GPIO,
octal-flash state, USB connection and audio. Combine that with our physical eFuse
controller reset, persistence flushes and ULP RTC-domain preservation. Replacing
either entire reboot function with the other loses behavior.

### Documentation overstates browser persistence

Our `docs/storage.md` describes an IndexedDB profile UI, import/export and clearing.
The reviewed fork does provide storage generation/export/import WASM functions,
but `web/run.html` contains no IndexedDB/profile/persistence UI implementation.
Do not report browser persistence as complete merely because the documentation
says it exists. Implement and test the frontend separately, or correct the docs
during integration.

## Trial merge

An isolated shared clone outside the working checkout was created for inspection.
The following merge was attempted without committing:

```sh
git checkout -b codex/sync-review-20261102 52fb1826
git merge --no-commit --no-ff 9fdeb323
git diff --name-only --diff-filter=U
```

It produced **32 conflicted files**:

```text
Cargo.toml
README.md
cli/src/lib.rs
docs/cli.md
docs/roadmap.md
docs/wasm.md
esp-periph/src/device.rs
esp-periph/src/lib.rs
esp-periph/src/rtc_cntl.rs
esp-periph/src/spi_mem.rs
esp-periph/src/uart.rs
esp-periph/tests/devices.rs
esp-soc/src/board.rs
esp-soc/src/lib.rs
esp-soc/src/machine.rs
esp-soc/src/machine/web.rs
esp-soc/src/soc.rs
esp32c3/src/bus.rs
esp32c3/src/periph.rs
esp32c3/src/soc.rs
esp32c6/src/bus.rs
esp32c6/src/periph.rs
esp32c6/src/soc.rs
esp32s3/src/bus.rs
esp32s3/src/periph.rs
esp32s3/src/soc.rs
riscv-rv32/src/state.rs
tests/README.md
wasm/src/lib.rs
wasm/src/machine.rs
web/run.html
xtensa-lx7/src/state.rs
```

The temporary inspection clone is `/private/tmp/esp32sim-sync-review-20261102`.
Its merge is unresolved and is **not** a runnable integration candidate. No
conflict markers or merge state were introduced into the user's checkout.
Cleanly merged files still require semantic review, particularly the added ULP
tests, DCS panel code and RISC-V execution changes.

The earlier `sync/main-upstream-2026-09-28` attempt was inspected rather than
reused blindly. Compared with `sync/main-merge-2026-09-28`, it lacks the later
RTC-write invalidation condition, cadence fixes and WASM safety documentation.
Those omissions are already fixed in current fork main. It is not a safe starting
point for this sync.

## Recommended fork integration

1. Create a fresh integration branch from `fork/main` and merge the reviewed
   upstream revision. Preserve both published histories; avoid a rebase/reset of
   published fork main. A merge records which upstream changes have been consumed
   and makes future syncs easier to reason about.
2. Use upstream's current architecture as the basis in each conflict, restoring
   the unique fork contracts from the table. Drop duplicate implementations, not
   their useful regression coverage. Do not select `ours`/`theirs` for entire
   bus, peripheral, CPU-state or web files.
3. Validate native workspace checks and current upstream goldens; exercise ULP
   fixtures, same-cycle ordering, decode invalidation, persistence restart and
   burn protection, download/flash lifecycles on S3/C3/C6, custom display controls,
   RGB565/RGB666 and relevant WASM tests. Follow current repository contribution
   commands and external-input conventions. Existing private-input qualifications
   are not automatically reproducible or replaced by a unit-test pass.
4. If execution/performance comparisons are needed, first search the current
   experiment catalog by mechanism and extend existing records with exact
   work/output checks. This review makes no new speed or timing claim.
5. Review retained evidence and run `node tools/check-evidence-privacy.mjs` before
   publishing. Push a reviewable integration branch/PR, then fast-forward fork
   main only after integration checks pass. No force push is needed. If split
   into dependent PRs, register them using native `gh stack` per AGENTS.md.

Starting from upstream and selectively cherry-picking the fork is possible, but
it would require the same semantic ports while losing the straightforward
ancestry record. With only 41 unique non-merge patches, a history-preserving merge
with deliberate deduplication is the preferred approach.

## Reproduction commands and limits

```sh
git rev-list --left-right --count d5446b4a...9fdeb323
git rev-list --left-right --count 52fb1826...9fdeb323
git cherry 9fdeb323 52fb1826
git diff --quiet 49b13e7b 52fb1826
git cherry 52fb1826 49b13e7b
git log --no-merges --oneline 52fb1826..9fdeb323
git diff --stat 4ab7e900..52fb1826
```

Source comparisons used `git show` and `git grep` at the pinned revisions,
including individual original patches, current definitions, tests and upstream
evidence. No firmware was executed, no performance measurements were taken,
and no integration build/test pass is claimed. The feature table covers the
published fork-main delta; unfinished untracked plans and unrelated unpublished
branches are outside this comparison. This report retains revisions, commands
and conflict paths without host/process inventories or private captures.


## Integration follow-up

Branch: `codex/sync-main-2026-11-02`, based on the pinned fork tip with the pinned
upstream tip as the second merge parent. Both published histories are retained.
The original trial merge above was subsequently resolved in the same isolated
checkout; validation details are retained in
[evidence/upstream-sync-2026-11-02/README.md](evidence/upstream-sync-2026-11-02/README.md).

Unpublished work in the original checkout consisted of a `docs/README.md` link,
`docs/ulp-plan.md`, `docs/mila-firmware-quickstart.md`, IDE project settings and this
new review. Those pre-existing local edits were not folded into the integration.
The older `sync/main-upstream-2026-09-28` branch has 25 commits absent by identity
from remote branches; it contains rebased feature copies and an obsolete
integration adjustment, rather than an additional feature series to replay.

Open upstream PR [#263](https://github.com/mikroverk/esp32sim/pull/263), reviewed
at `b76b165186192d0a90e1054061aa7e75b1f275cc`, implements `--flash-file` and
`--flash-out` but is not in the pinned upstream main. Its EX236 contract saves the
whole image at exit/signal, accepts shorter images padded with erased bytes, and
allows firmware seeds to overlay an existing file. Our `--flash-state` uses an
exact-size device image, ignores seeds when state exists, writes through completed
guest mutations and includes separate physical eFuse persistence. These flags are
not interchangeable. Keep our published contract for this sync; assess optional
aliases/convergence separately if #263 lands.

Resolution details include a separate ULP write-effect bit (upstream already uses
the previous bit for I2S RX configuration), upstream interrupt status publication
on C3/C6, capability-gated ULP RISC-V counters, upstream GPIO output callbacks,
physical eFuse import for octal identity, and the classic chip's original
coprocessor reset policy. Rotation hints are optional so a missing hint preserves
upstream manifest and built-in panel defaults. The inaccurate IndexedDB/UI claims
were removed from both storage and WASM docs. A malformed manual-encryption
request is bounded to the plaintext window rather than indexing past it.


Final source qualification is recorded at `5ccdd990`: 1,546 release tests, 64
native virtual-quantum tests, six quiet esptool/espefuse lifecycles and 13 WASM
demos pass; Node 24 also passes 111,126 differential and 24 waveform cases. The
EX114 follow-up retains narrow inactive-helper guards. Compared with current
upstream, three-pair median CPU remains +2.91% S3 hello / +2.75% C3 hello /
+0.38% C6 hello, with identical guest work/output. The bounded Pocket Tank boot
prefix is +2.00%; full inference throughput was not qualified. These are explicit
remaining costs of the combined feature tree, not a performance-parity result.
