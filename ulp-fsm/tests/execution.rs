use ulp_fsm::{decode, step, Bus, Cpu, Event, Trap};

#[derive(Clone)]
struct Ram {
    words: Vec<u32>,
    writes: Vec<(u16, u32)>,
}
impl Ram {
    fn new(words: &[u32]) -> Self {
        Self {
            words: words.to_vec(),
            writes: Vec::new(),
        }
    }
}
impl Bus for Ram {
    type Error = u16;
    fn read_word(&mut self, address: u16) -> Result<u32, Self::Error> {
        self.words.get(address as usize).copied().ok_or(address)
    }
    fn write_word(&mut self, address: u16, value: u32) -> Result<(), Self::Error> {
        let word = self.words.get_mut(address as usize).ok_or(address)?;
        *word = value;
        self.writes.push((address, value));
        Ok(())
    }
}

#[test]
fn assembled_program_moves_stores_loads_and_halts() {
    let mut words = vec![0; 16];
    words[..5].copy_from_slice(&[
        0x7481_2340,
        0x7480_00a1,
        0x6800_0184,
        0xd000_0006,
        0xb000_0000,
    ]);
    let mut ram = Ram::new(&words);
    let mut cpu = Cpu::new(0);
    for _ in 0..4 {
        assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
    }
    assert_eq!(ram.words[10] & 0xffff, 0x1234);
    assert_eq!(cpu.regs[2], 0x1234);
    assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Halt));
    assert!(cpu.halted);
    assert_eq!((cpu.insn_count, cpu.cycle_count), (5, 30));
}

#[test]
fn arithmetic_flags_drive_absolute_branches() {
    let mut ram = Ram::new(&[
        0x7420_0013,
        0x8480_0010,
        0x7480_0010,
        0x7480_0020,
        0xb000_0000,
    ]);
    let mut cpu = Cpu::new(0);
    assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
    assert!(cpu.overflow);
    assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
    assert_eq!(cpu.pc, 4, "overflow branch selected word 4");
}

#[test]
fn non_arithmetic_alu_operations_preserve_overflow() {
    let operations = [
        0x7080_001c, // move r0, r3
        0x7040_001c, // and r0, r3, r1
        0x7060_001c, // or r0, r3, r1
        0x74a0_001c, // lsh r0, r3, 1
        0x74c0_001c, // rsh r0, r3, 1
    ];
    for operation in operations {
        let mut ram = Ram::new(&[
            0x7420_0013,
            operation,
            0x8480_0010,
            0xb000_0000,
            0xb000_0000,
        ]);
        let mut cpu = Cpu::new(0);
        cpu.regs[1] = 1;
        cpu.regs[3] = 2;
        assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
        assert!(cpu.overflow);
        assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
        assert!(cpu.overflow, "operation {operation:08x}");
        assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
        assert_eq!(cpu.pc, 4, "operation {operation:08x}");
    }
}

#[test]
fn stage_and_relative_branch_use_word_pc_offsets() {
    let mut ram = Ram::new(&[0x7800_0030, 0x8808_8005, 0x7480_0010, 0xb000_0000]);
    let mut cpu = Cpu::new(0);
    assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
    assert_eq!(cpu.stage, 3);
    assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
    assert_eq!(cpu.pc, 3);
}

#[test]
fn illegal_instruction_traps_without_retiring() {
    let mut ram = Ram::new(&[0]);
    let mut cpu = Cpu::new(0);
    assert_eq!(
        step(&mut cpu, &mut ram),
        Err(Trap::Illegal { pc: 0, raw: 0 })
    );
    assert_eq!((cpu.pc, cpu.insn_count, cpu.cycle_count), (0, 0, 0));
}

#[test]
fn decoded_cycle_costs_match_the_esp_idf_instruction_reference() {
    assert_eq!(decode(0x7481_2340).cycles(), Some(6));
    assert_eq!(decode(0x6800_0d8e).cycles(), Some(8));
    assert_eq!(decode(0x8400_000c).cycles(), Some(4));
    assert_eq!(decode(0x4000_0011).cycles(), Some(23));
    assert_eq!(decode(0xb000_0000).cycles(), Some(2));
    assert_eq!(
        decode(0x5000_0011).cycles(),
        None,
        "ADC timing depends on RTC peripheral registers"
    );
}

#[test]
fn automatic_stores_toggle_halves_increment_offsets_and_write_metadata() {
    let mut words = vec![0; 16];
    words[..5].copy_from_slice(&[
        0x6600_0400,
        0x6200_0189,
        0x6200_0189,
        0x6200_0013,
        0xb000_0000,
    ]);
    let mut ram = Ram::new(&words);
    let mut cpu = Cpu::new(0);
    cpu.regs = [8, 0x1234, 8, 0xabcd];
    while step(&mut cpu, &mut ram).unwrap() != Event::Halt {}

    assert_eq!(ram.words[9], 0x1234_1234);
    assert_eq!(ram.words[10], (3 << 21) | (1 << 16) | 0xabcd);
    assert_eq!(cpu.store_offset, 3);
    assert!(!cpu.store_upper_next);
    assert_eq!((cpu.insn_count, cpu.cycle_count), (5, 34));
}

#[test]
fn signed_load_offset_addresses_the_previous_word() {
    let raw_load_r0_from_r1_minus_one = 0xd01f_fc04;
    let mut words = vec![0; 8];
    words[0] = raw_load_r0_from_r1_minus_one;
    words[1] = 0xb000_0000;
    words[4] = 0x1234_5678;
    let mut ram = Ram::new(&words);
    let mut cpu = Cpu::new(0);
    cpu.regs[1] = 5;
    assert_eq!(step(&mut cpu, &mut ram), Ok(Event::Continue));
    assert_eq!(cpu.regs[0], 0x5678);
}
