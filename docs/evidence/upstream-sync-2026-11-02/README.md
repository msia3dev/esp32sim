# Fork/upstream integration acceptance — 2026-11-02

Fork input `52fb1826c79a804d748ae34bf917e15c6b02c2ea`; upstream input and CPU
baseline `9fdeb323837d090b038a1036d2f344566546d466`; merge base
`4ab7e900fee998d0137c2c57d59de9d05339d093`. This combines the existing fork
contracts with upstream v0.3.0. It is not an isolated optimization or hardware
timing measurement. Related catalog entries: EX201 (S3 pin transport), EX203
(UART endpoints), EX205/EX206 (C3/C6 transport), EX210 (offline S3 PHY follow-up),
and the unmerged upstream EX236/PR #263 persistence proposal.

The initial merge had 32 conflicted files. Resolutions preserve upstream GPIO,
UART routes, interrupt status, chip/revision image validation and reset retention,
while adapting the fork's ULP, persistence, secure-first-boot, download mode and
panel capabilities. See [the source assessment](../../sync-review-2026-11-02.md)
for the feature-by-feature decision table.

Environment: macOS, arm64, Rust stable 1.99.1. Public ROMs, P4 images and
MicroPython were fetched with `tools/fetch-demo-assets.sh --no-linux`, which pins
input hashes. Pocket Tank uses `CI=1 tools/fetch-pocket-tank.sh` and its pinned
model hash. Esptool checks use a temporary Python environment with esptool 4.8.1;
no environment directory, private firmware or raw captures are committed.

Validation is recorded below after completion. The first sandboxed native run
passed its non-socket tests until the esp-soc library reported 22 socket-binding
permission errors; this is retained as an environment failure, followed by a run
with loopback socket access. A custom-target WASM build compiled successfully but
its wrapper's hardcoded copy path failed; the artifact was copied to that path
for preliminary smoke tests. Final validation uses the standard build path.

Semantic failures retained during integration: the first unrestricted native run
found the classic UART FIFO-sticky interrupt regression; it was resolved with an
explicit adapter policy while retaining modern download-mode level behavior. A
library run found the old counter-alias test assumed globally enabled counters;
that regression now opts into upstream's `user_counters` capability, as the ULP
wrapper does. The initial StopWatch UI harness omitted the new panel-control
function dependency; its scoped mock was updated, and a rotation-precedence test
now checks null vs explicit zero, manifest fallback, board hints and URL overrides.

Additional directed checks cover distinct write-effect bits, classic/S3
coprocessor reset policies, bounded malformed manual-encryption requests, octal
flash identity through physical eFuse export/reset, and loading a persistent S3
state after selecting 16 MiB capacity. The capacity check prevents an automatically
merged early state-load call from using the default 8 MiB before resizing.

A later native integration run exposed a second chip-boundary regression: shared
S3 ULP reset defaults initialized classic RTC CRC result registers at +0x100/+0x104.
ULP register defaults, command semantics and ticking are now explicitly enabled
only for the S3 constructor. Both classic CRC integration regressions pass after
that correction. The debug library suite passed before this final scoping change;
final release checks below cover the corrected tree.

The ULP documentation also claimed dedicated WASM fixture execution, but no such
fixture invocation exists in the retained WASM suites. Native wake/shared-lock
fixtures are exercised; WASM compilation and general smoke/JIT tests are separate
checks. The claim was narrowed rather than treating those checks as ULP parity.

The first full release pass completed with 1,538 passing tests and eight failures:
S3 reset-value expectation (1), classic calibration output/work (3), and ordinary
pin-routed UART sticky status (4). Corrections retain the S3 reset policy, preserve
classic calibration in its own adapter, and opt into FIFO-level behavior only for
the raw TCP adapter. Focused classic PSRAM/reset and all eight UART endpoint tests
pass after correction; no golden outputs were regenerated.

The initial esptool environment resolved an incompatible bitstring 5.0.0. Its
removed `BitStream` API prevented espefuse from importing; an S3 compressed-flash
run also failed in that environment. Pinning bitstring 4.3.1 with esptool 4.8.1
produced six passing flash/boot and eFuse-restart checks across S3/C3/C6. This does
not isolate the earlier S3 failure's cause; final checks repeat on the final tree.

## Final correctness results

- `ESP32SIM_ROM_DIR=<absolute-public-rom-directory> cargo test --release --workspace --no-fail-fast -- --include-ignored --skip external_`: **1,546 passed, 0 failed**, 148 test/doc-test targets. Golden files remain byte-identical to upstream.
- `ESP32SIM_VQ_NATIVE=1 cargo test --release -p esp32s3 --test machine --test virtual_stops`: **64 passed**, including ULP execution/order/invalidation cases.
- Native all-target Clippy and release WASM `jit-tests` Clippy both pass with `-D warnings`.
- Production `tools/wasm-build.sh`, section check (no DWARF; name section retained) and all **13** prescribed WASM demo smoke tests pass on the resolved tree.
- Timing/BLE observation and connection ABI checks pass. Worker pacing/channel checks pass. The browser/touch/StopWatch/build-policy suites pass **28** tests; Python benchmark harness tests pass **15** tests with Python 3.14.7 (the system Python 3.9 lacks required `tomllib`). Retained x5 receipt verifier suites and their verification commands pass.

An additional C3 flash run timed out awaiting the first compressed block response
while native/WASM builds and smoke tests ran concurrently (emulator still alive).
Five of six external checks passed in that run. Earlier, all six passed. This
negative result is retained; the quiet final rerun is recorded below. No raw
serial stream, host inventory or private temporary-state path is committed.

The first WASM differential invocation used local Node 22.22.2 and was stopped
without a verdict after more than five minutes. A one-second diagnostic sampled
WASM instance/table bookkeeping, not idle waiting; its numeric summary, raw hash
and redactions are in `jit-diagnostic.json`. The raw diagnostic was removed. A
profiling build also wrote the shared test-artifact path during that invocation;
therefore the final differential check is rebuilt and run sequentially under
CI's Node 24 major version, after profiling compilation. No browser speed claim
is inferred from this runtime change or the incomplete invocation.

## Disabled-feature guard follow-up (EX114 / EX203)

The first three alternating hello pairs on merge `60832194` retained identical
per-core work, cycles and console hashes but measured median CPU increases of
7.09% (S3), 4.21% (C3) and 1.72% (C6); the full samples are preserved in
`cpu-initial.json`, with `source-hashes.json` identifying that source. The full
30-second Pocket Tank campaign was stopped during its long-running warmup phase,
without a Pocket Tank verdict. Its original harness is retained at `60832194`.

This is a narrow EX114 retry on the combined merge, cross-referenced with EX203's
opt-in transport contract: ULP helper setup is skipped only when the current
controller state is Off and both private engine-active flags are false. Reset,
Running, WakeDelay and active-engine cleanup still enter the existing path. No
public statistics counter is used as a lifetime invariant. All peripheral clocks
and controller deadlines still run. A missing UART TCP bridge is checked at the
hot call site before entering its polling helper.

The follow-up Pocket Tank comparison explicitly limits total instructions while
retaining the 30-second upper bound. It is a bounded prefix, not a full inference
throughput measurement. The harness records its exact arguments, input hashes,
per-core counts, cycles, console hashes, alternating order and numeric CPU samples.
No quiet-machine load gate is imposed; no other task builds/tests run concurrently.
