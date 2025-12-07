use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use common::typed::{CTag, CValue, CStrView, CObjectEntry, CTypedRegistrar, CTypedHandler, cvalue_object, cvalue_bool, cvalue_string};

// In-process adapter for cpp_plugin using mainstage_register + JSON handlers.

fn list_compilers_json() -> String {
    let found = common::find_available_compilers_from(&["g++", "clang++", "clang", "gcc", "cl"]); // reasonable defaults
    let mut out: Vec<serde_json::Value> = Vec::new();
    for (name, path) in found.into_iter() {
        let version = common::get_compiler_version(path.as_path()).unwrap_or_default();
        out.push(serde_json::json!({ "name": name, "path": path.to_string_lossy(), "version": version }));
    }
    serde_json::to_string(&out).unwrap_or("[]".to_string())
}

fn compile_json(args_json: &serde_json::Value) -> String {
    // Parse args: accept args array or object like the CLI.
    let mut sources: Vec<String> = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    let mut compiler: Option<String> = None;

    match args_json {
        serde_json::Value::Array(a) => {
            if let Some(sv) = a.first() && let serde_json::Value::Array(sa) = sv { sources = sa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); }
            if let Some(fv) = a.get(1) && let serde_json::Value::Array(fa) = fv { flags = fa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); }
            if let Some(cv) = a.get(2) && let Some(s) = cv.as_str() { compiler = Some(s.to_string()); }
        }
        serde_json::Value::Object(map) => {
            if let Some(sv) = map.get("sources") && let serde_json::Value::Array(sa) = sv { sources = sa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); }
            if let Some(fv) = map.get("flags") && let serde_json::Value::Array(fa) = fv { flags = fa.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(); }
            if let Some(cv) = map.get("compiler") && let Some(s) = cv.as_str() { compiler = Some(s.to_string()); }
        }
        _ => {}
    }

    // Inline compile implementation using helpers from `common` to avoid
    // depending on a single helper that may not exist.
    fn candidate_compilers() -> Vec<&'static str> {
        #[cfg(target_os = "windows")] return vec!["cl", "g++", "clang++"];
        #[cfg(not(target_os = "windows"))] return vec!["g++", "clang++", "clang", "gcc"];
    }

    fn find_available_compilers() -> Vec<(String, std::path::PathBuf)> {
        common::find_available_compilers_from(&candidate_compilers())
    }

    fn select_compiler(hint: Option<&str>) -> Option<(String, std::path::PathBuf)> {
        if let Some(h) = hint && let Ok(p) = which::which(h) {
            return Some((h.to_string(), p));
        }
        find_available_compilers().into_iter().next()
    }

    fn compile_sources_with(sources: &[String], flags: &[String], compiler_hint: Option<&str>) -> Result<String, String> {
        if sources.is_empty() {
            return Err("No source files provided".to_string());
        }

        let (compiler_name, compiler_path) = match select_compiler(compiler_hint) {
            Some(p) => p,
            None => return Err("No supported C++ compiler found on the system".to_string()),
        };

        let out_name = if cfg!(target_os = "windows") { "output_binary.exe" } else { "output_binary" };

        let mut cmd = common::build_compile_command(&compiler_name, &compiler_path, sources, flags, out_name);

        if cfg!(target_os = "windows") && (compiler_name == "cl" || compiler_name.to_lowercase().contains("cl")) && let Some(envs) = common::ensure_msvc_env(compiler_path.as_path()) {
            cmd.envs(envs);
        }

        let output = cmd.output().map_err(|e| format!("Failed to execute compiler '{}': {}", compiler_name, e))?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Compilation failed: stdout:\n{}\nstderr:\n{}", stdout, stderr));
        }

        Ok(out_name.to_string())
    }

    // Prepare real file paths for the compiler. If callers passed file
    // contents (e.g. via `read()` returning contents), write them to
    // temporary files and pass those paths to the compiler. If an entry is
    // already a filesystem path that exists, use it directly.
    let mut temp_files: Vec<std::path::PathBuf> = Vec::new();
    let mut source_paths: Vec<String> = Vec::new();
    let mut anon_idx: usize = 0;
    for s in sources.iter() {
        let p = std::path::Path::new(s);
        if p.exists() {
            source_paths.push(p.to_string_lossy().to_string());
        } else {
            // Treat `s` as file contents; write to a temp file.
            let mut tmp = std::env::temp_dir();
            anon_idx += 1;
            let fname = format!("mainstage_tmp_{}_{}.cpp", std::process::id(), anon_idx);
            tmp.push(fname);
            if let Err(e) = std::fs::write(&tmp, s.as_bytes()) {
                // cleanup any previously created temp files
                for t in temp_files.iter() { let _ = std::fs::remove_file(t); }
                let output = serde_json::json!({"ok": false, "error": format!("failed to write temp source file: {}", e)});
                return serde_json::to_string(&output).unwrap_or("null".to_string());
            }
            source_paths.push(tmp.to_string_lossy().to_string());
            temp_files.push(tmp);
        }
    }

    let result = compile_sources_with(&source_paths, &flags, compiler.as_deref());
    // Remove temporary source files we created (best-effort)
    for t in temp_files.iter() { let _ = std::fs::remove_file(t); }
    let output = match result {
        Ok(path) => serde_json::json!({"ok": true, "path": path}),
        Err(err) => serde_json::json!({"ok": false, "error": err}),
    };
    serde_json::to_string(&output).unwrap_or("null".to_string())
}

// C ABI types mirrored from core
type CJsonHandler = unsafe extern "C" fn(input_json: *const c_char) -> *mut c_char;
type CRegistrar = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CJsonHandler);

/// Register JSON handlers for the cpp plugin.
///
/// # Safety
/// - `ctx` and `registrar` must be valid pointers provided by the host runtime.
/// - Caller guarantees the lifetime of `ctx` for the duration of registration.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_register(ctx: *mut std::ffi::c_void, registrar: CRegistrar) {
    unsafe extern "C" fn list_compilers_handler(_input_json: *const c_char) -> *mut c_char {
        let out = list_compilers_json();
        let c = CString::new(out).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn compile_handler(input_json: *const c_char) -> *mut c_char {
        let json = if input_json.is_null() {
            serde_json::Value::Null
        } else {
            // SAFETY: `input_json` is provided by the host as a valid, NUL-terminated C string.
            let s = unsafe { CStr::from_ptr(input_json) }.to_string_lossy().into_owned();
            serde_json::from_str::<serde_json::Value>(&s).unwrap_or(serde_json::Value::Null)
        };
        let out = compile_json(&json);
        let c = CString::new(out).unwrap();
        dup_cstr(&c)
    }

    // Qualified runtime names to align with in-process ABI expectations.
    let n1 = CString::new("cpp.list_compilers").unwrap();
    unsafe { registrar(ctx, n1.as_ptr(), list_compilers_handler); }
    let n2 = CString::new("cpp.compile").unwrap();
    unsafe { registrar(ctx, n2.as_ptr(), compile_handler); }
}
/// Register typed handlers for the cpp plugin.
///
/// # Safety
/// - `ctx` and `registrar` must be valid pointers provided by the host runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_register_typed(ctx: *mut std::ffi::c_void, registrar: CTypedRegistrar) {
    unsafe extern "C" fn compile_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { cvalue_object(out, std::ptr::null(), 0); } return 0; }
        let a0 = unsafe { *args };
        let mut sources: Vec<String> = Vec::new();
        let mut flags: Vec<String> = Vec::new();
        let mut compiler: Option<String> = None;
        // sources
        if a0.tag == CTag::Array && !a0.arr.ptr.is_null() {
            for i in 0..a0.arr.len { let v = unsafe { *a0.arr.ptr.add(i) }; if v.tag == CTag::String && !v.s.ptr.is_null() { let s = unsafe { CStr::from_ptr(v.s.ptr) }.to_string_lossy().into_owned(); sources.push(s); } }
        }
        // flags
        if argc > 1 {
            let a1 = unsafe { *args.add(1) };
            if a1.tag == CTag::Array && !a1.arr.ptr.is_null() {
                for i in 0..a1.arr.len { let v = unsafe { *a1.arr.ptr.add(i) }; if v.tag == CTag::String && !v.s.ptr.is_null() { let s = unsafe { CStr::from_ptr(v.s.ptr) }.to_string_lossy().into_owned(); flags.push(s); } }
            }
        }
        // compiler
        if argc > 2 {
            let a2 = unsafe { *args.add(2) };
            if a2.tag == CTag::String && !a2.s.ptr.is_null() { compiler = Some(unsafe { CStr::from_ptr(a2.s.ptr) }.to_string_lossy().into_owned()); }
        }

        // Run compile
        fn candidate_compilers() -> Vec<&'static str> { #[cfg(target_os = "windows")] { vec!["cl", "g++", "clang++"] } #[cfg(not(target_os = "windows"))] { vec!["g++", "clang++", "clang", "gcc"] } }
        let (compiler_name, compiler_path) = if let Some(hint) = compiler.as_deref() { if let Ok(p) = which::which(hint) { (hint.to_string(), p) } else { common::find_available_compilers_from(&candidate_compilers()).into_iter().next().unwrap_or((String::new(), std::path::PathBuf::new())) } } else { common::find_available_compilers_from(&candidate_compilers()).into_iter().next().unwrap_or((String::new(), std::path::PathBuf::new())) };
        if compiler_name.is_empty() {
            // Build error object { ok:false, error:String }
            let n = 2usize; let bytes = std::mem::size_of::<CObjectEntry>() * n; let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry }; if ptr.is_null() { unsafe { cvalue_object(out, std::ptr::null(), 0); } return 0; }
            unsafe {
                (*ptr.add(0)).key = CStrView { ptr: CString::new("ok").unwrap().into_raw(), len: 2 };
                cvalue_bool(&mut (*ptr.add(0)).value as *mut CValue, false);
            }
            let msg = CString::new("No supported C++ compiler found on the system").unwrap(); let mptr = msg.as_ptr(); std::mem::forget(msg);
            unsafe {
                (*ptr.add(1)).key = CStrView { ptr: CString::new("error").unwrap().into_raw(), len: 5 };
                cvalue_string(&mut (*ptr.add(1)).value as *mut CValue, mptr, 46);
                cvalue_object(out, ptr, n);
            }
            return 0;
        }

        let out_name = if cfg!(target_os = "windows") { "output_binary.exe" } else { "output_binary" };
        let mut cmd = common::build_compile_command(&compiler_name, &compiler_path, &sources, &flags, out_name);
        if cfg!(target_os = "windows") && (compiler_name == "cl" || compiler_name.to_lowercase().contains("cl")) && let Some(envs) = common::ensure_msvc_env(compiler_path.as_path()) { cmd.envs(envs); }
        let output = match cmd.output() { Ok(o) => o, Err(e) => { let n = 2usize; let bytes = std::mem::size_of::<CObjectEntry>() * n; let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry }; if ptr.is_null() { unsafe { cvalue_object(out, std::ptr::null(), 0); } return 0; } let msg = CString::new(format!("Failed to execute compiler '{}': {}", compiler_name, e)).unwrap(); let mptr = msg.as_ptr(); std::mem::forget(msg); unsafe { (*ptr.add(0)).key = CStrView { ptr: CString::new("ok").unwrap().into_raw(), len:2 }; cvalue_bool(&mut (*ptr.add(0)).value as *mut CValue, false); (*ptr.add(1)).key = CStrView { ptr: CString::new("error").unwrap().into_raw(), len:5 }; cvalue_string(&mut (*ptr.add(1)).value as *mut CValue, mptr, compiler_name.len()+e.to_string().len()+31); cvalue_object(out, ptr, n); } return 0; } };
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let n = 2usize; let bytes = std::mem::size_of::<CObjectEntry>() * n; let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry }; if ptr.is_null() { unsafe { cvalue_object(out, std::ptr::null(), 0); } return 0; }
            let msg = CString::new(format!("Compilation failed: stdout:\n{}\nstderr:\n{}", stdout, stderr)).unwrap(); let mptr = msg.as_ptr(); std::mem::forget(msg);
            unsafe { (*ptr.add(0)).key = CStrView { ptr: CString::new("ok").unwrap().into_raw(), len:2 }; cvalue_bool(&mut (*ptr.add(0)).value as *mut CValue, false); (*ptr.add(1)).key = CStrView { ptr: CString::new("error").unwrap().into_raw(), len:5 }; cvalue_string(&mut (*ptr.add(1)).value as *mut CValue, mptr, stdout.len()+stderr.len()+28); cvalue_object(out, ptr, n); }
            return 0;
        }
        // Success { ok:true, path:String }
        let n = 2usize; let bytes = std::mem::size_of::<CObjectEntry>() * n; let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry }; if ptr.is_null() { unsafe { cvalue_object(out, std::ptr::null(), 0); } return 0; }
        let p = CString::new(out_name).unwrap(); let pptr = p.as_ptr(); std::mem::forget(p);
        unsafe { (*ptr.add(0)).key = CStrView { ptr: CString::new("ok").unwrap().into_raw(), len:2 }; cvalue_bool(&mut (*ptr.add(0)).value as *mut CValue, true); (*ptr.add(1)).key = CStrView { ptr: CString::new("path").unwrap().into_raw(), len:4 }; cvalue_string(&mut (*ptr.add(1)).value as *mut CValue, pptr, out_name.len()); cvalue_object(out, ptr, n); }
        0
    }

    // Register typed compile under qualified name
    let n = CString::new("cpp.compile").unwrap();
    unsafe { registrar(ctx, n.as_ptr(), compile_typed as CTypedHandler); }
}

/// Free a C string previously allocated by `dup_cstr`.
///
/// # Safety
/// - `ptr` must be a pointer returned from `dup_cstr` and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_free(ptr: *mut c_char) {
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
