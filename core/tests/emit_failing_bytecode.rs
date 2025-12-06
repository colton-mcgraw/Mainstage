use std::path::PathBuf;
use mainstage_core::{ast, ir, script::Script};

const SAMPLE: &str = r#"[entrypoint]
workspace demo_ws {
    projects = [test_pj];

    for p in projects
    {
        say(test_pj.sources);
        process_project_stage(p);
    }
}

project test_pj {
    sources = ["./samples/e2e/*.ms"];
}

stage load_stage(var)
{
    return read(var);
}

stage process_project_stage(prj)
{
    if prj.sources == null {
        say("No sources found.");
        return;
    }
    in = load_stage(prj.sources[0]);
    say(in);
}
"#;

#[test]
fn emit_bytecode_calllabel_should_have_args() {
    let script = Script {
        name: "test.ms".to_string(),
        path: PathBuf::from("test.ms"),
        content: SAMPLE.to_string(),
    };

    let ast = ast::generate_ast_from_source(&script).expect("failed to parse sample");
    let ir_mod = ir::lower_ast_to_ir(&ast, false, None);
    // Debug print IR to help diagnose missing args
    println!("Lowered IR:\n{}", ir_mod);
    let bytes = ir::emit_bytecode(&ir_mod);

    // Parse bytecode header
    assert!(bytes.len() > 12, "bytecode too short");
    // version at 4..8, op_count at 8..12
    let op_count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;

    // helpers to parse values similar to VM
    fn read_u32(buf: &[u8], off: &mut usize) -> u32 {
        let v = u32::from_le_bytes(buf[*off..*off+4].try_into().unwrap()); *off += 4; v
    }
    fn read_u64(buf: &[u8], off: &mut usize) -> u64 {
        let v = u64::from_le_bytes(buf[*off..*off+8].try_into().unwrap()); *off += 8; v
    }
    fn read_string(buf: &[u8], off: &mut usize) -> String {
        let len = read_u32(buf, off) as usize; let s = String::from_utf8(buf[*off..*off+len].to_vec()).unwrap(); *off += len; s
    }
    fn skip_value(buf: &[u8], off: &mut usize) {
        let tag = buf[*off]; *off += 1;
        match tag {
            0x01 => { read_u64(buf, off); }
            0x02 => { read_u64(buf, off); }
            0x03 => { *off += 1; }
            0x04 | 0x05 => { let _ = read_string(buf, off); }
            0x06 => { let len = read_u32(buf, off) as usize; for _ in 0..len { skip_value(buf, off); } }
            0x07 => {}
            0x08 => { let len = read_u32(buf, off) as usize; for _ in 0..len { let _k = read_string(buf, off); skip_value(buf, off); } }
            _ => {}
        }
    }

    let mut found_calllabel_with_arg = false;
    let mut i = 0usize;
    let mut off = 12usize;
    while i < op_count && off < bytes.len() {
        let code = bytes[off]; off += 1; i += 1;
        match code {
            0x01 => { let _dest = read_u32(&bytes, &mut off); skip_value(&bytes, &mut off); }
            0x02 => { off += 4; off += 4; }
            0x03 => { off += 4; off += 4; }
            0x10..=0x14 | 0x20..=0x27 => { off += 4+4+4; }
            0x28 => { off += 4; off += 4; }
            0x30|0x31 => { off += 4; }
            0x40 => { let _name = read_string(&bytes, &mut off); }
            0x41 => { off += 4; }
            0x42|0x43 => { off += 4+4; }
            0x50 => {}
            0x60..=0x62 => { off += 4; }
            0x70 => { off += 4+4; let argc = read_u32(&bytes, &mut off) as usize; off += argc*4; }
            0x71 => {
                off += 4; // dest
                off += 4; // label idx
                let argc = read_u32(&bytes, &mut off) as usize;
                if argc == 1 { found_calllabel_with_arg = true; }
                off += argc * 4;
            }
            0x72 => {
                // PluginCall: plugin_name, func_name, argc, args..., has_result (0/1), [result_reg]
                let _pname = read_string(&bytes, &mut off);
                let _fname = read_string(&bytes, &mut off);
                let argc = read_u32(&bytes, &mut off) as usize;
                off += argc * 4;
                let has_result = read_u32(&bytes, &mut off);
                if has_result == 1 { off += 4; }
            }
            0x90 => { off += 4; let len = read_u32(&bytes, &mut off) as usize; off += len * 4; }
            0x91 => { off += 4+4+4; }
            0x92 => { off += 4+4+4; }
            0x95 => { off += 4+4; }
            0x93 => { off += 4+4+4; }
            0x94 => { off += 4+4+4; }
            0x80 => { off += 4; }
            other => panic!("unknown opcode 0x{:02x}", other),
        }
    }

    assert!(found_calllabel_with_arg, "Expected at least one CallLabel with 1 arg, bytecode may have lost args (op_count={})", op_count);
}
