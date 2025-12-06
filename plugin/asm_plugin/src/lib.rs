use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// In-process adapter for asm_plugin using mainstage_register + JSON handlers.

fn list_compilers_json() -> String {
    let found = common::find_available_compilers_from(&["nasm", "yasm", "gcc", "clang"]);
    let mut out: Vec<serde_json::Value> = Vec::new();
    for (name, path) in found.into_iter() {
        let version = common::get_compiler_version(path.as_path()).unwrap_or_default();
        out.push(serde_json::json!({ "name": name, "path": path.to_string_lossy(), "version": version }));
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".to_string())
}

fn assemble_json(args_json: &serde_json::Value) -> String {
    let mut sources: Vec<String> = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    let mut compiler: Option<String> = None;

    match args_json {
        serde_json::Value::Array(a) => {
            if let Some(sv) = a.get(0) { if let serde_json::Value::Array(sa) = sv { sources = sa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); } }
            if let Some(fv) = a.get(1) { if let serde_json::Value::Array(fa) = fv { flags = fa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); } }
            if let Some(cv) = a.get(2) { if let Some(s) = cv.as_str() { compiler = Some(s.to_string()); } }
        }
        serde_json::Value::Object(map) => {
            if let Some(sv) = map.get("sources") { if let serde_json::Value::Array(sa) = sv { sources = sa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); } }
            if let Some(fv) = map.get("flags") { if let serde_json::Value::Array(fa) = fv { flags = fa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); } }
            if let Some(cv) = map.get("compiler") { if let Some(s) = cv.as_str() { compiler = Some(s.to_string()); } }
        }
        _ => {}
    }

    // Inline assemble implementation using helpers from `common`.
    fn candidate_compilers() -> Vec<&'static str> {
        #[cfg(target_os = "windows")] return vec!["ml64", "ml", "nasm", "yasm", "cl", "gcc", "clang"];
        #[cfg(not(target_os = "windows"))] return vec!["nasm", "yasm", "gcc", "clang"];
    }

    fn find_available_compilers() -> Vec<(String, std::path::PathBuf)> {
        common::find_available_compilers_from(&candidate_compilers())
    }

    fn select_compiler(hint: Option<&str>) -> Option<(String, std::path::PathBuf)> {
        if let Some(h) = hint {
            if let Ok(p) = which::which(h) {
                return Some((h.to_string(), p));
            }
        }
        find_available_compilers().into_iter().next()
    }

    fn assemble_sources_with(sources: &[String], flags: &[String], compiler_hint: Option<&str>) -> Result<String, String> {
        if sources.is_empty() {
            return Err("No source files provided".to_string());
        }

        let (compiler_name, compiler_path) = match select_compiler(compiler_hint) {
            Some(p) => p,
            None => return Err("No supported assembler/compiler found on the system".to_string()),
        };

        let out_name = if cfg!(target_os = "windows") { "output_binary.exe" } else { "output_binary" };

        let mut cmd = common::build_compile_command(&compiler_name, &compiler_path, sources, flags, out_name);

        if cfg!(target_os = "windows") {
            if let Some(envs) = common::ensure_msvc_env(compiler_path.as_path()) {
                cmd.envs(envs.into_iter());
            }
        }

        let output = cmd.output().map_err(|e| format!("Failed to execute assembler/compiler '{}': {}", compiler_name, e))?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Assembly failed: stdout:\n{}\nstderr:\n{}", stdout, stderr));
        }

        Ok(out_name.to_string())
    }

    let result = assemble_sources_with(&sources, &flags, compiler.as_deref());
    let output = match result {
        Ok(path) => serde_json::json!({"ok": true, "path": path}),
        Err(err) => serde_json::json!({"ok": false, "error": err}),
    };
    serde_json::to_string(&output).unwrap_or("null".to_string())
}

// C ABI types mirrored from core
type CJsonHandler = unsafe extern "C" fn(input_json: *const c_char) -> *mut c_char;
type CRegistrar = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CJsonHandler);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_register(ctx: *mut std::ffi::c_void, registrar: CRegistrar) {
    unsafe extern "C" fn list_compilers_handler(_input_json: *const c_char) -> *mut c_char {
        let out = list_compilers_json();
        let c = CString::new(out).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn assemble_handler(input_json: *const c_char) -> *mut c_char {
        let json = if input_json.is_null() {
            serde_json::Value::Null
        } else {
            let s = CStr::from_ptr(input_json).to_string_lossy().into_owned();
            serde_json::from_str::<serde_json::Value>(&s).unwrap_or(serde_json::Value::Null)
        };
        let out = assemble_json(&json);
        let c = CString::new(out).unwrap();
        dup_cstr(&c)
    }
    let n1 = CString::new("list_compilers").unwrap();
    registrar(ctx, n1.as_ptr(), list_compilers_handler);
    let n2 = CString::new("compile").unwrap();
    registrar(ctx, n2.as_ptr(), assemble_handler);
    let n3 = CString::new("assemble").unwrap();
    registrar(ctx, n3.as_ptr(), assemble_handler);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_free(ptr: *mut c_char){
    if !ptr.is_null() {
        unsafe { libc::free(ptr as *mut libc::c_void); }
    }
}

fn dup_cstr(s: &CString) -> *mut c_char {
    unsafe {
        let bytes = s.as_bytes_with_nul();
        let ptr = libc::malloc(bytes.len()) as *mut u8;
        if ptr.is_null() { return std::ptr::null_mut(); }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        ptr as *mut c_char
    }
}
