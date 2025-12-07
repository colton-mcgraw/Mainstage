use clap::ArgMatches;
use log::{error, info, warn};

pub fn handle(
    sub_m: &ArgMatches,
    manifests: &std::collections::HashMap<String, mainstage_core::vm::plugin::PluginDescriptor>,
) {
    let module = sub_m.get_one::<String>("module").map(|s| s.to_string());
    let mut results: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut mismatched = 0usize;

    let load_runtime = |mod_name: &str| -> Option<Vec<String>> {
        if let Some(desc) = manifests.get(mod_name) {
            if let Some(dir) = &desc.path {
                let entry = desc.manifest.entry.clone().unwrap_or_else(|| desc.manifest.name.clone());
                let manifest_dir = dir.clone();
                let base = manifest_dir.join(&entry);
                let mut candidates: Vec<std::path::PathBuf> = Vec::new();
                #[cfg(target_os = "windows")] {
                    let mut p_dll = base.clone(); p_dll.set_extension("dll"); candidates.push(p_dll);
                    candidates.push(base.clone());
                }
                #[cfg(target_os = "macos")] {
                    let mut p_dylib = base.clone(); p_dylib.set_extension("dylib"); candidates.push(p_dylib);
                    let mut pref = base.clone(); pref.set_file_name(format!("lib{}", entry)); pref.set_extension("dylib"); candidates.push(pref);
                    candidates.push(base.clone());
                }
                #[cfg(all(unix, not(target_os = "macos")))] {
                    let mut p_so = base.clone(); p_so.set_extension("so"); candidates.push(p_so);
                    let mut pref = base.clone(); pref.set_file_name(format!("lib{}", entry)); pref.set_extension("so"); candidates.push(pref);
                    candidates.push(base.clone());
                }
                for c in candidates.into_iter() {
                    if c.exists() && std::fs::metadata(&c).map(|m| m.is_file()).unwrap_or(false) {
                        match mainstage_core::vm::inprocess::InProcessPlugin::new(c.as_path()) {
                            Ok(ip) => { return Some(ip.list_registered_functions()); }
                            Err(e) => { warn!("in-process load failed for '{}': {}", mod_name, e); continue; }
                        }
                    }
                }
            }
        }
        None
    };

    let names_to_check: Vec<String> = if let Some(m) = module { vec![m] } else { manifests.keys().cloned().collect() };
    for mod_name in names_to_check.iter() {
        checked += 1;
        let manifest_funcs: Vec<String> = manifests.get(mod_name)
            .map(|d| d.manifest.functions.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default();
        let runtime_funcs = load_runtime(mod_name);
        match runtime_funcs {
            Some(rf) => {
                let ms: std::collections::HashSet<String> = manifest_funcs.iter().cloned().collect();
                let rs: std::collections::HashSet<String> = rf.iter().cloned().collect();
                let missing: Vec<String> = ms.difference(&rs).cloned().collect();
                let extra: Vec<String> = rs.difference(&ms).cloned().collect();
                if missing.is_empty() && extra.is_empty() {
                    results.push(format!("{}: OK ({} functions)", mod_name, rf.len()));
                } else {
                    mismatched += 1;
                    if !missing.is_empty() { results.push(format!("{}: manifest entries missing in plugin: {}", mod_name, missing.join(", "))); }
                    if !extra.is_empty() { results.push(format!("{}: plugin has extra functions not in manifest: {}", mod_name, extra.join(", "))); }
                }
            }
            None => {
                results.push(format!("{}: cannot verify (not in-process or library not found)", mod_name));
            }
        }
    }

    for line in results.iter() { info!("{}", line); }
    if mismatched > 0 { error!("{} of {} plugin(s) mismatched", mismatched, checked); } else { info!("Verified {} plugin(s)", checked); }
}
