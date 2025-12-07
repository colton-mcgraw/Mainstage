use clap::ArgMatches;
use log::{error, warn};
use std::fs;

pub fn handle(sub_m: &ArgMatches) {
    let file = sub_m.get_one::<String>("file").expect("required argument");
    let output_file = sub_m.get_one::<String>("output");

    let orig_cwd = std::env::current_dir().ok();
    let rel_path = std::path::PathBuf::from(file);
    let abs_path = if rel_path.is_absolute() { rel_path.clone() } else if let Some(ref cwd) = orig_cwd { cwd.join(&rel_path) } else { rel_path.clone() };
    if let Some(parent) = abs_path.parent() {
        if let Err(e) = std::env::set_current_dir(parent) { warn!("failed to set working dir to {:?}: {}", parent, e); }
    }

    let bytecode = fs::read(&abs_path).expect("Failed to read .msx file");

    match crate::disassembler::disassemble(&bytecode) {
        Ok(f) => {
            if let Some(output_file) = output_file {
                let out_rel = std::path::PathBuf::from(output_file);
                let out_path = if out_rel.is_absolute() { out_rel } else if let Some(ref cwd) = orig_cwd { cwd.join(out_rel) } else { out_rel };
                if let Err(e) = fs::write(out_path, f) { error!("Failed to write disassembly output file: {}", e); }
            } else {
                println!("{}", f);
            }
        }
        Err(e) => { error!("Failed to disassemble bytecode: {}", e); }
    }

    if let Some(orig) = orig_cwd { let _ = std::env::set_current_dir(orig); }
}
