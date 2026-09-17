# ESP32-S3 ULP support

esp32sim executes both ESP32-S3 ultra-low-power coprocessor architectures from the same RTC
slow memory used by the main CPUs. The implementation is generic: no board address, firmware
flag or application-specific waveform is embedded in either engine.

## Architecture

`esp-periph::UlpController` owns the register-facing lifecycle: architecture selection, clock
gate, reset, force start, wake timer, entry point, halt and repeated timer runs. It remains
separate from instruction semantics.

The ULP-FSM architecture is implemented by the standalone `ulp-fsm` crate. `esp32s3::ulp`
adapts its word-addressed memory and RTC peripheral operations to the SoC timeline. Instructions
are fetched once, complete at their documented cycle boundary and are cached by the same
256-byte RTC-memory page versions used to invalidate main-CPU code. Long `WAIT` instructions
remain one retirement and one exact completion deadline.

The ULP RISC-V architecture reuses `riscv-rv32` for RV32IMC decode and integer execution.
`esp32s3::ulp_riscv` exposes only the hardware-visible address space:

| ULP address | Target |
| --- | --- |
| `0x0000..0x1fff` | RTC slow memory, program and data |
| `0x8000..0x83ff` | RTC_CNTL converted register window |
| `0xa400..0xa7ff` | RTC_IO converted register window |
| `0xc800..0xcbff` | SENS converted register window |
| `0xec00..0xefff` | RTC_I2C converted register window |

Other accesses fault and assert the RTC coprocessor-trap interrupt. The wrapper provides the
standard read-only cycle/instret CSR aliases used by ESP-IDF, resets at PC 0 on each run, and
supports the LR/SC and AMO operations used by ESP-IDF's shared lock implementation.

Both engines use the programmed RTC fast-clock source: XTAL/2 is 20 MHz; RC_FAST is modelled
as 17.5 MHz with its register divider. Main-CPU RTC accesses first advance pending ULP work to
the shared time horizon. ULP stores bump code-page versions, and RTC GPIO edges retain their
instruction-completion timestamps during normal scheduled execution.

## Interrupts and lifecycle

The RTC core is ESP32-S3 interrupt source 39. `RTC_CNTL_INT_RAW`, `INT_ENA`, `INT_ST`, `INT_CLR`
and enable W1TS/W1TC semantics are modelled for the ULP signals:

- bit 5: ULP-FSM `WAKE`;
- bit 13: ULP RISC-V software wake request;
- bit 17: ULP RISC-V trap.

An enabled source wakes an Xtensa core parked in `waiti`. Halting while the ULP timer remains
enabled schedules the next run; disabling the timer prevents later starts without aborting the
instruction already running. RISC-V DONE/reset writes follow the controller lifecycle used by
ESP-IDF's startup and halt routines.

## Diagnostics and tests

`--debug ulp` prints controller timer/start/halt events. The final machine report keeps separate
ULP-FSM and ULP RISC-V instruction, cycle, trap and wake-request totals; they do not consume the
main-core instruction limit.

The test tiers are:

```sh
cargo test -p ulp-fsm
cargo test -p esp32s3 --test machine ulp_
cargo test --workspace
tools/wasm-jit-test.sh
```

The checked-in ULP RISC-V fixture is the unchanged ESP-IDF v5.2.1 public test binary. Native and
WebAssembly tests run its wake path and its 100,000-iteration shared-lock command. ULP-FSM ALU
flags were additionally compared with an ESP32-S3-WROOM board; the recorded hardware result is
under `ulp-fsm/tests/hardware/`.

## Current limits

- The simulator has no main-CPU deep/light-sleep power-state model. ULP interrupts wake a
  running or `waiti` core, but deep-sleep reset and `esp_sleep_get_wakeup_cause()` are deliberately
  not fabricated.
- RTC-I2C instruction/register behavior uses the configurable RTC-I2C register bank; routing a
  transaction into a board's external I2C device graph is not implemented.
- ULP RISC-V GPIO wake as a controller start source and main-CPU-to-ULP internal interrupts are
  not implemented.
- There is no ULP native JIT, auxiliary ULP ELF symbol loader or per-instruction ULP CLI trace.
  The page-versioned FSM decode cache and bounded instruction deadlines are the current
  performance path.
