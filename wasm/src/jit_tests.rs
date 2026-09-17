//! Real-machine regressions for scheduler exits; invoked only by the WASM test build.
use esp_soc::{ScriptAction, SocBus, Stop};
use xtensa_lx7::{Bus, Core};
const BASE: u32 = 0x4037_0000;
const CONTROL: u32 = 0x600c_0000;
const RTC_CNTL: u32 = 0x6000_8000;
// addi.n a3,a3,1; s32i a5,a4,0; j back
const LOOP: [u8; 8] = [0x1b, 0x33, 0x52, 0x64, 0x00, 0xc6, 0xfd, 0xff];
fn machine(jit: bool) -> esp32s3::Machine {
    let mut m = esp32s3::machine([1, 2, 3, 4, 5, 6]);
    m.console.capture = true;
    SocBus::load_bytes(&mut m.bus, BASE, &LOOP).unwrap();
    SocBus::load_bytes(
        &mut m.bus,
        0x4000_0400,
        &[0x00, 0x70, 0x00, 0x06, 0xff, 0xff],
    )
    .unwrap();
    for c in &mut m.cores {
        c.set_jit(jit);
    }
    let c = &mut m.cores[0];
    c.pc = BASE;
    c.ps = 0;
    c.set_ar(4, BASE + 0x400);
    c.set_ar(5, 0);
    m.max_cycles = 4096;
    assert!(matches!(m.run(u64::MAX), Stop::Halted));
    if jit {
        assert!(
            m.cores[0].blocks.jit_instructions > 100,
            "machine never entered compiled code"
        );
    }
    m
}
fn same(a: &esp32s3::Machine, b: &esp32s3::Machine) {
    assert_eq!(a.bus.cycles, b.bus.cycles);
    assert_eq!(a.script.pos, b.script.pos);
    assert_eq!(a.console.all, b.console.all);
    assert_eq!(a.console.uart0, b.console.uart0);
    for (a, b) in a.cores.iter().zip(&b.cores) {
        assert_eq!(a.pc, b.pc);
        assert_eq!(a.ar, b.ar);
        assert_eq!(a.ps, b.ps);
        assert_eq!(a.ccount, b.ccount);
        assert_eq!(a.insn_count, b.insn_count);
        assert_eq!(a.interrupt, b.interrupt);
        assert_eq!(a.waiting(), b.waiting());
        assert_eq!(a.windowbase, b.windowbase);
        assert_eq!(a.epc, b.epc);
    }
    assert_eq!(a.bus.rtc_slow, b.bus.rtc_slow);
    assert_eq!(a.bus.ulp_fsm.cpu, b.bus.ulp_fsm.cpu);
    assert_eq!(a.bus.ulp_fsm.wake_requests, b.bus.ulp_fsm.wake_requests);
    assert_eq!(a.bus.ulp_fsm.traps, b.bus.ulp_fsm.traps);
    assert_eq!(a.bus.ulp_fsm.decode_hits, b.bus.ulp_fsm.decode_hits);
    assert_eq!(a.bus.ulp_fsm.decode_misses, b.bus.ulp_fsm.decode_misses);
    assert_eq!(a.bus.ulp_riscv.cpu.pc, b.bus.ulp_riscv.cpu.pc);
    assert_eq!(a.bus.ulp_riscv.cpu.x, b.bus.ulp_riscv.cpu.x);
    assert_eq!(a.bus.ulp_riscv.cpu.mcause, b.bus.ulp_riscv.cpu.mcause);
    assert_eq!(a.bus.ulp_riscv.cpu.insn_count, b.bus.ulp_riscv.cpu.insn_count);
    assert_eq!(a.bus.ulp_riscv.cpu.retired_count, b.bus.ulp_riscv.cpu.retired_count);
    assert_eq!(a.bus.ulp_riscv.cpu.cycle_count, b.bus.ulp_riscv.cpu.cycle_count);
    assert_eq!(a.bus.ulp_riscv.wake_requests, b.bus.ulp_riscv.wake_requests);
    assert_eq!(a.bus.ulp_riscv.traps, b.bus.ulp_riscv.traps);
    assert_eq!(a.bus.periph.rtc.interrupt_status(), b.bus.periph.rtc.interrupt_status());
}
pub fn run() -> u32 {
    let (mut a, mut b) = (machine(false), machine(true));
    for m in [&mut a, &mut b] {
        m.bus.periph.uart[0].tx_out.extend(b"compiled stop\n");
        m.script
            .events
            .push((m.bus.cycles + 64, ScriptAction::Stop));
        m.max_cycles = u64::MAX;
        assert!(matches!(m.run(u64::MAX), Stop::Halted));
        assert!(
            m.console.all.ends_with(b"compiled stop\n"),
            "script stop did not drain console"
        );
    }
    same(&a, &b);
    let (mut a, mut b) = (machine(false), machine(true));
    // The compiled store first releases the peer, then holds it while it is asleep
    // with a CCOMPARE deadline in the same round. Compare that round and the next.
    for value in [2, 0] {
        let before = b.cores[0].blocks.jit_instructions;
        for m in [&mut a, &mut b] {
            m.cores[0].pc = BASE;
            m.cores[0].set_ar(4, CONTROL);
            m.cores[0].set_ar(5, value);
            if value == 0 {
                let c = &mut m.cores[1];
                c.ps = 0;
                c.ccompare[0] = c.ccount + 1;
                c.intenable = 1 << 6;
            }
            m.max_cycles = m.bus.cycles + 64;
            assert!(matches!(m.run(u64::MAX), Stop::Halted));
        }
        assert!(
            b.cores[0].blocks.jit_instructions > before,
            "MMIO store missed compiled block"
        );
        same(&a, &b);
        for m in [&mut a, &mut b] {
            m.max_cycles = m.bus.cycles + 64;
            assert!(matches!(m.run(u64::MAX), Stop::Halted));
        }
        same(&a, &b);
    }
    let (mut a, mut b) = (machine(false), machine(true));
    let words = [0x7481_2340_u32, 0x7480_00a1, 0x6800_0184, 0x9000_0001, 0xb000_0000];
    let program: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    for m in [&mut a, &mut b] {
        SocBus::load_bytes(&mut m.bus, esp32s3::bus::RTC_SLOW_LOW, &program).unwrap();
        m.bus.periph.rtc.write(0x104, (1 << 27) | (1 << 23));
        m.bus.periph.rtc.write(0x100, (1 << 30) | (1 << 28) | (512 << 11) | 512);
        m.max_cycles = m.bus.cycles + 1_000;
        assert!(matches!(m.run(u64::MAX), Stop::Halted));
    }
    same(&a, &b);
    assert_eq!(a.bus.ulp_fsm.wake_requests, 1);
    const ULP_RISCV: &[u8] = include_bytes!("../../esp32s3/tests/fixtures/ulp-riscv/esp-idf-v5.2.1-test-app.bin");
    let (mut a, mut b) = (machine(false), machine(true));
    for m in [&mut a, &mut b] {
        SocBus::load_bytes(&mut m.bus, esp32s3::bus::RTC_SLOW_LOW, ULP_RISCV).unwrap();
        m.bus.write32(esp32s3::bus::RTC_SLOW_LOW + 0x1d0, 4).unwrap();
        m.bus.write32(RTC_CNTL + 0x104, (1 << 27) | (1 << 24) | (1 << 22) | 1).unwrap();
        m.bus.write32(RTC_CNTL + 0x100, (1 << 30) | (512 << 11) | 512).unwrap();
        m.max_cycles = m.bus.cycles + 20_000;
        assert!(matches!(m.run(u64::MAX), Stop::Halted));
    }
    same(&a, &b);
    assert_eq!(u32::from_le_bytes(a.bus.rtc_slow[0x1c8..0x1cc].try_into().unwrap()), 4);
    assert_eq!(u32::from_le_bytes(a.bus.rtc_slow[0x1cc..0x1d0].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(a.bus.rtc_slow[0x1e0..0x1e4].try_into().unwrap()), 1);
    assert_eq!(a.bus.ulp_riscv.traps, 0);
    let (mut a, mut b) = (machine(false), machine(true));
    for m in [&mut a, &mut b] {
        SocBus::load_bytes(&mut m.bus, esp32s3::bus::RTC_SLOW_LOW, ULP_RISCV).unwrap();
        m.bus.write32(esp32s3::bus::RTC_SLOW_LOW + 0x1d0, 6).unwrap();
        m.bus.write32(RTC_CNTL + 0x104, (1 << 27) | (1 << 24) | (1 << 22) | 1).unwrap();
        m.bus.write32(RTC_CNTL + 0x100, (1 << 30) | (512 << 11) | 512).unwrap();
        m.bus.tick(40_000_000);
    }
    same(&a, &b);
    assert_eq!(u32::from_le_bytes(a.bus.rtc_slow[0x1dc..0x1e0].try_into().unwrap()), 100_000);
    assert_eq!(a.bus.ulp_riscv.traps, 0);
    6
}
