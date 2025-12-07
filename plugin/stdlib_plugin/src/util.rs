use std::ffi::CString;
use std::os::raw::c_char;
use serde_json::Value as JsonValue;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            libc::free(ptr as *mut libc::c_void);
        }
    }
}

pub fn dup_cstr(s: &CString) -> *mut c_char {
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

pub fn parse_args(j: *const c_char) -> Vec<JsonValue> {
    if j.is_null() {
        return Vec::new();
    }
    let s = unsafe { std::ffi::CStr::from_ptr(j).to_string_lossy().into_owned() };
    let v = serde_json::from_str::<serde_json::Value>(&s).unwrap_or(JsonValue::Null);
    v.as_array().cloned().unwrap_or_default()
}

#[derive(serde::Serialize)]
pub struct ExecParsed {
    pub result: crate::domains::ExecResult,
}

pub fn parse_exec_args(j: *const c_char) -> ExecParsed {
    let args = parse_args(j);
    let cmd = args
        .first()
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
    ExecParsed { result: crate::domains::exec(cmd, a, cwd, env, timeout) }
}
