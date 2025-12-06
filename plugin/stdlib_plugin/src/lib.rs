use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::ffi::CString;
use std::os::raw::c_char;

type CJsonHandler = unsafe extern "C" fn(input_json: *const c_char) -> *mut c_char;
type CRegistrar =
    unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CJsonHandler);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_register(ctx: *mut std::ffi::c_void, registrar: CRegistrar) {
    // Helper macros to reduce boilerplate for JSON arg extraction
    macro_rules! str_arg {
        ($args:expr, $i:expr) => {
            $args
                .get($i)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
    }
    macro_rules! opt_bool {
        ($args:expr, $i:expr) => {
            $args.get($i).and_then(|v| v.as_bool())
        };
    }
    // macro_rules! opt_int {($args:expr, $i:expr) => { $args.get($i).and_then(|v| v.as_i64()) } }
    macro_rules! int_arg {
        ($args:expr, $i:expr) => {
            $args.get($i).and_then(|v| v.as_i64()).unwrap_or(0)
        };
    }

    // IO
    unsafe extern "C" fn util_ask(j: *const c_char) -> *mut c_char {
        let args = parse_args(j);
        let out = serde_json::to_string(&JsonValue::String(ask(str_arg!(args, 0)))).unwrap();
        let c = CString::new(out).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn util_say(j: *const c_char) -> *mut c_char {
        let args = parse_args(j);
        let v = args.get(0).cloned().unwrap_or(JsonValue::Null);
        let s = match v {
            JsonValue::String(s) => s,
            JsonValue::Null => "null".to_string(),
            _ => serde_json::to_string(&v).unwrap_or_else(|_| "".to_string()),
        };
        say(s);
        let c = CString::new("null").unwrap();
        dup_cstr(&c)
    }

    // Env
    unsafe extern "C" fn env_get(j: *const c_char) -> *mut c_char {
        let args = parse_args(j);
        let v = match get_env(str_arg!(args, 0)) {
            Some(s) => JsonValue::String(s),
            None => JsonValue::Null,
        };
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn env_set(j: *const c_char) -> *mut c_char {
        let args = parse_args(j);
        set_env(str_arg!(args, 0), str_arg!(args, 1));
        let c = CString::new("null").unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn env_list(_j: *const c_char) -> *mut c_char {
        let envs = list_env();
        let c = CString::new(serde_json::to_string(&envs).unwrap()).unwrap();
        dup_cstr(&c)
    }

    // FS
    unsafe extern "C" fn fs_read(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = match read(str_arg!(a, 0)) {
            Some(s) => JsonValue::String(s),
            None => JsonValue::Null,
        };
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_write(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Bool(write(str_arg!(a, 0), str_arg!(a, 1)));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_delete(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Bool(delete(str_arg!(a, 0)));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_exists(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Bool(exists(str_arg!(a, 0)));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_list_dir(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = serde_json::to_value(list_dir(str_arg!(a, 0))).unwrap_or(JsonValue::Null);
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_make_dir(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Bool(make_dir(str_arg!(a, 0)));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_remove_dir(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Bool(remove_dir(str_arg!(a, 0)));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_glob(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let rec = opt_bool!(a, 1).unwrap_or(false);
        // Accept one or more patterns: String or Array<String>
        let patterns: Vec<String> = match a.get(0) {
            Some(JsonValue::String(s)) => vec![s.clone()],
            Some(JsonValue::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };
        let mut all: Vec<String> = Vec::new();
        for pat in patterns.iter() {
            let mut matches = glob(pat.clone(), Some(rec));
            all.append(&mut matches);
        }
        // Deduplicate while preserving order
        let mut seen = std::collections::HashSet::new();
        all.retain(|p| seen.insert(p.clone()));
        let v = serde_json::to_value(all).unwrap_or(JsonValue::Null);
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_copy(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Bool(copy(str_arg!(a, 0), str_arg!(a, 1)));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_move(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Bool(move_path(str_arg!(a, 0), str_arg!(a, 1)));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn fs_stat(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = serde_json::to_value(stat(str_arg!(a, 0))).unwrap_or(JsonValue::Null);
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }

    // Time/Random
    unsafe extern "C" fn time_sleep(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        sleep(int_arg!(a, 0));
        let c = CString::new("null").unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn time_current_time_millis(_j: *const c_char) -> *mut c_char {
        let v = JsonValue::Number(serde_json::Number::from(current_time_millis()));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn rand_int(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = JsonValue::Number(serde_json::Number::from(random_int(
            int_arg!(a, 0),
            int_arg!(a, 1),
        )));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn rand_float(_j: *const c_char) -> *mut c_char {
        let v = JsonValue::Number(
            serde_json::Number::from_f64(random_float())
                .unwrap_or_else(|| serde_json::Number::from(0)),
        );
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }

    // Process
    unsafe extern "C" fn proc_which(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        let v = match which(str_arg!(a, 0)) {
            Some(s) => JsonValue::String(s),
            None => JsonValue::Null,
        };
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn proc_exec(j: *const c_char) -> *mut c_char {
        let a = parse_exec_args(j);
        let v = serde_json::to_value(a.result).unwrap_or(JsonValue::Null);
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }

    // Exit
    unsafe extern "C" fn proc_exit(j: *const c_char) -> *mut c_char {
        let a = parse_args(j);
        exit(a.get(0).and_then(|v| v.as_i64()));
        let c = CString::new("null").unwrap();
        dup_cstr(&c)
    }

    // Array utilities (functional style: return new arrays)
    unsafe extern "C" fn array_empty(_j: *const c_char) -> *mut c_char {
        let v = JsonValue::Array(Vec::new());
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn array_append(j: *const c_char) -> *mut c_char {
        let args = parse_args(j);
        let mut arr: Vec<JsonValue> = args
            .get(0)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let item = args.get(1).cloned().unwrap_or(JsonValue::Null);
        arr.push(item);
        let v = JsonValue::Array(arr);
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }
    unsafe extern "C" fn array_extend(j: *const c_char) -> *mut c_char {
        let args = parse_args(j);
        let mut arr: Vec<JsonValue> = args
            .get(0)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let items: Vec<JsonValue> = args
            .get(1)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        arr.extend(items.into_iter());
        let v = JsonValue::Array(arr);
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }

    unsafe extern "C" fn array_length(j: *const c_char) -> *mut c_char {
        let args = parse_args(j);
        let len = args
            .get(0)
            .and_then(|v| v.as_array())
            .map(|a| a.len() as i64)
            .unwrap_or(0);
        let v = JsonValue::Number(serde_json::Number::from(len));
        let c = CString::new(serde_json::to_string(&v).unwrap()).unwrap();
        dup_cstr(&c)
    }

    // Registrar: register functions with domain names
    macro_rules! reg {
        ($n:expr, $f:expr) => {{
            let name = CString::new($n).unwrap();
            registrar(ctx, name.as_ptr(), $f);
        }};
    }
    reg!("ask", util_ask);
    reg!("say", util_say);
    reg!("env.get", env_get);
    reg!("env.set", env_set);
    reg!("env.list", env_list);
    reg!("fs.read", fs_read);
    reg!("fs.write", fs_write);
    reg!("fs.delete", fs_delete);
    reg!("fs.exists", fs_exists);
    reg!("fs.list_dir", fs_list_dir);
    reg!("fs.make_dir", fs_make_dir);
    reg!("fs.remove_dir", fs_remove_dir);
    reg!("fs.glob", fs_glob);
    reg!("fs.copy", fs_copy);
    reg!("fs.move", fs_move);
    reg!("fs.stat", fs_stat);
    reg!("time.sleep", time_sleep);
    reg!("time.current_time_millis", time_current_time_millis);
    reg!("rand.int", rand_int);
    reg!("rand.float", rand_float);
    reg!("proc.which", proc_which);
    reg!("proc.exec", proc_exec);
    reg!("proc.exit", proc_exit);
    reg!("util.array.empty", array_empty);
    reg!("util.array.append", array_append);
    reg!("util.array.extend", array_extend);
    reg!("util.array.length", array_length);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            libc::free(ptr as *mut libc::c_void);
        }
    }
}

// --- IO ---
fn ask(question: String) -> String {
    print!("{} ", question);
    let _ = io::stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
    buf.trim_end().to_string()
}

fn say(message: String) {
    println!("{}", message);
}

// --- Env ---
fn get_env(key: String) -> Option<String> {
    std::env::var(&key).ok()
}

fn set_env(key: String, value: String) {
    unsafe {
        std::env::set_var(key, value);
    }
}

#[derive(Serialize)]
struct EnvKV {
    key: String,
    value: String,
}

fn list_env() -> Vec<EnvKV> {
    std::env::vars()
        .map(|(k, v)| EnvKV { key: k, value: v })
        .collect()
}

// --- FS ---
fn read(path: String) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn write(path: String, content: String) -> bool {
    fs::write(path, content).is_ok()
}

fn delete(path: String) -> bool {
    fs::remove_file(path).is_ok()
}

fn exists(path: String) -> bool {
    Path::new(&path).exists()
}

fn list_dir(path: String) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(path) {
        for e in rd.flatten() {
            out.push(e.path().to_string_lossy().to_string());
        }
    }
    out
}

fn make_dir(path: String) -> bool {
    fs::create_dir_all(path).is_ok()
}

fn remove_dir(path: String) -> bool {
    fs::remove_dir_all(path).is_ok()
}

fn glob(pattern: String, recursive: Option<bool>) -> Vec<String> {
    let recursive = recursive.unwrap_or(false);
    let pattern_path = PathBuf::from(&pattern);
    let (base, pat) = if let Some(parent) = pattern_path.parent() {
        (
            parent.to_path_buf(),
            pattern_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or("*".into()),
        )
    } else {
        (PathBuf::from("."), pattern.clone())
    };
    let mut out = Vec::new();
    let walker: Box<dyn Iterator<Item = fs::DirEntry>> = if recursive {
        Box::new(walk_dir_recursive(base))
    } else {
        Box::new(
            fs::read_dir(base)
                .unwrap_or_else(|_| fs::read_dir(".").unwrap())
                .flatten(),
        )
    };
    for e in walker {
        let p = e.path();
        if let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) {
            if glob_match(&name, &pat) {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
    out
}

fn walk_dir_recursive(base: PathBuf) -> impl Iterator<Item = fs::DirEntry> {
    let mut stack = vec![base];
    let mut items: Vec<fs::DirEntry> = Vec::new();
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                }
                items.push(e);
            }
        }
    }
    items.into_iter()
}

fn glob_match(name: &str, pat: &str) -> bool {
    // Very simple wildcard: '*' matches any sequence; '?' matches one char.
    // Not perfect; replace with glob crate later if desired.
    let n: Vec<char> = name.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    let mut i = 0usize;
    let mut j = 0usize;
    let mut si = 0usize;
    let mut sj = usize::MAX;
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || n[i] == p[j]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            sj = j;
            si = i;
            j += 1;
        } else if sj != usize::MAX {
            j = sj + 1;
            si += 1;
            i = si;
        } else {
            return false;
        }
    }
    while j < p.len() && p[j] == '*' {
        j += 1;
    }
    j == p.len()
}

fn copy(from: String, to: String) -> bool {
    let src = Path::new(&from);
    if src.is_dir() {
        copy_dir_recursive(src, Path::new(&to)).is_ok()
    } else {
        fs::copy(src, to).is_ok()
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let p = entry.path();
        let target = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir_recursive(&p, &target)?;
        } else {
            fs::copy(&p, &target)?;
        }
    }
    Ok(())
}

fn move_path(from: String, to: String) -> bool {
    fs::rename(from, to).is_ok()
}

#[derive(Serialize)]
struct Stat {
    size: i64,
    modified: String,
    is_dir: bool,
}

fn stat(path: String) -> Stat {
    match fs::metadata(&path) {
        Ok(m) => {
            let size = m.len() as i64;
            let is_dir = m.is_dir();
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis().to_string())
                .unwrap_or_default();
            Stat {
                size,
                modified,
                is_dir,
            }
        }
        Err(_) => Stat {
            size: -1,
            modified: String::new(),
            is_dir: false,
        },
    }
}

// --- Time/Random ---
fn sleep(milliseconds: i64) {
    if milliseconds > 0 {
        thread::sleep(Duration::from_millis(milliseconds as u64));
    }
}

fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn random_int(min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    let mut rng = rand::thread_rng();
    rng.gen_range(min..max)
}

fn random_float() -> f64 {
    let mut rng = rand::thread_rng();
    rng.r#gen::<f64>()
}

// --- Process ---
fn which(name: String) -> Option<String> {
    which::which(name)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

#[derive(Serialize)]
struct ExecResult {
    code: i32,
    stdout: String,
    stderr: String,
}

fn exec(
    command: String,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<serde_json::Value>,
    _timeout: Option<i64>,
) -> ExecResult {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(command);
    if let Some(a) = args {
        cmd.args(a);
    }
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    if let Some(e) = env {
        if let Some(obj) = e.as_object() {
            for (k, v) in obj.iter() {
                if let Some(s) = v.as_str() {
                    cmd.env(k, s);
                }
            }
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            return ExecResult {
                code: -1,
                stdout: String::new(),
                stderr: e.to_string(),
            }
        }
    };
    ExecResult {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

// --- Exit ---
fn exit(code: Option<i64>) {
    std::process::exit(code.unwrap_or(0) as i32)
}
fn dup_cstr(s: &CString) -> *mut c_char {
    unsafe {
        let bytes = s.as_bytes_with_nul();
        let ptr = libc::malloc(bytes.len()) as *mut u8;
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        ptr as *mut c_char
    }
}

// --- Helpers to parse C JSON args ---
fn parse_args(j: *const c_char) -> Vec<JsonValue> {
    if j.is_null() {
        return Vec::new();
    }
    let s = unsafe { std::ffi::CStr::from_ptr(j).to_string_lossy().into_owned() };
    let v = serde_json::from_str::<serde_json::Value>(&s).unwrap_or(JsonValue::Null);
    v.as_array().cloned().unwrap_or_default()
}

struct ExecParsed {
    result: ExecResult,
}
fn parse_exec_args(j: *const c_char) -> ExecParsed {
    let args = parse_args(j);
    let cmd = args
        .get(0)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let a = args.get(1).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    });
    let cwd = args.get(2).and_then(|v| v.as_str()).map(|s| s.to_string());
    let env = args.get(3).cloned();
    let timeout = args.get(4).and_then(|v| v.as_i64());
    ExecParsed {
        result: exec(cmd, a, cwd, env, timeout),
    }
}
