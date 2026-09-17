use ulp_fsm::{decode, disasm};

#[test]
fn decoder_matches_checked_in_assembler_vectors() {
    let corpus = include_str!("corpus/assembler-vectors.txt");
    let mut count = 0;
    for (line_no, line) in corpus.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (raw, expected) = line
            .split_once('|')
            .unwrap_or_else(|| panic!("bad corpus line {}", line_no + 1));
        let raw = u32::from_str_radix(raw, 16).unwrap();
        let insn = decode(raw);
        assert!(!insn.is_illegal(), "line {}: {raw:08x}", line_no + 1);
        assert_eq!(
            disasm::format(&insn),
            expected,
            "line {}: {raw:08x}",
            line_no + 1
        );
        count += 1;
    }
    assert_eq!(count, 54, "assembler corpus was unexpectedly truncated");
}
