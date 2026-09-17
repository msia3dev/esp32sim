# ESP-IDF ULP RISC-V fixture

`esp-idf-v5.2.1-test-app.bin` is the unchanged `ulp_test_app` binary produced by ESP-IDF
v5.2.1's public `components/ulp/test_apps/ulp_riscv` project for ESP32-S3 on 2026-09-17.

- Size: 468 bytes
- SHA-256: `6c2b2b6be56771d92e9032d8cfef75729d15d813631f05c6abcdaddb1150605b`
- Link address: RTC slow-memory offset 0
- `main_cpu_reply`: `0x1cc`
- `riscv_counter`: `0x1e0`

The fixture is checked in so native and WebAssembly tests exercise the exact same public binary.
