use clap::ArgMatches;
use log::{error, warn};

pub fn handle(
    sub_m: &ArgMatches,
    manifests: &std::collections::HashMap<String, mainstage_core::vm::plugin::PluginDescriptor>,
    matches: &ArgMatches,
) {
    let file = sub_m.get_one::<String>("file").expect("required argument");
    let optimize = sub_m.get_flag("optimize");
    let trace = sub_m.get_flag("trace");

    let script = mainstage_core::script::Script::new(std::path::PathBuf::from(file))
        .expect("Failed to load script file");

    let mut ast = match mainstage_core::ast::generate_ast_from_source(&script) {
        Ok(ast) => ast,
        Err(e) => {
            error!("Error generating AST: {}", e);
            return;
        }
    };

    let analysis = match mainstage_core::analyze_semantic_rules(&mut ast, Some(manifests)) {
        Ok((_, analysis)) => analysis,
        Err(diags) => {
            diags
                .iter()
                .for_each(|d| error!("Semantic analysis error: {d}"));
            return;
        }
    };

    if let Err(e) = mainstage_core::analyze_acyclic_rules(&ast) {
        error!("Acyclic analysis error: {}", e);
        return;
    }

    let ir_module = mainstage_core::ir::lower_ast_to_ir(&ast, optimize, Some(&analysis));
    let bytecode = mainstage_core::ir::emit_bytecode(&ir_module);

    let orig_cwd = std::env::current_dir().ok();
    let mut run_vm = mainstage_core::VM::new(bytecode);

    let no_stl = matches.get_flag("no-stl");
    let mut stl_selection: Vec<String> = Vec::new();
    if let Some(user_stl) = matches.get_many::<String>("stl") {
        stl_selection.extend(user_stl.map(|s| s.to_string()));
    } else if !no_stl {
        stl_selection.extend(["stdlib".to_string()]);
    }

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
                    let mp_resolved = if mp.is_absolute() {
                        mp
                    } else {
                        manifest_dir.join(mp)
                    };
                    if mp_resolved.is_dir() {
                        let base = mp_resolved.join(&entry);
                        #[cfg(target_os = "windows")]
                        {
                            let mut p_exe = base.clone();
                            p_exe.set_extension("exe");
                            exe_candidates.push(p_exe);
                            let mut p_dll = base.clone();
                            p_dll.set_extension("dll");
                            exe_candidates.push(p_dll);
                            exe_candidates.push(base);
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            exe_candidates.push(p_dylib);
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            exe_candidates.push(p_so);
                            exe_candidates.push(base);
                        }
                        #[cfg(all(unix, not(target_os = "macos")))]
                        {
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            exe_candidates.push(p_so);
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            exe_candidates.push(p_dylib);
                            exe_candidates.push(base);
                        }
                    } else {
                        exe_candidates.push(mp_resolved.clone());
                    }
                }
                let next_to_manifest = manifest_dir.join(&entry);
                let push_platform_candidates =
                    |base: &std::path::PathBuf, dst: &mut Vec<std::path::PathBuf>| {
                        #[cfg(target_os = "windows")]
                        {
                            let mut p_exe = base.clone();
                            p_exe.set_extension("exe");
                            dst.push(p_exe);
                            let mut p_dll = base.clone();
                            p_dll.set_extension("dll");
                            dst.push(p_dll);
                            dst.push(base.clone());
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            dst.push(p_dylib);
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            dst.push(p_so);
                            dst.push(base.clone());
                        }
                        #[cfg(all(unix, not(target_os = "macos")))]
                        {
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            dst.push(p_so);
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            dst.push(p_dylib);
                            dst.push(base.clone());
                        }
                    };
                push_platform_candidates(&next_to_manifest, &mut exe_candidates);
                let crate_root = manifest_dir
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(manifest_dir.clone());
                let cand_debug = crate_root.join("target").join("debug").join(&entry);
                push_platform_candidates(&cand_debug, &mut exe_candidates);
                let cand_rel = crate_root.join("target").join("release").join(&entry);
                push_platform_candidates(&cand_rel, &mut exe_candidates);
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
                    let exe_abs = std::fs::canonicalize(&exe).unwrap_or(exe.clone());
                    if desc.manifest.prefers_inprocess() {
                        match mainstage_core::vm::inprocess::InProcessPlugin::new(exe_abs.as_path())
                        {
                            Ok(plugin) => {
                                run_vm.register_plugin(std::sync::Arc::new(plugin));
                            }
                            Err(e) => {
                                warn!("failed to load in-process plugin '{}': {}", mod_name, e);
                                let ep = mainstage_core::vm::external::ExternalPlugin::new(
                                    alias.to_string(),
                                    exe_abs,
                                );
                                run_vm.register_plugin(std::sync::Arc::new(ep));
                            }
                        }
                    } else {
                        let ep = mainstage_core::vm::external::ExternalPlugin::new(
                            alias.to_string(),
                            exe_abs,
                        );
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
            warn!("no plugin descriptor found for module '{}'", mod_name);
        }
    };

    for name in stl_selection.iter() {
        register_by_name(name, name);
    }

    let mut import_aliases: Vec<(String, String)> = Vec::new();
    let src_text = script.display_content().to_string();
    for line in src_text.lines() {
        let s = line.trim();
        if s.starts_with("import ")
            && let Some(rest) = s.strip_prefix("import ")
        {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 && parts[1] == "as" {
                let module = parts[0].trim().trim_matches('"').to_string();
                let alias = parts[2].trim().trim_end_matches(';').to_string();
                import_aliases.push((alias, module));
            }
        }
    }

    for (alias, mod_name) in import_aliases.into_iter() {
        if let Some(desc) = manifests.get(&mod_name) {
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
                    let mp_resolved = if mp.is_absolute() {
                        mp
                    } else {
                        manifest_dir.join(mp)
                    };
                    if mp_resolved.is_dir() {
                        let base = mp_resolved.join(&entry);
                        #[cfg(target_os = "windows")]
                        {
                            let mut p_exe = base.clone();
                            p_exe.set_extension("exe");
                            exe_candidates.push(p_exe);
                            let mut p_dll = base.clone();
                            p_dll.set_extension("dll");
                            exe_candidates.push(p_dll);
                            exe_candidates.push(base);
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            exe_candidates.push(p_dylib);
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            exe_candidates.push(p_so);
                            exe_candidates.push(base);
                        }
                        #[cfg(all(unix, not(target_os = "macos")))]
                        {
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            exe_candidates.push(p_so);
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            exe_candidates.push(p_dylib);
                            exe_candidates.push(base);
                        }
                    } else {
                        exe_candidates.push(mp_resolved.clone());
                    }
                }
                let next_to_manifest = manifest_dir.join(&entry);
                let push_platform_candidates =
                    |base: &std::path::PathBuf, dst: &mut Vec<std::path::PathBuf>| {
                        #[cfg(target_os = "windows")]
                        {
                            let mut p_exe = base.clone();
                            p_exe.set_extension("exe");
                            dst.push(p_exe);
                            let mut p_dll = base.clone();
                            p_dll.set_extension("dll");
                            dst.push(p_dll);
                            dst.push(base.clone());
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            dst.push(p_dylib);
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            dst.push(p_so);
                            dst.push(base.clone());
                        }
                        #[cfg(all(unix, not(target_os = "macos")))]
                        {
                            let mut p_so = base.clone();
                            p_so.set_extension("so");
                            dst.push(p_so);
                            let mut p_dylib = base.clone();
                            p_dylib.set_extension("dylib");
                            dst.push(p_dylib);
                            dst.push(base.clone());
                        }
                    };
                push_platform_candidates(&next_to_manifest, &mut exe_candidates);
                let crate_root = manifest_dir
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(manifest_dir.clone());
                let cand_debug = crate_root.join("target").join("debug").join(&entry);
                push_platform_candidates(&cand_debug, &mut exe_candidates);
                let cand_rel = crate_root.join("target").join("release").join(&entry);
                push_platform_candidates(&cand_rel, &mut exe_candidates);
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
                    let exe_abs = std::fs::canonicalize(&exe).unwrap_or(exe.clone());
                    if desc.manifest.prefers_inprocess() {
                        match mainstage_core::vm::inprocess::InProcessPlugin::new(exe_abs.as_path())
                        {
                            Ok(plugin) => {
                                run_vm.register_plugin(std::sync::Arc::new(plugin));
                            }
                            Err(e) => {
                                warn!("failed to load in-process plugin '{}': {}", mod_name, e);
                                let ep = mainstage_core::vm::external::ExternalPlugin::new(
                                    alias.clone(),
                                    exe_abs,
                                );
                                run_vm.register_plugin(std::sync::Arc::new(ep));
                            }
                        }
                    } else {
                        let ep = mainstage_core::vm::external::ExternalPlugin::new(
                            alias.clone(),
                            exe_abs,
                        );
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

    if let Some(parent) = script.path.parent()
        && let Err(e) = std::env::set_current_dir(parent)
    {
        warn!("failed to set working dir to {:?}: {}", parent, e);
    }

    match run_vm.run(trace) {
        Ok(()) => {}
        Err(e) => {
            error!("{}", e.lines().collect::<Vec<&str>>().join("\n\t"));
        }
    }

    if let Some(orig) = orig_cwd {
        let _ = std::env::set_current_dir(orig);
    }
}
