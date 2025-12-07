use std::io::{self, Read};
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let func = args.next().unwrap_or_default();

    if cmd != "call" {
        eprintln!("unsupported command");
        std::process::exit(1);
    }

    // Read JSON request from stdin (we'll accept either full object or just args array)
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).unwrap_or(0);
    let json: serde_json::Value = if buf.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&buf).unwrap_or(serde_json::json!({}))
    };

    // Provide a `list_compilers` function to enumerate available compilers.
    if func == "list_compilers" {
        let found = find_available_compilers();
        let mut out: Vec<serde_json::Value> = Vec::new();
        for (name, path) in found.into_iter() {
            let version = get_compiler_version(path.clone()).unwrap_or_default();
            out.push(serde_json::json!({ "name": name, "path": path.to_string_lossy(), "version": version }));
        }
        println!("{}", serde_json::to_string(&out).unwrap_or("[]".to_string()));
        return;
    }

    // For `compile`, accept either an args-array or args-object.
    if func == "compile" {
        // defaults
        let mut sources: Vec<String> = Vec::new();
        let mut flags: Vec<String> = Vec::new();
        let mut compiler: Option<String> = None;

        match &json["args"] {
            serde_json::Value::Array(a) => {
                // args array: [sources, flags?, compiler?]
                if let Some(sv) = a.first() && let serde_json::Value::Array(sa) = sv {
                    sources = sa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
                }
                if let Some(fv) = a.get(1) && let serde_json::Value::Array(fa) = fv {
                    flags = fa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
                }
                if let Some(cv) = a.get(2) && let Some(s) = cv.as_str() { compiler = Some(s.to_string()); }
            }
            serde_json::Value::Object(map) => {
                if let Some(sv) = map.get("sources") && let serde_json::Value::Array(sa) = sv { sources = sa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); }
                if let Some(fv) = map.get("flags") && let serde_json::Value::Array(fa) = fv { flags = fa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); }
                if let Some(cv) = map.get("compiler") && let Some(s) = cv.as_str() { compiler = Some(s.to_string()); }
            }
            _ => {}
        }

        let result = compile_cpp_sources_with(sources, flags, compiler);
        let output = match result {
            Ok(path) => serde_json::json!({"ok": true, "path": path}),
            Err(err) => serde_json::json!({"ok": false, "error": err}),
        };
        let s = serde_json::to_string(&output).unwrap_or("null".to_string());
        println!("{}", s);
        return;
    }

    eprintln!("unknown function");
    std::process::exit(1);
}

/// Compiles the given C++ source files and returns the path to the compiled binary or an error message.
/// # Arguments
/// * `sources` - A vector of strings representing the paths to C++ source files.
/// # Returns
/// * `Ok(String)` - The path to the compiled binary if compilation is successful.
/// * `Err(String)` - An error message if compilation fails.
fn compile_cpp_sources_with(sources: Vec<String>, flags: Vec<String>, compiler_hint: Option<String>) -> Result<String, String> {
    // basic validation
    if sources.is_empty() {
        return Err("No source files provided".to_string());
    }

    // Select a compiler (hint preferred)
    let (compiler_name, compiler_path) = match select_compiler(compiler_hint) {
        Some(p) => p,
        None => return Err("No supported C++ compiler found on the system".to_string()),
    };

    // Choose output name
    let out_name = if cfg!(target_os = "windows") { "output_binary.exe" } else { "output_binary" };

    // Build command
    let mut cmd = build_compile_command(&compiler_name, compiler_path.clone(), &sources, &flags, out_name);

    // If MSVC, try to populate env via vcvars before running
    if cfg!(target_os = "windows") && (compiler_name == "cl" || compiler_name.to_lowercase().contains("cl")) && let Some(envs) = common::ensure_msvc_env(compiler_path.as_path()) {
        cmd.envs(envs);
    }

    // Execute
    let output = cmd.output().map_err(|e| format!("Failed to execute compiler '{}': {}", compiler_name, e))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Compilation failed: stdout:\n{}\nstderr:\n{}", stdout, stderr));
    }

    Ok(out_name.to_string())
}

// --- Helper functions -------------------------------------------------

fn candidate_compilers() -> Vec<&'static str> {
    #[cfg(target_os = "windows")]
    return vec!["cl", "g++", "clang++"];
    #[cfg(not(target_os = "windows"))]
    return vec!["g++", "clang++", "clang", "gcc"];
}

fn find_available_compilers() -> Vec<(String, PathBuf)> {
    common::find_available_compilers_from(&candidate_compilers())
}

fn select_compiler(hint: Option<String>) -> Option<(String, PathBuf)> {
    if let Some(h) = hint && let Ok(p) = which::which(&h) {
        return Some((h, p));
    }
    find_available_compilers().into_iter().next()
}

fn get_compiler_version(path: PathBuf) -> Option<String> {
    common::get_compiler_version(path.as_path())
}

fn build_compile_command(name: &str, path: PathBuf, sources: &[String], flags: &[String], out_name: &str) -> Command {
    common::build_compile_command(name, path.as_path(), sources, flags, out_name)
}