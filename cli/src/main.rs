//! file: cli/src/main.rs
//! description: command-line interface for MainStage.
//!
//! This binary provides user-facing commands to build, analyze and run
//! MainStage scripts. It wires together the `mainstage_core` APIs, performs
//! plugin discovery, and exposes subcommands for common developer workflows.
//!
use clap::{Arg, ArgMatches, Command};
use console::style;
use log::{Level, error, info, warn};
use mainstage_core::{
    VM, analyze_acyclic_rules, analyze_semantic_rules, ast::generate_ast_from_source,
};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

mod disassembler;

fn main() {
    // Initialize logger with a clean, human-friendly format and colored level tags.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            let lvl = match record.level() {
                Level::Error => style("error").red().bold(),
                Level::Warn => style("warn").yellow().bold(),
                Level::Info => style("info").green().bold(),
                Level::Debug => style("debug").cyan(),
                Level::Trace => style("trace").magenta(),
            };
            writeln!(buf, "{}: {}", lvl, record.args())
        })
        .init();

    let cli = Command::new("MainStage")
        .version("0.1.0")
        .author("Colton McGraw <https://github.com/ColtMcG1>")
        .about("A CLI for MainStage");

    let cli = setup_cli(cli).arg(
        Arg::new("plugin-dir")
            .help("Directory to load plugins from")
            .short('P')
            .long("plugin-dir")
            .value_parser(clap::value_parser!(String))
            .value_name("DIR")
            .global(true),
    )
    .arg(
        Arg::new("stl")
            .help("Select STL plugin(s) to load (repeatable)")
            .long("stl")
            .value_name("NAME")
            .value_parser(clap::value_parser!(String))
            .action(clap::ArgAction::Append)
            .global(true),
    )
    .arg(
        Arg::new("no-stl")
            .help("Do not auto-load the default STL plugin(s)")
            .long("no-stl")
            .action(clap::ArgAction::SetTrue)
            .global(true),
    );

    let matches = cli.get_matches();

    // VM plugin discovery (CLI may override the directory)
    // Resolve plugin-dir against the original CLI CWD, independent of later CWD changes.
    let mut vm = VM::new(vec![]);
    let orig_cli_cwd = std::env::current_dir().ok();
    let plugin_dir: Option<PathBuf> = matches
        .get_one::<String>("plugin-dir")
        .map(|s| {
            let p = PathBuf::from(s);
            if p.is_absolute() {
                p
            } else if let Some(ref cwd) = orig_cli_cwd {
                cwd.join(p)
            } else {
                p
            }
        });
    match vm.discover_plugins(plugin_dir.as_ref()) {
        Ok(n) => info!("Discovered {} plugin manifest(s)", n),
        Err(e) => error!("Plugin discovery failed: {}", e),
    }

    // Clone descriptors map for analyzer usage during CLI commands.
    let manifests_map = vm.plugin_descriptors();

    dispatch_commands(&matches, &manifests_map);
}

/// Sets up the CLI with subcommands and arguments.
/// This function configures the command-line interface using the `clap` crate.
/// It defines subcommands for analyzing scripts and generating reports.
fn setup_cli(cli: Command) -> Command {
    cli.subcommand(
        Command::new("build")
            .about("Build the specified script file")
            .arg(
                Arg::new("file")
                    .help("The script file to build")
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("dump")
                    .help("Specify the dump stage")
                    .short('d')
                    .long("dump")
                    .value_parser(clap::value_parser!(String))
                    .value_name("STAGE"),
            )
            .arg(
                Arg::new("optimize")
                    .help("Enable IR optimization")
                    .short('O')
                    .long("optimize")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("output")
                    .help("Specify the output file")
                    .short('o')
                    .long("output")
                    .value_parser(clap::value_parser!(String))
                    .value_name("FILE"),
            ),
    )
    .subcommand(
        Command::new("run")
            .about("Run a script file")
            .arg(
                Arg::new("file")
                    .help("The script file to run")
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("optimize")
                    .help("Enable IR optimization")
                    .short('O')
                    .long("optimize")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new("trace")
                    .help("Enable tracing of script execution")
                    .short('t')
                    .long("trace")
                    .action(clap::ArgAction::SetTrue),
            ),
    )
    .subcommand(
        Command::new("inspect")
            .about("Disassemble a .msx file")
            .arg(
                Arg::new("file")
                    .help("The .msx file to disassemble")
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("output")
                    .help("Specify the output file for disassembly")
                    .short('o')
                    .long("output")
                    .value_parser(clap::value_parser!(String))
                    .value_name("FILE"),
            ),
    )
}

/// Dispatches the command based on the parsed arguments.
/// This function matches the subcommand used and calls the appropriate handler.
fn dispatch_commands(
    matches: &ArgMatches,
    manifests: &std::collections::HashMap<String, mainstage_core::vm::plugin::PluginDescriptor>,
) {
    match matches.subcommand() {
        Some(("build", sub_m)) => {
            let file = sub_m.get_one::<String>("file").expect("required argument");
            let out = sub_m.get_one::<String>("output");
            let optimize = sub_m.get_flag("optimize");

            // Preserve CLI CWD for argument resolution; set CWD to script dir for core.
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

            // Properly handle the Result so we don't silently drop errors.
            let mut ast = match generate_ast_from_source(&script) {
                Ok(ast) => ast,
                Err(e) => {
                    // Print a helpful message and stop processing this command.
                    error!("Error generating AST: {}", e);
                    return;
                }
            };

            let analysis = match analyze_semantic_rules(&mut ast, Some(manifests)) {
                Ok((_, analysis)) => analysis,
                Err(diags) => {
                    diags
                        .iter()
                        .for_each(|d| error!("Semantic analysis error: {d}"));
                    return;
                }
            };

            if let Err(e) = analyze_acyclic_rules(&ast) {
                error!("Acyclic analysis error: {}", e);
                return;
            }

            let ir_module =
                mainstage_core::ir::lower_ast_to_ir(&ast, optimize, Some(&analysis));

            let bytecode = mainstage_core::ir::emit_bytecode(&ir_module);

            // Resolve output and dump paths relative to original CLI CWD.
            if let Some(output_file) = out {
                let out_rel = std::path::PathBuf::from(output_file);
                let out_path = if out_rel.is_absolute() {
                    out_rel
                } else if let Some(ref cwd) = orig_cwd {
                    cwd.join(out_rel)
                } else {
                    out_rel
                };
                if let Err(e) = fs::write(out_path.with_extension("msx"), &bytecode) {
                    error!("Failed to write output file: {}", e);
                }
            }

            if let Some(dump_stage) = sub_m.get_one::<String>("dump") {
                match dump_stage.as_str() {
                    "ast" => {
                        let dump_path = orig_cwd
                            .as_ref()
                            .map(|d| d.join("dumped_ast.txt"))
                            .unwrap_or_else(|| std::path::PathBuf::from("dumped_ast.txt"));
                        if let Err(e) = fs::write(dump_path, format!("{:#?}", ast)) {
                            error!("Failed to write dumped AST: {}", e);
                        }
                    }
                    "ir" => {
                        let dump_path = orig_cwd
                            .as_ref()
                            .map(|d| d.join("dumped_ir.txt"))
                            .unwrap_or_else(|| std::path::PathBuf::from("dumped_ir.txt"));
                        if let Err(e) = fs::write(dump_path, format!("{}", ir_module)) {
                            error!("Failed to write dumped IR: {}", e);
                        }
                    }
                    _ => {
                        error!("Unknown dump stage: {}", dump_stage);
                    }
                }
            }

            // Restore original working directory if available
            if let Some(orig) = orig_cwd {
                let _ = std::env::set_current_dir(orig);
            }
        }

        Some(("run", sub_m)) => {
            let file = sub_m.get_one::<String>("file").expect("required argument");
            let optimize = sub_m.get_flag("optimize");
            let trace = sub_m.get_flag("trace");

            let script = mainstage_core::script::Script::new(std::path::PathBuf::from(file))
                .expect("Failed to load script file");

            // Properly handle the Result so we don't silently drop errors. 
            let mut ast = match generate_ast_from_source(&script) {
                Ok(ast) => ast,
                Err(e) => {
                    // Print a helpful message and stop processing this command.
                    error!("Error generating AST: {}", e);
                    return;
                }
            };

            let analysis = match analyze_semantic_rules(&mut ast, Some(manifests)) {
                Ok((_, analysis)) => analysis,
                Err(diags) => {
                    diags
                        .iter()
                        .for_each(|d| error!("Semantic analysis error: {d}"));
                    return;
                }
            };

            if let Err(e) = analyze_acyclic_rules(&ast) {
                error!("Acyclic analysis error: {}", e);
                return;
            }

            let ir_module =
                mainstage_core::ir::lower_ast_to_ir(&ast, optimize, Some(&analysis));

            let bytecode = mainstage_core::ir::emit_bytecode(&ir_module);
            // Run the bytecode in the VM. We register external plugin
            // executables before switching the process working directory so
            // relative manifest paths are resolved against the original CWD.
            let orig_cwd = std::env::current_dir().ok();

            // Create the VM now so we can register runtime plugin instances
            // against it before we change the CWD to the script location.
            let mut run_vm = mainstage_core::VM::new(bytecode);

            // Determine STL plugins to load based on CLI flags and availability.
            let no_stl = matches.get_flag("no-stl");
            let mut stl_selection: Vec<String> = Vec::new();
                if let Some(user_stl) = matches.get_many::<String>("stl") {
                stl_selection.extend(user_stl.map(|s| s.to_string()));
            } else if !no_stl {
                    // Default candidate shipped with releases, loaded if present.
                    stl_selection.extend([
                        "stdlib".to_string(),
                    ]);
            }

            // Helper to register a plugin by descriptor name.
            let mut register_by_name = |mod_name: &str, alias: &str| {
                if let Some(desc) = manifests.get(mod_name) {
                    if let Some(dir) = &desc.path {
                        let entry = desc
                            .manifest
                            .entry
                            .clone()
                            .unwrap_or_else(|| desc.manifest.name.clone());
                        let manifest_dir = dir.clone();
                        let mut exe_candidates: Vec<std::path::PathBuf> = Vec::new();
                        if !desc.manifest.path.trim().is_empty() {
                            let mp = std::path::PathBuf::from(desc.manifest.path.clone());
                            let mp_resolved = if mp.is_absolute() { mp } else { manifest_dir.join(mp) };
                            if mp_resolved.is_dir() {
                                let base = mp_resolved.join(&entry);
                                #[cfg(target_os = "windows")]
                                {
                                    let mut p_exe = base.clone(); p_exe.set_extension("exe"); exe_candidates.push(p_exe);
                                    let mut p_dll = base.clone(); p_dll.set_extension("dll"); exe_candidates.push(p_dll);
                                    exe_candidates.push(base);
                                }
                                #[cfg(target_os = "macos")]
                                {
                                    let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); exe_candidates.push(p_dylib);
                                    let mut p_so = base.clone(); p_so.set_extension("so"); exe_candidates.push(p_so);
                                    exe_candidates.push(base);
                                }
                                #[cfg(all(unix, not(target_os = "macos")))]
                                {
                                    let mut p_so = base.clone(); p_so.set_extension("so"); exe_candidates.push(p_so);
                                    let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); exe_candidates.push(p_dylib);
                                    exe_candidates.push(base);
                                }
                            } else {
                                exe_candidates.push(mp_resolved.clone());
                            }
                        }
                        let next_to_manifest = manifest_dir.join(&entry);
                        let push_platform_candidates = |base: &std::path::PathBuf, dst: &mut Vec<std::path::PathBuf>| {
                            #[cfg(target_os = "windows")]
                            {
                                let mut p_exe = base.clone(); p_exe.set_extension("exe"); dst.push(p_exe);
                                let mut p_dll = base.clone(); p_dll.set_extension("dll"); dst.push(p_dll);
                                dst.push(base.clone());
                            }
                            #[cfg(target_os = "macos")]
                            {
                                let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); dst.push(p_dylib);
                                let mut p_so = base.clone(); p_so.set_extension("so"); dst.push(p_so);
                                dst.push(base.clone());
                            }
                            #[cfg(all(unix, not(target_os = "macos")))]
                            {
                                let mut p_so = base.clone(); p_so.set_extension("so"); dst.push(p_so);
                                let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); dst.push(p_dylib);
                                dst.push(base.clone());
                            }
                        };
                        push_platform_candidates(&next_to_manifest, &mut exe_candidates);
                        let crate_root = manifest_dir.parent().map(|p| p.to_path_buf()).unwrap_or(manifest_dir.clone());
                        let cand_debug = crate_root.join("target").join("debug").join(&entry);
                        push_platform_candidates(&cand_debug, &mut exe_candidates);
                        let cand_rel = crate_root.join("target").join("release").join(&entry);
                        push_platform_candidates(&cand_rel, &mut exe_candidates);
                        let mut found: Option<std::path::PathBuf> = None;
                        for c in exe_candidates.into_iter() {
                            match std::fs::metadata(&c) { Ok(meta) if meta.is_file() => { found = Some(c); break; }, _ => continue }
                        }
                        if let Some(exe) = found {
                            let exe_abs = std::fs::canonicalize(&exe).unwrap_or(exe.clone());
                            if desc.manifest.prefers_inprocess() {
                                match mainstage_core::vm::inprocess::InProcessPlugin::new(exe_abs.as_path()) {
                                    Ok(plugin) => { run_vm.register_plugin(std::sync::Arc::new(plugin)); }
                                    Err(e) => {
                                        warn!("failed to load in-process plugin '{}': {}", mod_name, e);
                                        let ep = mainstage_core::vm::external::ExternalPlugin::new(alias.to_string(), exe_abs);
                                        run_vm.register_plugin(std::sync::Arc::new(ep));
                                    }
                                }
                            } else {
                                let ep = mainstage_core::vm::external::ExternalPlugin::new(alias.to_string(), exe_abs);
                                run_vm.register_plugin(std::sync::Arc::new(ep));
                            }
                        } else {
                            warn!("could not locate executable for plugin module '{}' at expected path(s)", mod_name);
                        }
                    } else {
                        warn!("no path specified in manifest for imported module '{}'", mod_name);
                    }
                } else {
                    warn!("no plugin descriptor found for module '{}'", mod_name);
                }
            };

            // Load selected/default STL plugins before processing script imports.
            for name in stl_selection.iter() {
                register_by_name(name, name);
            }

            // Collect imports from the source text as a fallback: alias -> module name
            let mut import_aliases: Vec<(String, String)> = Vec::new();
            // Use the already-loaded script content instead of re-reading the file.
            let src_text = script.display_content().to_string();
            for line in src_text.lines() {
                let s = line.trim();
                if s.starts_with("import ") {
                    // very small parser for: import "mod" as alias;
                    // tolerant to spacing
                    if let Some(rest) = s.strip_prefix("import ") {
                        let parts: Vec<&str> = rest.split_whitespace().collect();
                        if parts.len() >= 3 && parts[1] == "as" {
                            let module = parts[0].trim().trim_matches('"').to_string();
                            let alias = parts[2].trim().trim_end_matches(';').to_string();
                            import_aliases.push((alias, module));
                        }
                    }
                }
            }

            // Register external plugin executables under each alias when present
            for (alias, mod_name) in import_aliases.into_iter() {
                if let Some(desc) = manifests.get(&mod_name) {
                    if let Some(dir) = &desc.path {
                        // The manifest can include a `path` attribute which may point
                        // to the plugin executable (file) or a directory containing
                        // it. Prefer this field when present.
                        let entry = desc
                            .manifest
                            .entry
                            .clone()
                            .unwrap_or_else(|| desc.manifest.name.clone());

                        // canonical base directory for resolving manifest-relative paths
                        let manifest_dir = dir.clone();

                        // If manifest.path (the field inside the JSON) is set, try it first.
                        let mut exe_candidates: Vec<std::path::PathBuf> = Vec::new();
                        if !desc.manifest.path.trim().is_empty() {
                            let mp = std::path::PathBuf::from(desc.manifest.path.clone());
                            let mp_resolved = if mp.is_absolute() {
                                mp
                            } else {
                                // resolve relative manifest.path against the manifest directory
                                manifest_dir.join(mp)
                            };
                            // If `manifest.path` points at a directory (common when
                            // plugins are built into `target/release/`), prefer the
                            // actual plugin file inside that directory (i.e.
                            // `target/release/<entry>`) before considering the
                            // directory itself. This prevents trying to `spawn`
                            // a directory (which leads to "Access is denied").
                            if mp_resolved.is_dir() {
                                // If `manifest.path` is a directory, prefer the
                                // platform-specific artifact names inside it
                                // (e.g. `<entry>.exe`, `<entry>.dll`, etc.) and
                                // avoid adding the directory itself as a
                                // candidate.
                                let base = mp_resolved.join(&entry);
                                #[cfg(target_os = "windows")]
                                {
                                    let mut p_exe = base.clone(); p_exe.set_extension("exe"); exe_candidates.push(p_exe);
                                    let mut p_dll = base.clone(); p_dll.set_extension("dll"); exe_candidates.push(p_dll);
                                    exe_candidates.push(base);
                                }
                                #[cfg(target_os = "macos")]
                                {
                                    let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); exe_candidates.push(p_dylib);
                                    let mut p_so = base.clone(); p_so.set_extension("so"); exe_candidates.push(p_so);
                                    exe_candidates.push(base);
                                }
                                #[cfg(all(unix, not(target_os = "macos")))]
                                {
                                    let mut p_so = base.clone(); p_so.set_extension("so"); exe_candidates.push(p_so);
                                    let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); exe_candidates.push(p_dylib);
                                    exe_candidates.push(base);
                                }
                            } else {
                                exe_candidates.push(mp_resolved.clone());
                            }
                        }

                        // Fallback: prefer executable/library sitting next to manifest
                        // and try platform-specific filename extensions first so
                        // we don't accidentally pick a directory or the wrong
                        // artifact shape on cross-platform hosts.
                        let next_to_manifest = manifest_dir.join(&entry);

                        // Build a platform-prioritized list for the base path.
                        let push_platform_candidates = |base: &std::path::PathBuf, dst: &mut Vec<std::path::PathBuf>| {
                            #[cfg(target_os = "windows")]
                            {
                                let mut p_exe = base.clone(); p_exe.set_extension("exe"); dst.push(p_exe);
                                let mut p_dll = base.clone(); p_dll.set_extension("dll"); dst.push(p_dll);
                                dst.push(base.clone());
                            }
                            #[cfg(target_os = "macos")]
                            {
                                let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); dst.push(p_dylib);
                                let mut p_so = base.clone(); p_so.set_extension("so"); dst.push(p_so);
                                dst.push(base.clone());
                            }
                            #[cfg(all(unix, not(target_os = "macos")))]
                            {
                                let mut p_so = base.clone(); p_so.set_extension("so"); dst.push(p_so);
                                let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); dst.push(p_dylib);
                                dst.push(base.clone());
                            }
                        };

                        push_platform_candidates(&next_to_manifest, &mut exe_candidates);

                        // Also try typical cargo target locations with the same ordering
                        let crate_root = manifest_dir
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or(manifest_dir.clone());
                        let cand_debug = crate_root.join("target").join("debug").join(&entry);
                        push_platform_candidates(&cand_debug, &mut exe_candidates);
                        let cand_rel = crate_root.join("target").join("release").join(&entry);
                        push_platform_candidates(&cand_rel, &mut exe_candidates);

                        // Pick the first candidate that exists and is a file
                        let mut found: Option<std::path::PathBuf> = None;
                        for c in exe_candidates.into_iter() {
                            match std::fs::metadata(&c) {
                                Ok(meta) if meta.is_file() => {
                                    found = Some(c);
                                    break;
                                }
                                _ => continue,
                            }
                        }

                        if let Some(exe) = found {
                            // Try to canonicalize to an absolute path so later
                            // spawns/loads are not affected by CWD changes.
                            let exe_abs = std::fs::canonicalize(&exe).unwrap_or(exe.clone());
                            // If the manifest prefers in-process loading, try
                            // to load the shared library rather than spawn it.
                            if desc.manifest.prefers_inprocess() {
                                match mainstage_core::vm::inprocess::InProcessPlugin::new(exe_abs.as_path()) {
                                    Ok(plugin) => {
                                        run_vm.register_plugin(std::sync::Arc::new(plugin));
                                    }
                                    Err(e) => {
                                        warn!("failed to load in-process plugin '{}': {}", mod_name, e);
                                        // fallback to ExternalPlugin
                                        let ep = mainstage_core::vm::external::ExternalPlugin::new(alias.clone(), exe_abs);
                                        run_vm.register_plugin(std::sync::Arc::new(ep));
                                    }
                                }
                            } else {
                                let ep = mainstage_core::vm::external::ExternalPlugin::new(alias.clone(), exe_abs);
                                run_vm.register_plugin(std::sync::Arc::new(ep));
                            }
                        } else {
                            warn!(
                                "could not locate executable for plugin module '{}' at expected path(s)",
                                mod_name
                            );
                        }
                    } else {
                        warn!(
                            "no path specified in manifest for imported module '{}'",
                            mod_name
                        );
                    }
                } else {
                    warn!(
                        "no plugin descriptor found for imported module '{}'",
                        mod_name
                    );
                }
            }
            // Now that plugin registration is complete, switch the process
            // working directory to the script folder so host functions like
            // `read` and glob-based resolution work relative to the script.
            if let Some(parent) = script.path.parent() {
                if let Err(e) = std::env::set_current_dir(parent) {
                    warn!("failed to set working dir to {:?}: {}", parent, e);
                }
            }

            match run_vm.run(trace) {
                Ok(()) => {}
                Err(e) => {
                    error!("{}", e.lines().collect::<Vec<&str>>().join("\n\t"));
                }
            }

            // Restore original working directory if available
            if let Some(orig) = orig_cwd {
                let _ = std::env::set_current_dir(orig);
            }
        }
        Some(("disasm", sub_m)) => {
            let file = sub_m.get_one::<String>("file").expect("required argument");
            let output_file = sub_m.get_one::<String>("output");

            // Preserve original CLI CWD; set CWD to file dir for reading.
            let orig_cwd = std::env::current_dir().ok();
            let rel_path = std::path::PathBuf::from(file);
            let abs_path = if rel_path.is_absolute() {
                rel_path.clone()
            } else if let Some(ref cwd) = orig_cwd {
                cwd.join(&rel_path)
            } else {
                rel_path.clone()
            };
            if let Some(parent) = abs_path.parent() {
                if let Err(e) = std::env::set_current_dir(parent) {
                    warn!("failed to set working dir to {:?}: {}", parent, e);
                }
            }

            let bytecode = fs::read(&abs_path).expect("Failed to read .msx file");

            match disassembler::disassemble(&bytecode) {
                Ok(f) => {
                    if let Some(output_file) = output_file {
                        let out_rel = std::path::PathBuf::from(output_file);
                        let out_path = if out_rel.is_absolute() {
                            out_rel
                        } else if let Some(ref cwd) = orig_cwd {
                            cwd.join(out_rel)
                        } else {
                            out_rel
                        };
                        if let Err(e) = fs::write(out_path, f) {
                            error!("Failed to write disassembly output file: {}", e);
                        }
                    } else {
                        println!("{}", f);
                    }
                }
                Err(e) => {
                    error!("Failed to disassemble bytecode: {}", e);
                }
            }

            // Restore original working directory if available
            if let Some(orig) = orig_cwd {
                let _ = std::env::set_current_dir(orig);
            }
        }
        _ => {
            error!("No valid subcommand was used. Use --help for more information.");
        }
    }
}
