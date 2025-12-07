use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod typed;

pub use typed::*;

/// Ensure MSVC environment for a given `cl.exe` path by attempting multiple discovery strategies.
/// Returns an environment map suitable for passing to `Command::envs` if successful.
pub fn ensure_msvc_env(cl_path: &Path) -> Option<HashMap<String, String>> {
    #[cfg(not(target_os = "windows"))]
    {
        return None;
    }

    #[cfg(target_os = "windows")]
    {
        // Candidates to search for vcvarsall.bat
        let mut candidates: Vec<PathBuf> = Vec::new();

        // Check common environment variables
        if let Some(vs) = std::env::var_os("VSINSTALLDIR") {
            let vs_clone = vs.clone();
            candidates.push(PathBuf::from(vs_clone).join("VC/Auxiliary/Build/vcvarsall.bat"));
            candidates.push(PathBuf::from(vs).join("VC\\Auxiliary\\Build\\vcvarsall.bat"));
        }
        if let Some(vc) = std::env::var_os("VCINSTALLDIR") {
            candidates.push(PathBuf::from(vc).join("vcvarsall.bat"));
        }

        // Walk up from cl.exe looking for vcvarsall.bat
        if let Some(mut anc) = cl_path.parent() {
            for _ in 0..6 {
                candidates.push(anc.join("VC/Auxiliary/Build/vcvarsall.bat"));
                candidates.push(anc.join("VC\\Auxiliary\\Build\\vcvarsall.bat"));
                if let Some(p) = anc.parent() { anc = p; } else { break; }
            }
        }

        // Try each candidate; if we can capture env, return it
        for c in candidates.into_iter() {
            if c.exists() && let Some(envs) = capture_env_from_vcvars(c) {
                    return Some(envs);
            }
        }

        // Fallback: try vswhere-based discovery
        if let Some(envs) = find_vcvars_via_vswhere() {
            return Some(envs);
        }

        None
    }
}

/// Probe a compiler binary for a short version string.
pub fn get_compiler_version(path: &Path) -> Option<String> {
    let probes = vec!["--version", "-v", "-V", "/?", "-help"];
    for flag in probes {
        let out = Command::new(path).arg(flag).output();
        if let Ok(o) = out {
            let combined = format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            let first = combined.lines().next().map(|l| l.to_string());
            if first.is_some() { return first; }
        }
    }
    None
}

/// Build a compile `Command` for the given compiler. Uses MSVC-style `/Fe:` when the name indicates `cl`.
pub fn build_compile_command(name: &str, path: &Path, sources: &[String], flags: &[String], out_name: &str) -> Command {
    let mut cmd = Command::new(path);
    if name == "cl" || name.to_lowercase().contains("cl") {
        for src in sources { cmd.arg(src); }
        for f in flags { cmd.arg(f); }
        cmd.arg(format!("/Fe:{}", out_name));
    } else {
        for src in sources { cmd.arg(src); }
        for f in flags { cmd.arg(f); }
        cmd.arg("-o");
        cmd.arg(out_name);
    }
    cmd
}

/// Find available compilers from a list of candidate names (uses `which`).
pub fn find_available_compilers_from(candidates: &[&str]) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for c in candidates {
        if let Ok(p) = which::which(c) {
            found.push((c.to_string(), p));
        }
    }
    found
}

#[cfg(target_os = "windows")]
fn find_vcvars_via_vswhere() -> Option<HashMap<String, String>> {
    // Try running vswhere (may be on PATH) to get installation paths
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Preferred invocation: ask for latest installation path
    let vswhere_args = ["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"];
    let try_vswhere = |exe: &str| -> Option<String> {
        let out = Command::new(exe).args(vswhere_args).output();
        if let Ok(o) = out && (o.status.success() || !o.stdout.is_empty()) {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            let s = s.trim();
            if !s.is_empty() { return Some(s.to_string()); }
        }
        None
    };

    // First try plain `vswhere` on PATH
    if let Some(p) = try_vswhere("vswhere") {
        candidates.push(PathBuf::from(p));
    }

    // Common hardcoded location for vswhere on Windows
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from);
    if let Some(mut pf) = program_files_x86 {
        pf.push("Microsoft Visual Studio/Installer/vswhere.exe");
        if pf.exists() && let Some(p) = try_vswhere(&pf.to_string_lossy()) {
                candidates.push(PathBuf::from(p));
        }
    }

    // If vswhere produced installation paths, try vcvars in those installations
    for inst in candidates.into_iter() {
        let vcvars = inst.join("VC/Auxiliary/Build/vcvarsall.bat");
        if vcvars.exists() && let Some(envs) = capture_env_from_vcvars(vcvars) { 
            return Some(envs);
        }
        let vcvars2 = inst.join("VC\\Auxiliary\\Build\\vcvarsall.bat");
        if vcvars2.exists() && let Some(envs) = capture_env_from_vcvars(vcvars2) { 
            return Some(envs); 
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn capture_env_from_vcvars(vcvars: PathBuf) -> Option<HashMap<String, String>> {
    // Try a few common architectures
    let archs = ["x64", "x86_amd64", "x86"];
    for arch in archs.iter() {
        // cmd /C "call "<vcvars>" <arch> && set"
        let cmdline = format!("call \"{}\" {} && set", vcvars.display(), arch);
        let output = Command::new("cmd").args(["/C", &cmdline]).output();
        if let Ok(o) = output && (o.status.success() || !o.stdout.is_empty()) {
            let out = String::from_utf8_lossy(&o.stdout).to_string();
            let mut map = HashMap::new();
            for line in out.lines() {
                if let Some(idx) = line.find('=') {
                    let k = &line[..idx];
                    let v = &line[idx+1..];
                    map.insert(k.to_string(), v.to_string());
                }
            }
            if !map.is_empty() { return Some(map); }
        }
    }
    None
}
