use clap::ArgMatches;
use log::{error, info, warn};

pub fn handle(
    sub_m: &ArgMatches,
    manifests: &std::collections::HashMap<String, mainstage_core::vm::plugin::PluginDescriptor>,
) {
    // Options: --json (bool), --strict (bool)
    let emit_json = sub_m.get_flag("json");
    let strict = sub_m.get_flag("strict");
    let module = sub_m.get_one::<String>("module").map(|s| s.to_string());
    let mut results: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut mismatched = 0usize;

    let load_runtime = |mod_name: &str| -> Option<Vec<String>> {
        if let Some(desc) = manifests.get(mod_name) {
            info!("Verifying plugin module '{}'", mod_name);
            info!(
                "Plugin contains {} functions in manifest",
                desc.manifest.functions.len()
            );
            if let Some(dir) = &desc.path {
                let entry = desc
                    .manifest
                    .entry
                    .clone()
                    .unwrap_or_else(|| desc.manifest.name.clone());
                let manifest_dir = dir.clone();
                let base = manifest_dir.join(&entry);
                let mut candidates: Vec<std::path::PathBuf> = Vec::new();
                // Prefer manifest.path when provided (may point to build output dir or file)
                if !desc.manifest.path.trim().is_empty() {
                    let mp = std::path::PathBuf::from(desc.manifest.path.clone());
                    let mp_resolved = if mp.is_absolute() { mp } else { manifest_dir.join(mp) };
                    if mp_resolved.is_dir() {
                        let mf_base = mp_resolved.join(&entry);
                        #[cfg(target_os = "windows")] {
                            let mut p_exe = mf_base.clone(); p_exe.set_extension("exe"); candidates.push(p_exe);
                            let mut p_dll = mf_base.clone(); p_dll.set_extension("dll"); candidates.push(p_dll);
                            candidates.push(mf_base.clone());
                        }
                        #[cfg(target_os = "macos")] {
                            let mut p_dylib = mf_base.clone(); p_dylib.set_extension("dylib"); candidates.push(p_dylib);
                            let mut p_so = mf_base.clone(); p_so.set_extension("so"); candidates.push(p_so);
                            candidates.push(mf_base.clone());
                        }
                        #[cfg(all(unix, not(target_os = "macos")))] {
                            let mut p_so = mf_base.clone(); p_so.set_extension("so"); candidates.push(p_so);
                            let mut p_dylib = mf_base.clone(); p_dylib.set_extension("dylib"); candidates.push(p_dylib);
                            candidates.push(mf_base.clone());
                        }
                    } else {
                        candidates.push(mp_resolved.clone());
                    }
                }
                #[cfg(target_os = "windows")]
                {
                    let mut p_dll = base.clone();
                    p_dll.set_extension("dll");
                    candidates.push(p_dll);
                    candidates.push(base.clone());
                }
                #[cfg(target_os = "macos")]
                {
                    let mut p_dylib = base.clone();
                    p_dylib.set_extension("dylib");
                    candidates.push(p_dylib);
                    let mut pref = base.clone();
                    pref.set_file_name(format!("lib{}", entry));
                    pref.set_extension("dylib");
                    candidates.push(pref);
                    candidates.push(base.clone());
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    let mut p_so = base.clone();
                    p_so.set_extension("so");
                    candidates.push(p_so);
                    let mut pref = base.clone();
                    pref.set_file_name(format!("lib{}", entry));
                    pref.set_extension("so");
                    candidates.push(pref);
                    candidates.push(base.clone());
                }
                // Also try typical cargo target locations
                let crate_root = manifest_dir
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(manifest_dir.clone());
                let cand_debug = crate_root.join("target").join("debug").join(&entry);
                #[cfg(target_os = "windows")]
                {
                    let mut p_exe = cand_debug.clone(); p_exe.set_extension("exe"); candidates.push(p_exe);
                    let mut p_dll = cand_debug.clone(); p_dll.set_extension("dll"); candidates.push(p_dll);
                    candidates.push(cand_debug.clone());
                }
                #[cfg(target_os = "macos")]
                {
                    let mut p_dylib = cand_debug.clone(); p_dylib.set_extension("dylib"); candidates.push(p_dylib);
                    let mut p_so = cand_debug.clone(); p_so.set_extension("so"); candidates.push(p_so);
                    candidates.push(cand_debug.clone());
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    let mut p_so = cand_debug.clone(); p_so.set_extension("so"); candidates.push(p_so);
                    let mut p_dylib = cand_debug.clone(); p_dylib.set_extension("dylib"); candidates.push(p_dylib);
                    candidates.push(cand_debug.clone());
                }
                let cand_rel = crate_root.join("target").join("release").join(&entry);
                #[cfg(target_os = "windows")]
                {
                    let mut p_exe = cand_rel.clone(); p_exe.set_extension("exe"); candidates.push(p_exe);
                    let mut p_dll = cand_rel.clone(); p_dll.set_extension("dll"); candidates.push(p_dll);
                    candidates.push(cand_rel.clone());
                }
                #[cfg(target_os = "macos")]
                {
                    let mut p_dylib = cand_rel.clone(); p_dylib.set_extension("dylib"); candidates.push(p_dylib);
                    let mut p_so = cand_rel.clone(); p_so.set_extension("so"); candidates.push(p_so);
                    candidates.push(cand_rel.clone());
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    let mut p_so = cand_rel.clone(); p_so.set_extension("so"); candidates.push(p_so);
                    let mut p_dylib = cand_rel.clone(); p_dylib.set_extension("dylib"); candidates.push(p_dylib);
                    candidates.push(cand_rel.clone());
                }

                for c in candidates.into_iter() {
                    if c.exists() && std::fs::metadata(&c).map(|m| m.is_file()).unwrap_or(false) {
                        match mainstage_core::vm::inprocess::InProcessPlugin::new(c.as_path()) {
                            Ok(ip) => {
                                return Some(ip.list_registered_functions());
                            }
                            Err(e) => {
                                warn!("in-process load failed for '{}': {}", mod_name, e);
                                continue;
                            }
                        }
                    }
                }
            }
        }
        None
    };

    let names_to_check: Vec<String> = if let Some(m) = module {
        vec![m]
    } else {
        manifests.keys().cloned().collect()
    };

    for mod_name in names_to_check.iter() {
        checked += 1;
        // Use fully-qualified names (domain.name) to match runtime
        let manifest_funcs: Vec<String> = manifests
            .get(mod_name)
            .map(|d| {
                d.manifest
                    .functions
                    .iter()
                    .map(|f| {
                        // Prefer explicit domain if present; fall back to name
                        // Runtime uses qualified names like "fs.read" or "env.set"
                        match &f.domain {
                            Some(dom) if !dom.trim().is_empty() => format!("{}.{}", dom, f.name),
                            _ => f.name.clone(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let runtime_funcs = load_runtime(mod_name);
        match runtime_funcs {
            Some(rf) => {
                // Build runtime set for quick lookup
                let runtime_set: std::collections::HashSet<String> = rf.iter().cloned().collect();

                // Helper to derive unqualified name (after last '.')
                let unqual = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();

                // Determine missing: a manifest entry is missing only if neither its
                // qualified form nor its unqualified form appears in the runtime set.
                let mut missing: Vec<String> = Vec::new();
                for m in manifest_funcs.iter() {
                    let mu = unqual(m);
                    if !runtime_set.contains(m) && !runtime_set.contains(&mu) {
                        missing.push(m.clone());
                    }
                }

                // Determine extras: a runtime name is extra only if neither its
                // qualified form nor its unqualified form matches any manifest entry.
                let manifest_set: std::collections::HashSet<String> =
                    manifest_funcs.iter().cloned().collect();
                let manifest_unqual_set: std::collections::HashSet<String> = manifest_funcs
                    .iter()
                    .map(|m| unqual(m))
                    .collect();
                let mut extra: Vec<String> = Vec::new();
                for r in rf.iter() {
                    let ru = unqual(r);
                    if !manifest_set.contains(r) && !manifest_unqual_set.contains(&ru) {
                        extra.push(r.clone());
                    }
                }

                if missing.is_empty() && extra.is_empty() {
                    results.push(format!("{}: OK ({} functions)", mod_name, rf.len()));
                } else {
                    mismatched += 1;
                    if !missing.is_empty() {
                        results.push(format!(
                            "{}: manifest entries missing in plugin: {}",
                            mod_name,
                            missing.join(", ")
                        ));
                    }
                    if !extra.is_empty() {
                        results.push(format!(
                            "{}: plugin has extra functions not in manifest: {}",
                            mod_name,
                            extra.join(", ")
                        ));
                    }
                }
            }
            None => {
                results.push(format!(
                    "{}: cannot verify (not in-process or library not found)",
                    mod_name
                ));
            }
        }
    }

    if emit_json {
        // Emit machine-readable summary
        // Structure: { checked: N, mismatched: M, results: [ "module: msg", ... ] }
        let obj = serde_json::json!({
            "checked": checked,
            "mismatched": mismatched,
            "results": results,
        });
        println!("{}", obj);
    } else {
        for line in results.iter() {
            info!("{}", line);
        }
        if mismatched > 0 {
            error!("{} of {} plugin(s) mismatched", mismatched, checked);
        } else {
            info!("Verified {} plugin(s)", checked);
        }
    }

    if strict && mismatched > 0 {
        // Non-zero exit on mismatch when --strict provided
        std::process::exit(2);
    }
}
