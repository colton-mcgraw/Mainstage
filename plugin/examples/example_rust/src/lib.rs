use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// C ABI types mirrored from core
type CJsonHandler = unsafe extern "C" fn(input_json: *const c_char) -> *mut c_char;
type CRegistrar = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CJsonHandler);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_register(ctx: *mut std::ffi::c_void, registrar: CRegistrar) {
    // Handler: util.echo — echoes back the input JSON in an object
    unsafe extern "C" fn echo_handler(input_json: *const c_char) -> *mut c_char {
        if input_json.is_null() {
            let s = CString::new("null").unwrap();
            return libc::strdup(s.as_ptr());
        }
        let in_str = CStr::from_ptr(input_json).to_string_lossy().into_owned();
        let out = format!("{{\"ok\":true,\"args\":{}}}", in_str);
        let c = CString::new(out).unwrap();
        libc::strdup(c.as_ptr())
    }
    let name = CString::new("util.echo").unwrap();
    registrar(ctx, name.as_ptr(), echo_handler);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        libc::free(ptr as *mut libc::c_void);
    }
}
