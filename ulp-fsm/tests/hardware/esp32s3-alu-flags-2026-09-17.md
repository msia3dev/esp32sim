# ESP32-S3 ULP-FSM ALU flag differential — 2026-09-17

Purpose: resolve behavior not fully specified by the ESP-IDF ULP instruction reference: whether
non-arithmetic ALU instructions clear or preserve the overflow flag.

Hardware and software:

- ESP32-S3 QFN56 revision v0.2, 40 MHz crystal.
- ESP-IDF v5.2.1.
- RTC slow clock: internal RC (`CONFIG_RTC_CLK_SRC_INT_RC`).
- Main CPU: 160 MHz.
- Standalone, unencrypted test application; no MILA firmware or data.
- Application binary SHA-256: `4809ecefba83744a67a8059c69521d84d40c6d32369dab760736df99e815454e`.
- Persisted 52-byte result record SHA-256: `c60f62316dc150dbe87434f0e5cab86c059d0b2ae2ed923427e49189b4ad8b97`.

The ULP program set overflow with `SUB 0 - 1`, executed the named operation, then immediately
recorded whether `JUMP ..., OV` was taken. Zero tests similarly recorded `JUMP ..., EQ`.

| Case | Observed |
|---|---:|
| SUB borrow sets overflow | 1 |
| ADD carry sets overflow | 1 |
| MOVE preserves overflow | 1 |
| AND preserves overflow | 1 |
| OR preserves overflow | 1 |
| LSH preserves overflow | 1 |
| RSH preserves overflow | 1 |
| MOVE zero sets zero | 1 |
| AND zero sets zero | 1 |
| OR nonzero clears zero | 0 |
| ADD wrap sets zero | 1 |
| Completion marker | 1 |

Conclusion: ADD and SUB replace overflow with carry/borrow. MOVE, AND, OR, LSH and RSH preserve
the previous overflow value. All tested ALU operations update the zero flag from their result.
