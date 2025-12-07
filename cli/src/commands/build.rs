use clap::ArgMatches;
use log::{error, warn};
use std::fs;

pub fn handle(
    sub_m: &ArgMatches,
    manifests: &std::collections::HashMap<String, mainstage_core::vm::plugin::PluginDescriptor>,
) {
    let file = sub_m.get_one::<String>("file").expect("required argument");
    let out = sub_m.get_one::<String>("output");
    let optimize = sub_m.get_flag("optimize");

    let orig_cwd = std::env::current_dir().ok();
    let rel_script_path = std::path::PathBuf::from(file);
    let script_path_abs = if rel_script_path.is_absolute() {
        rel_script_path.clone()
    } else if let Some(ref cwd) = orig_cwd {
        cwd.join(&rel_script_path)
    } else {
        rel_script_path.clone()
    };
    if let Some(parent) = script_path_abs.parent() {
        if let Err(e) = std::env::set_current_dir(parent) {
            warn!("failed to set working dir to {:?}: {}", parent, e);
        }
    }

    let script = mainstage_core::script::Script::new(script_path_abs.clone())
        .expect("Failed to load script file");

    let mut ast = match mainstage_core::ast::generate_ast_from_source(&script) {
        Ok(ast) => ast,
        Err(e) => { error!("Error generating AST: {}", e); return; }
    };

    let analysis = match mainstage_core::analyze_semantic_rules(&mut ast, Some(manifests)) {
        Ok((_, analysis)) => analysis,
        Err(diags) => { diags.iter().for_each(|d| error!("Semantic analysis error: {d}")); return; }
    };

    if let Err(e) = mainstage_core::analyze_acyclic_rules(&ast) { error!("Acyclic analysis error: {}", e); return; }

    let ir_module = mainstage_core::ir::lower_ast_to_ir(&ast, optimize, Some(&analysis));
    let bytecode = mainstage_core::ir::emit_bytecode(&ir_module);

    if let Some(output_file) = out {
        let out_rel = std::path::PathBuf::from(output_file);
        let out_path = if out_rel.is_absolute() { out_rel } else if let Some(ref cwd) = orig_cwd { cwd.join(out_rel) } else { out_rel };
        if let Err(e) = fs::write(out_path.with_extension("msx"), &bytecode) { error!("Failed to write output file: {}", e); }
    }

    if let Some(dump_stage) = sub_m.get_one::<String>("dump") {
        match dump_stage.as_str() {
            "ast" => {
                let dump_path = orig_cwd.as_ref().map(|d| d.join("dumped_ast.txt")).unwrap_or_else(|| std::path::PathBuf::from("dumped_ast.txt"));
                if let Err(e) = fs::write(dump_path, format!("{:#?}", ast)) { error!("Failed to write dumped AST: {}", e); }
            }
            "ir" => {
                let dump_path = orig_cwd.as_ref().map(|d| d.join("dumped_ir.txt")).unwrap_or_else(|| std::path::PathBuf::from("dumped_ir.txt"));
                if let Err(e) = fs::write(dump_path, format!("{}", ir_module)) { error!("Failed to write dumped IR: {}", e); }
            }
            _ => { error!("Unknown dump stage: {}", dump_stage); }
        }
    }

    if let Some(orig) = orig_cwd { let _ = std::env::set_current_dir(orig); }
}
