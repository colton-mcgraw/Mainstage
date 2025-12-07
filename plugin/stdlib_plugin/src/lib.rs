mod util;
mod domains;
mod typed;

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::collections::HashSet;

use util::{parse_args, dup_cstr};
use domains as d;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CStrView { pub ptr: *const c_char, pub len: usize }

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CArrayView { pub ptr: *const CValue, pub len: usize }

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CValue { pub tag: typed::CTag, pub b: bool, pub i: i64, pub f: f64, pub s: CStrView, pub arr: CArrayView, pub obj: typed::CObjectView }

type CJsonHandler = unsafe extern "C" fn(args_json: *const c_char) -> *mut c_char;
type CRegistrar = unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char, handler: CJsonHandler);

/// # Safety
/// This function is unsafe because it involves raw pointers and FFI.
/// The caller must ensure that the provided context and registrar function
/// are valid and adhere to the expected calling conventions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_register(ctx: *mut c_void, registrar: CRegistrar) {
    unsafe extern "C" fn util_echo(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let s = if args.is_empty() { String::new() } else { args[0].to_string() };
        let out = CString::new(s).unwrap(); dup_cstr(&out) as *mut c_char
    }

    unsafe extern "C" fn util_say(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let msg = if args.is_empty() { String::new() } else { args[0].to_string() };
        d::say(msg);
        let out = CString::new("").unwrap(); dup_cstr(&out) as *mut c_char
    }

    unsafe extern "C" fn util_ask(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let prompt = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        let ans = d::ask(prompt);
        let out = CString::new(ans).unwrap(); dup_cstr(&out) as *mut c_char
    }

    unsafe extern "C" fn util_array_empty(_args_json: *const c_char) -> *mut c_char {
        CString::new("[]").unwrap().into_raw()
    }

    unsafe extern "C" fn util_array_append(args_json: *const c_char) -> *mut c_char {
        let mut arr: Vec<serde_json::Value> = Vec::new();
        let args = parse_args(args_json);
        if let Some(a0) = args.first() && let Some(a) = a0.as_array() { arr = a.clone(); }
        if let Some(v) = args.get(1) { arr.push(v.clone()); }
        let json = serde_json::to_string(&arr).unwrap_or("[]".to_string());
        CString::new(json).unwrap().into_raw()
    }

    unsafe extern "C" fn util_array_extend(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let mut arr: Vec<serde_json::Value> = Vec::new();
        if let Some(a0) = args.first() && let Some(a) = a0.as_array() { arr = a.clone(); }
        if let Some(a1) = args.get(1) && let Some(ext) = a1.as_array() { arr.extend(ext.clone()); }
        let json = serde_json::to_string(&arr).unwrap_or("[]".to_string());
        CString::new(json).unwrap().into_raw()
    }

    unsafe extern "C" fn util_array_length(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let len = args.first().and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        CString::new(len.to_string()).unwrap().into_raw()
    }

    unsafe extern "C" fn env_get(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let key = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        match d::get_env(key) {
            Some(s) => { let out = CString::new(s).unwrap(); dup_cstr(&out) as *mut c_char }
            None => ptr::null_mut(),
        }
    }

    unsafe extern "C" fn env_set(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        if args.len() < 2 { return CString::new("false").unwrap().into_raw(); }
        let key = args[0].as_str().unwrap_or("").to_string();
        let value = args[1].as_str().unwrap_or("").to_string();
        d::set_env(key, value);
        CString::new("true").unwrap().into_raw()
    }

    unsafe extern "C" fn env_list(_args_json: *const c_char) -> *mut c_char {
        let items = d::list_env();
        let json = serde_json::to_string(&items).unwrap_or("[]".to_string());
        let s = CString::new(json).unwrap();
        dup_cstr(&s) as *mut c_char
    }

    unsafe extern "C" fn fs_read(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let path = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        match d::read(path) {
            Some(s) => { let out = CString::new(s).unwrap(); dup_cstr(&out) as *mut c_char }
            None => ptr::null_mut(),
        }
    }

    unsafe extern "C" fn fs_write(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        if args.len() < 2 { return CString::new("false").unwrap().into_raw(); }
        let path = args[0].as_str().unwrap_or("").to_string();
        let content = args[1].as_str().unwrap_or("").to_string();
        let ok = d::write(path, content);
        CString::new(if ok { "true" } else { "false" }).unwrap().into_raw()
    }

    unsafe extern "C" fn fs_delete(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let path = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        let ok = d::delete(path);
        CString::new(if ok { "true" } else { "false" }).unwrap().into_raw()
    }

    unsafe extern "C" fn fs_exists(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let path = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        let v = d::exists(path);
        CString::new(if v { "true" } else { "false" }).unwrap().into_raw()
    }

    unsafe extern "C" fn fs_glob(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let recursive = args.get(1).and_then(|v| v.as_bool()).unwrap_or(false);
        let mut set: HashSet<String> = HashSet::new();
        if let Some(first) = args.first() {
            if let Some(arr) = first.as_array() {
                for v in arr {
                    if let Some(pat) = v.as_str() {
                        for p in d::glob(pat.to_string(), Some(recursive)) { set.insert(p); }
                    }
                }
            } else if let Some(pat) = first.as_str() {
                for p in d::glob(pat.to_string(), Some(recursive)) { set.insert(p); }
            }
        }
        let mut out: Vec<String> = set.into_iter().collect();
        out.sort();
        let json = serde_json::to_string(&out).unwrap_or("[]".to_string());
        let s = CString::new(json).unwrap();
        dup_cstr(&s) as *mut c_char
    }

    unsafe extern "C" fn fs_make_dir(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let path = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        let ok = d::make_dir(path);
        CString::new(if ok { "true" } else { "false" }).unwrap().into_raw()
    }

    unsafe extern "C" fn fs_remove_dir(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let path = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        let ok = d::remove_dir(path);
        CString::new(if ok { "true" } else { "false" }).unwrap().into_raw()
    }

    unsafe extern "C" fn fs_list_dir(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let path = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        let items = d::list_dir(path);
        let json = serde_json::to_string(&items).unwrap_or("[]".to_string());
        let s = CString::new(json).unwrap();
        dup_cstr(&s) as *mut c_char
    }

    unsafe extern "C" fn fs_stat(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let path = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        let st = d::stat(path);
        let json = serde_json::to_string(&st).unwrap_or("{}".to_string());
        let s = CString::new(json).unwrap();
        dup_cstr(&s) as *mut c_char
    }

    unsafe extern "C" fn fs_copy(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        if args.len() < 2 { return CString::new("false").unwrap().into_raw(); }
        let from = args[0].as_str().unwrap_or("").to_string();
        let to = args[1].as_str().unwrap_or("").to_string();
        let ok = d::copy(from, to);
        CString::new(if ok { "true" } else { "false" }).unwrap().into_raw()
    }

    unsafe extern "C" fn fs_move(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        if args.len() < 2 { return CString::new("false").unwrap().into_raw(); }
        let from = args[0].as_str().unwrap_or("").to_string();
        let to = args[1].as_str().unwrap_or("").to_string();
        let ok = d::move_path(from, to);
        CString::new(if ok { "true" } else { "false" }).unwrap().into_raw()
    }

    unsafe extern "C" fn time_current_time_millis(_args_json: *const c_char) -> *mut c_char {
        let ts = d::current_time_millis();
        CString::new(ts.to_string()).unwrap().into_raw()
    }

    unsafe extern "C" fn time_sleep(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let ms = args.first().and_then(|v| v.as_i64()).unwrap_or(0);
        d::sleep(ms);
        CString::new("").unwrap().into_raw()
    }

    unsafe extern "C" fn rand_int(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        if args.len() < 2 { return CString::new("0").unwrap().into_raw(); }
        let min = args[0].as_i64().unwrap_or(0);
        let max = args[1].as_i64().unwrap_or(0);
        let v = d::random_int(min, max);
        CString::new(v.to_string()).unwrap().into_raw()
    }

    unsafe extern "C" fn rand_float(_args_json: *const c_char) -> *mut c_char {
        let v = d::random_float();
        CString::new(v.to_string()).unwrap().into_raw()
    }

    unsafe extern "C" fn proc_which(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let name = if args.is_empty() { String::new() } else { args[0].as_str().unwrap_or("").to_string() };
        match d::which(name) {
            Some(s) => CString::new(s).unwrap().into_raw(),
            None => ptr::null_mut(),
        }
    }

    unsafe extern "C" fn proc_exec(args_json: *const c_char) -> *mut c_char {
        let parsed = util::parse_exec_args(args_json);
        let json = serde_json::to_string(&parsed.result).unwrap_or("{}".to_string());
        let s = CString::new(json).unwrap();
        dup_cstr(&s) as *mut c_char
    }

    unsafe extern "C" fn proc_exit(args_json: *const c_char) -> *mut c_char {
        let args = parse_args(args_json);
        let code = args.first().and_then(|v| v.as_i64());
        d::exit(code);
        CString::new("").unwrap().into_raw()
    }

    macro_rules! reg { ($n:expr, $f:expr) => {{ let name = CString::new($n).unwrap(); unsafe { registrar(ctx, name.as_ptr(), $f) } }} }

    reg!("util.echo", util_echo);
    reg!("say", util_say);
    reg!("ask", util_ask);
    reg!("util.array.empty", util_array_empty);
    reg!("util.array.append", util_array_append);
    reg!("util.array.extend", util_array_extend);
    reg!("util.array.length", util_array_length);
    reg!("env.get", env_get);
    reg!("env.set", env_set);
    reg!("env.list", env_list);
    reg!("fs.read", fs_read);
    reg!("fs.write", fs_write);
    reg!("fs.delete", fs_delete);
    reg!("fs.exists", fs_exists);
    reg!("fs.glob", fs_glob);
    reg!("fs.make_dir", fs_make_dir);
    reg!("fs.remove_dir", fs_remove_dir);
    reg!("fs.list_dir", fs_list_dir);
    reg!("fs.stat", fs_stat);
    reg!("fs.copy", fs_copy);
    reg!("fs.move", fs_move);
    reg!("time.current_time_millis", time_current_time_millis);
    reg!("time.sleep", time_sleep);
    reg!("rand.int", rand_int);
    reg!("rand.float", rand_float);
    reg!("proc.which", proc_which);
    reg!("proc.exec", proc_exec);
    reg!("proc.exit", proc_exit);
}

/// # Safety
/// This function is unsafe because it involves raw pointers and FFI.
/// The caller must ensure that the provided context and registrar function
/// are valid and adhere to the expected calling conventions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_register_typed(ctx: *mut std::ffi::c_void, registrar: typed::CTypedRegistrar) {
    unsafe { typed::register_typed_impl(ctx, registrar) }
}
