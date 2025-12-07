use std::ffi::CString;
use crate::domains;
use crate::util::dup_cstr;
use common::typed::{CTag, CValue, CStrView, CArrayView, CObjectEntry, CObjectView, CTypedRegistrar};

// Types moved to crate common::typed

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mainstage_free_value(val: *const CValue) {
    if val.is_null() { return; }
    let v = unsafe { &*val };
    match v.tag {
        CTag::String => {
            if !v.s.ptr.is_null() { unsafe { libc::free(v.s.ptr as *mut libc::c_void) }; }
        }
        CTag::Array => {
            if !v.arr.ptr.is_null() {
                unsafe {
                    for i in 0..v.arr.len {
                        let elem = &*v.arr.ptr.add(i);
                        mainstage_free_value(elem as *const CValue);
                    }
                    libc::free(v.arr.ptr as *mut libc::c_void) 
                };
            }
        }
        CTag::Object => {
            if !v.obj.ptr.is_null() {
                unsafe {
                    for i in 0..v.obj.len {
                        let entry = &*v.obj.ptr.add(i);
                        if !entry.key.ptr.is_null() { libc::free(entry.key.ptr as *mut libc::c_void); }
                        mainstage_free_value(&entry.value as *const CValue);
                    }
                }
                unsafe { libc::free(v.obj.ptr as *mut libc::c_void) };
            }
        }
        _ => {}
    }
}

pub unsafe fn register_typed_impl(ctx: *mut std::ffi::c_void, registrar: CTypedRegistrar) {
    // Deep-copy a CValue (strings, arrays, objects) into plugin-owned allocations.
    unsafe fn deep_copy_value(src: &CValue) -> CValue {
        unsafe {
        match src.tag {
            CTag::Null => CValue { tag: CTag::Null, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } },
            CTag::Bool => CValue { tag: CTag::Bool, b: src.b, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } },
            CTag::Int => CValue { tag: CTag::Int, b: false, i: src.i, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } },
            CTag::Float => CValue { tag: CTag::Float, b: false, i: 0, f: src.f, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } },
            CTag::String => {
                let s = if src.s.ptr.is_null() { String::new() } else { std::ffi::CStr::from_ptr(src.s.ptr).to_string_lossy().into_owned() };
                let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
                CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: dup, len: cs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } }
            }
            CTag::Array => {
                if src.arr.ptr.is_null() || src.arr.len == 0 {
                    CValue { tag: CTag::Array, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } }
                } else {
                    let n = src.arr.len;
                    let bytes = std::mem::size_of::<CValue>() * n;
                    let ptr = libc::malloc(bytes) as *mut CValue;
                    if ptr.is_null() {
                        CValue { tag: CTag::Array, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } }
                    } else {
                        for i in 0..n {
                            let elem = *src.arr.ptr.add(i);
                            let cp = deep_copy_value(&elem);
                            *ptr.add(i) = cp;
                        }
                        CValue { tag: CTag::Array, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr, len: n }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } }
                    }
                }
            }
            CTag::Object => {
                if src.obj.ptr.is_null() || src.obj.len == 0 {
                    CValue { tag: CTag::Object, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } }
                } else {
                    let n = src.obj.len;
                    let bytes = std::mem::size_of::<CObjectEntry>() * n;
                    let ptr = libc::malloc(bytes) as *mut CObjectEntry;
                    if ptr.is_null() {
                        CValue { tag: CTag::Object, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } }
                    } else {
                        for i in 0..n {
                            let e = *src.obj.ptr.add(i);
                            let key_s = if e.key.ptr.is_null() { String::new() } else { std::ffi::CStr::from_ptr(e.key.ptr).to_string_lossy().into_owned() };
                            let kcs = CString::new(key_s).unwrap(); let kdup = dup_cstr(&kcs);
                            let v = deep_copy_value(&e.value);
                            *ptr.add(i) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: v };
                        }
                        CValue { tag: CTag::Object, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr, len: n } }
                    }
                }
            }
        }
        }
    }
        unsafe extern "C" fn util_array_empty_typed(_args: *const CValue, _argc: usize, out: *mut CValue) -> i32 {
            if out.is_null() { return -1; }
            unsafe {
                (*out).tag = CTag::Array;
                (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 };
                (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
            }
            0
        }

        unsafe extern "C" fn util_array_append_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
            if out.is_null() { return -1; }
            unsafe {
            let (base, add) = if args.is_null() || argc < 2 { (None, None) } else { (Some(*args), Some(*args.add(1))) };
            let base_len = match base { Some(b) if b.tag == CTag::Array && !b.arr.ptr.is_null() => b.arr.len, _ => 0 };
            let new_len = base_len + if add.is_some() { 1 } else { 0 };
            if new_len == 0 {
                (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
                return 0;
            }
            let bytes = std::mem::size_of::<CValue>() * new_len;
            let ptr = libc::malloc(bytes) as *mut CValue;
            if ptr.is_null() {
                (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
                return 0;
            }
            // Copy existing
            if let Some(b) = base && b.tag == CTag::Array && !b.arr.ptr.is_null() {
                for i in 0..b.arr.len { let cv = deep_copy_value(&*b.arr.ptr.add(i)); *ptr.add(i) = cv; }
            }
            // Append
            if let Some(v) = add { let cp = deep_copy_value(&v); *ptr.add(base_len) = cp; }
            (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr, len: new_len }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
            }
            0
        }

        unsafe extern "C" fn util_array_extend_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
            if out.is_null() { return -1; }
            unsafe {
            if args.is_null() || argc < 2 {
                (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
                return 0;
            }
            let a0 = *args; let a1 = *args.add(1);
            let len0 = if a0.tag == CTag::Array && !a0.arr.ptr.is_null() { a0.arr.len } else { 0 };
            let len1 = if a1.tag == CTag::Array && !a1.arr.ptr.is_null() { a1.arr.len } else { 0 };
            let new_len = len0 + len1;
            if new_len == 0 {
                (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
                return 0;
            }
            let bytes = std::mem::size_of::<CValue>() * new_len;
            let ptr = libc::malloc(bytes) as *mut CValue;
            if ptr.is_null() {
                (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
                return 0;
            }
            // Copy first
            if len0 > 0 { for i in 0..len0 { let cv = deep_copy_value(&*a0.arr.ptr.add(i)); *ptr.add(i) = cv; } }
            if len1 > 0 { for i in 0..len1 { let cv = deep_copy_value(&*a1.arr.ptr.add(i)); *ptr.add(len0 + i) = cv; } }
            (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr, len: new_len }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 };
            }
            0
        }
    unsafe extern "C" fn util_echo_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if argc == 0 || args.is_null() {
            unsafe { (*out).tag = CTag::Null; (*out).b = false; (*out).i = 0; (*out).f = 0.0; (*out).s = CStrView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
            return 0;
        }
        let first = unsafe { *args };
        unsafe {
            match first.tag {
                CTag::Null => { (*out).tag = CTag::Null; (*out).b = false; (*out).i = 0; (*out).f = 0.0; (*out).s = CStrView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; }
                CTag::Bool => { (*out).tag = CTag::Bool; (*out).b = first.b; }
                CTag::Int => { (*out).tag = CTag::Int; (*out).i = first.i; }
                CTag::Float => { (*out).tag = CTag::Float; (*out).f = first.f; }
                CTag::String => {
                    if !first.s.ptr.is_null() && first.s.len > 0 {
                        let s = std::ffi::CStr::from_ptr(first.s.ptr).to_string_lossy().into_owned();
                        let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
                        (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() };
                    } else {
                        (*out).tag = CTag::String; let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs);
                        (*out).s = CStrView { ptr: dup, len: 0 };
                    }
                }
                _ => { (*out).tag = CTag::Null; (*out).b = false; (*out).i = 0; (*out).f = 0.0; (*out).s = CStrView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; }
            }
        }
        0
    }

    unsafe extern "C" fn util_say_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        let mut msg = String::new();
        if !args.is_null() && argc > 0 {
            let a0 = unsafe { *args };
            match a0.tag {
                CTag::String => { if !a0.s.ptr.is_null() { unsafe { msg = std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned(); } } }
                CTag::Null => { msg = "null".to_string(); }
                CTag::Bool => { msg = if a0.b { "true".into() } else { "false".into() }; }
                CTag::Int => { msg = a0.i.to_string(); }
                CTag::Float => { msg = a0.f.to_string(); }
                _ => { msg.clear(); }
            }
        }
        domains::say(msg);
        unsafe { (*out).tag = CTag::Null; (*out).b = false; (*out).i = 0; (*out).f = 0.0; (*out).s = CStrView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
        0
    }

    unsafe extern "C" fn util_ask_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 {
            let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs);
            unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 }; }
            return 0;
        }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() {
            let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs);
            unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 }; }
            return 0;
        }
        let q = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let ans = domains::ask(q);
        let cs = CString::new(ans).unwrap(); let dup = dup_cstr(&cs);
        unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() }; }
        0
    }

    unsafe extern "C" fn env_get_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Null; } return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Null; } return 0; }
        let key = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        match domains::get_env(key) {
            Some(s) => { let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
                unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() }; }
            }
            None => unsafe { (*out).tag = CTag::Null; },
        }
        0
    }

    unsafe extern "C" fn fs_read_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Null; } return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Null; } return 0; }
        let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        match domains::read(path) {
            Some(s) => { let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
                unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() }; }
            }
            None => unsafe { (*out).tag = CTag::Null; },
        }
        0
    }

    unsafe extern "C" fn fs_write_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 2 { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) };
        if a0.tag != CTag::String || a1.tag != CTag::String || a0.s.ptr.is_null() || a1.s.ptr.is_null() {
            unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0;
        }
        let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let content = unsafe { std::ffi::CStr::from_ptr(a1.s.ptr).to_string_lossy().into_owned() };
        let ok = domains::write(path, content);
        unsafe { (*out).tag = CTag::Bool; (*out).b = ok; }
        0
    }

    unsafe extern "C" fn fs_delete_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let ok = domains::delete(path);
        unsafe { (*out).tag = CTag::Bool; (*out).b = ok; }
        0
    }

    unsafe extern "C" fn fs_make_dir_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let ok = domains::make_dir(path);
        unsafe { (*out).tag = CTag::Bool; (*out).b = ok; }
        0
    }

    unsafe extern "C" fn fs_remove_dir_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let ok = domains::remove_dir(path);
        unsafe { (*out).tag = CTag::Bool; (*out).b = ok; }
        0
    }

    unsafe extern "C" fn fs_copy_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 2 { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) };
        if a0.tag != CTag::String || a1.tag != CTag::String || a0.s.ptr.is_null() || a1.s.ptr.is_null() { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let from = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let to = unsafe { std::ffi::CStr::from_ptr(a1.s.ptr).to_string_lossy().into_owned() };
        let ok = domains::copy(from, to);
        unsafe { (*out).tag = CTag::Bool; (*out).b = ok; }
        0
    }

    unsafe extern "C" fn fs_move_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 2 { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) };
        if a0.tag != CTag::String || a1.tag != CTag::String || a0.s.ptr.is_null() || a1.s.ptr.is_null() { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let from = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let to = unsafe { std::ffi::CStr::from_ptr(a1.s.ptr).to_string_lossy().into_owned() };
        let ok = domains::move_path(from, to);
        unsafe { (*out).tag = CTag::Bool; (*out).b = ok; }
        0
    }

    unsafe extern "C" fn fs_exists_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        let mut v = false;
        if !args.is_null() && argc > 0 {
            let a0 = unsafe { *args };
            if a0.tag == CTag::String && !a0.s.ptr.is_null() {
                let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
                v = domains::exists(path);
            }
        }
        unsafe { (*out).tag = CTag::Bool; (*out).b = v; }
        0
    }

    unsafe extern "C" fn time_current_time_millis_typed(_args: *const CValue, _argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        let ts = domains::current_time_millis();
        unsafe { (*out).tag = CTag::Int; (*out).i = ts; }
        0
    }

    unsafe extern "C" fn rand_int_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 2 { unsafe { (*out).tag = CTag::Int; (*out).i = 0; } return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) };
        if a0.tag != CTag::Int || a1.tag != CTag::Int { unsafe { (*out).tag = CTag::Int; (*out).i = 0; } return 0; }
        let v = domains::random_int(a0.i, a1.i);
        unsafe { (*out).tag = CTag::Int; (*out).i = v; }
        0
    }

    unsafe extern "C" fn rand_float_typed(_args: *const CValue, _argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        let v = domains::random_float();
        unsafe { (*out).tag = CTag::Float; (*out).f = v; }
        0
    }

    unsafe extern "C" fn proc_which_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Null; } return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Null; } return 0; }
        let name = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        match domains::which(name) {
            Some(s) => { let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
                unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() }; }
            }
            None => unsafe { (*out).tag = CTag::Null; },
        }
        0
    }

    unsafe extern "C" fn env_set_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 2 { unsafe { (*out).tag = CTag::Bool; (*out).b = false; } return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) };
        if a0.tag != CTag::String || a1.tag != CTag::String || a0.s.ptr.is_null() || a1.s.ptr.is_null() {
            unsafe { (*out).tag = CTag::Bool; (*out).b = false; }
            return 0;
        }
        let key = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let value = unsafe { std::ffi::CStr::from_ptr(a1.s.ptr).to_string_lossy().into_owned() };
        domains::set_env(key, value);
        unsafe { (*out).tag = CTag::Bool; (*out).b = true; }
        0
    }

    unsafe extern "C" fn time_sleep_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        let ms = if !args.is_null() && argc > 0 { let a0 = unsafe { *args }; if a0.tag == CTag::Int { a0.i } else { 0 } } else { 0 };
        domains::sleep(ms);
        unsafe { (*out).tag = CTag::Null; (*out).b = false; (*out).i = 0; (*out).f = 0.0; (*out).s = CStrView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
        0
    }

    unsafe extern "C" fn proc_exit_typed(args: *const CValue, argc: usize, _out: *mut CValue) -> i32 {
        let code_opt = if !args.is_null() && argc > 0 { let a0 = unsafe { *args }; if a0.tag == CTag::Int { Some(a0.i) } else { None } } else { None };
        domains::exit(code_opt);
        0
    }

    unsafe extern "C" fn util_array_length_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        let len = if !args.is_null() && argc > 0 { let a0 = unsafe { *args }; if a0.tag == CTag::Array { a0.arr.len as i64 } else { 0 } } else { 0 };
        unsafe { (*out).tag = CTag::Int; (*out).i = len; }
        0
    }

    unsafe extern "C" fn fs_glob_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 {
            unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
            return 0;
        }
        let a0 = unsafe { *args };
        let recursive = if argc > 1 { let a1 = unsafe { *args.add(1) }; a1.tag == CTag::Bool && a1.b } else { false };
        let mut set: std::collections::HashSet<String> = std::collections::HashSet::new();
        match a0.tag {
            CTag::String => {
                if !a0.s.ptr.is_null() {
                    let pat = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
                    for p in domains::glob(pat, Some(recursive)) { set.insert(p); }
                }
            }
            CTag::Array => {
                if !a0.arr.ptr.is_null() && a0.arr.len > 0 {
                    for i in 0..a0.arr.len {
                        let elem = unsafe { *a0.arr.ptr.add(i) };
                        if elem.tag == CTag::String && !elem.s.ptr.is_null() {
                            let pat = unsafe { std::ffi::CStr::from_ptr(elem.s.ptr).to_string_lossy().into_owned() };
                            for p in domains::glob(pat, Some(recursive)) { set.insert(p); }
                        }
                    }
                }
            }
            _ => {}
        }
        let mut items: Vec<String> = set.into_iter().collect();
        items.sort();
        let n = items.len();
        if n == 0 {
            unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
            return 0;
        }
        let bytes = std::mem::size_of::<CValue>() * n;
        let ptr = unsafe { libc::malloc(bytes) as *mut CValue };
        if ptr.is_null() {
            unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
            return 0;
        }
        for (i, s) in items.iter().enumerate().take(n) {
            let cs = CString::new(s.as_str()).unwrap(); let dup = dup_cstr(&cs);
            let val = CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: dup, len: cs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(i) = val; }
        }
        unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr, len: n }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
        0
    }

    unsafe extern "C" fn env_list_typed(_args: *const CValue, _argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        let items = domains::list_env();
        let n = items.len();
        let bytes = std::mem::size_of::<CObjectEntry>() * n;
        let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry };
        if n > 0 && ptr.is_null() { unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } }; return 0; }
        for (i, kv) in items.iter().enumerate().take(n) {
            let kcs = CString::new(kv.key.as_str()).unwrap(); let kdup = dup_cstr(&kcs);
            let vcs = CString::new(kv.value.as_str()).unwrap(); let vdup = dup_cstr(&vcs);
            let entry = CObjectEntry {
                key: CStrView { ptr: kdup, len: kcs.as_bytes().len() },
                value: CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: vdup, len: vcs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } },
            };
            unsafe { *ptr.add(i) = entry; }
        }
        unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr, len: n }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } };
        0
    }

    unsafe extern "C" fn proc_exec_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        // args: cmd:String, argv:Array<String>?, cwd:String?, env:Object(String->String)?, timeout:Int?
        if args.is_null() || argc == 0 {
            unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } };
            return 0;
        }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() {
            unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } };
            return 0;
        }
        let cmd = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let argv = if argc > 1 {
            let a1 = unsafe { *args.add(1) };
            if a1.tag == CTag::Array && !a1.arr.ptr.is_null() && a1.arr.len > 0 {
                let mut v = Vec::with_capacity(a1.arr.len);
                for i in 0..a1.arr.len {
                    let e = unsafe { *a1.arr.ptr.add(i) };
                    if e.tag == CTag::String && !e.s.ptr.is_null() {
                        v.push(unsafe { std::ffi::CStr::from_ptr(e.s.ptr).to_string_lossy().into_owned() });
                    }
                }
                Some(v)
            } else { None }
        } else { None };
        let cwd = if argc > 2 {
            let a2 = unsafe { *args.add(2) };
            if a2.tag == CTag::String && !a2.s.ptr.is_null() {
                Some(unsafe { std::ffi::CStr::from_ptr(a2.s.ptr).to_string_lossy().into_owned() })
            } else { None }
        } else { None };
        let env = if argc > 3 {
            let a3 = unsafe { *args.add(3) };
            if a3.tag == CTag::Object && !a3.obj.ptr.is_null() && a3.obj.len > 0 {
                let mut m = serde_json::Map::new();
                for i in 0..a3.obj.len {
                    let entry = unsafe { *a3.obj.ptr.add(i) };
                    if !entry.key.ptr.is_null() && entry.value.tag == CTag::String && !entry.value.s.ptr.is_null() {
                        let k = unsafe { std::ffi::CStr::from_ptr(entry.key.ptr).to_string_lossy().into_owned() };
                        let v = unsafe { std::ffi::CStr::from_ptr(entry.value.s.ptr).to_string_lossy().into_owned() };
                        m.insert(k, serde_json::Value::String(v));
                    }
                }
                Some(serde_json::Value::Object(m))
            } else { None }
        } else { None };
        let timeout = if argc > 4 {
            let a4 = unsafe { *args.add(4) };
            if a4.tag == CTag::Int { Some(a4.i) } else { None }
        } else { None };
        let result = domains::exec(cmd, argv, cwd, env, timeout);
        // Build typed object { code:Int, stdout:String, stderr:String }
        let n = 3usize;
        let bytes = std::mem::size_of::<CObjectEntry>() * n;
        let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry };
        if ptr.is_null() {
            unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } };
            return 0;
        }
        // code
        {
            let kcs = CString::new("code").unwrap(); let kdup = dup_cstr(&kcs);
            let val = CValue { tag: CTag::Int, b: false, i: result.code as i64, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(0) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: val } };
        }
        // stdout
        {
            let kcs = CString::new("stdout").unwrap(); let kdup = dup_cstr(&kcs);
            let vcs = CString::new(result.stdout.as_str()).unwrap(); let vdup = dup_cstr(&vcs);
            let val = CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: vdup, len: vcs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(1) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: val } };
        }
        // stderr
        {
            let kcs = CString::new("stderr").unwrap(); let kdup = dup_cstr(&kcs);
            let vcs = CString::new(result.stderr.as_str()).unwrap(); let vdup = dup_cstr(&vcs);
            let val = CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: vdup, len: vcs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(2) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: val } };
        }
        unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr, len: n }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } };
        0
    }

    unsafe extern "C" fn fs_list_dir_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } }; return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } }; return 0; }
        let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let mut items = domains::list_dir(path);
        items.sort();
        let n = items.len();
        if n == 0 { unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } }; return 0; }
        let bytes = std::mem::size_of::<CValue>() * n;
        let ptr = unsafe { libc::malloc(bytes) as *mut CValue };
        if ptr.is_null() { unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } }; return 0; }
        for (i, s) in items.iter().enumerate().take(n) {
            let cs = CString::new(s.as_str()).unwrap(); let dup = dup_cstr(&cs);
            let val = CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: dup, len: cs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(i) = val; }
        }
        unsafe { (*out).tag = CTag::Array; (*out).arr = CArrayView { ptr, len: n }; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 } };
        0
    }

    unsafe extern "C" fn fs_stat_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } }; return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } }; return 0; }
        let path = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let st = domains::stat(path);
        let n = 3usize;
        let bytes = std::mem::size_of::<CObjectEntry>() * n;
        let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry };
        if ptr.is_null() { unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr: std::ptr::null(), len: 0 }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } }; return 0; }

        // size
        {
            let kcs = CString::new("size").unwrap(); let kdup = dup_cstr(&kcs);
            let val = CValue { tag: CTag::Int, b: false, i: st.size, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(0) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: val } };
        }
        // modified
        {
            let kcs = CString::new("modified").unwrap(); let kdup = dup_cstr(&kcs);
            let vcs = CString::new(st.modified.as_str()).unwrap(); let vdup = dup_cstr(&vcs);
            let val = CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: vdup, len: vcs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(1) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: val } };
        }
        // is_dir
        {
            let kcs = CString::new("is_dir").unwrap(); let kdup = dup_cstr(&kcs);
            let val = CValue { tag: CTag::Bool, b: st.is_dir, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } };
            unsafe { *ptr.add(2) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: val } };
        }

        unsafe { (*out).tag = CTag::Object; (*out).obj = CObjectView { ptr, len: n }; (*out).arr = CArrayView { ptr: std::ptr::null(), len: 0 } };
        0
    }

    // --- Path ---
    unsafe extern "C" fn path_normalize_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let p = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let mut buf = std::path::PathBuf::new();
        for comp in std::path::Path::new(&p).components() { buf.push(comp.as_os_str()); }
        let s = buf.to_string_lossy().into_owned();
        let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
        unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() } };
        0
    }

    unsafe extern "C" fn path_resolve_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 2 { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) };
        if a0.tag != CTag::String || a1.tag != CTag::String || a0.s.ptr.is_null() || a1.s.ptr.is_null() { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let base = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let rel = unsafe { std::ffi::CStr::from_ptr(a1.s.ptr).to_string_lossy().into_owned() };
        let s = std::path::Path::new(&base).join(&rel).to_string_lossy().into_owned();
        let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
        unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() } };
        0
    }

    unsafe extern "C" fn path_relativize_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 2 { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) };
        if a0.tag != CTag::String || a1.tag != CTag::String || a0.s.ptr.is_null() || a1.s.ptr.is_null() { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let from = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let to = unsafe { std::ffi::CStr::from_ptr(a1.s.ptr).to_string_lossy().into_owned() };
        let from_p = std::path::Path::new(&from);
        let to_p = std::path::Path::new(&to);
        let rel = match to_p.strip_prefix(from_p) { Ok(p) => p.to_path_buf(), Err(_) => to_p.to_path_buf() };
        let s = rel.to_string_lossy().into_owned();
        let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
        unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() } };
        0
    }

    // --- JSON ---
    fn json_to_cvalue(v: &serde_json::Value) -> CValue {
        match v {
            serde_json::Value::Null => CValue { tag: CTag::Null, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } },
            serde_json::Value::Bool(b) => CValue { tag: CTag::Bool, b: *b, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } },
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() { CValue { tag: CTag::Int, b: false, i, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } } }
                else { CValue { tag: CTag::Float, b: false, i: 0, f: n.as_f64().unwrap_or(0.0), s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } } }
            }
            serde_json::Value::String(s) => { let cs = CString::new(s.as_str()).unwrap(); let dup = dup_cstr(&cs); CValue { tag: CTag::String, b: false, i: 0, f: 0.0, s: CStrView { ptr: dup, len: cs.as_bytes().len() }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } } }
            serde_json::Value::Array(arr) => {
                let n = arr.len();
                if n == 0 { CValue { tag: CTag::Array, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } } }
                else {
                    let bytes = std::mem::size_of::<CValue>() * n;
                    let ptr = unsafe { libc::malloc(bytes) as *mut CValue };
                    if ptr.is_null() { CValue { tag: CTag::Array, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } } }
                    else {
                        for (i, v) in arr.iter().enumerate() { let cv = json_to_cvalue(v); unsafe { *ptr.add(i) = cv; } }
                        CValue { tag: CTag::Array, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr, len: n }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } }
                    }
                }
            }
            serde_json::Value::Object(obj) => {
                let n = obj.len();
                if n == 0 { CValue { tag: CTag::Object, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } } }
                else {
                    let bytes = std::mem::size_of::<CObjectEntry>() * n;
                    let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry };
                    if ptr.is_null() { CValue { tag: CTag::Object, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr: std::ptr::null(), len: 0 } } }
                    else {
                        for (i, (k, v)) in obj.iter().enumerate() {
                            let kcs = CString::new(k.as_str()).unwrap(); let kdup = dup_cstr(&kcs);
                            let vv = json_to_cvalue(v);
                            unsafe { *ptr.add(i) = CObjectEntry { key: CStrView { ptr: kdup, len: kcs.as_bytes().len() }, value: vv } };
                        }
                        CValue { tag: CTag::Object, b: false, i: 0, f: 0.0, s: CStrView { ptr: std::ptr::null(), len: 0 }, arr: CArrayView { ptr: std::ptr::null(), len: 0 }, obj: CObjectView { ptr, len: n } }
                    }
                }
            }
        }
    }

    unsafe fn cvalue_to_json(v: &CValue) -> serde_json::Value {
        match v.tag {
            CTag::Null => serde_json::Value::Null,
            CTag::Bool => serde_json::Value::Bool(v.b),
            CTag::Int => serde_json::Value::Number(serde_json::Number::from(v.i)),
            CTag::Float => serde_json::Value::Number(serde_json::Number::from_f64(v.f).unwrap_or_else(|| serde_json::Number::from_f64(0.0).unwrap())),
            CTag::String => {
                if v.s.ptr.is_null() { serde_json::Value::String(String::new()) }
                else { let s = unsafe { std::ffi::CStr::from_ptr(v.s.ptr).to_string_lossy().into_owned() }; serde_json::Value::String(s) }
            }
            CTag::Array => {
                if v.arr.ptr.is_null() || v.arr.len == 0 { serde_json::Value::Array(vec![]) }
                else {
                    let mut a = Vec::with_capacity(v.arr.len);
                    for i in 0..v.arr.len { let e = unsafe { &*v.arr.ptr.add(i) }; a.push(unsafe { cvalue_to_json(e) }); }
                    serde_json::Value::Array(a)
                }
            }
            CTag::Object => {
                if v.obj.ptr.is_null() || v.obj.len == 0 { serde_json::Value::Object(serde_json::Map::new()) }
                else {
                    let mut m = serde_json::Map::new();
                    for i in 0..v.obj.len {
                        let entry = unsafe { &*v.obj.ptr.add(i) };
                        let k = if entry.key.ptr.is_null() { String::new() } 
                        else { unsafe { std::ffi::CStr::from_ptr(entry.key.ptr).to_string_lossy().into_owned() } };
                        let val = unsafe { cvalue_to_json(&entry.value) };
                        m.insert(k, val);
                    }
                    serde_json::Value::Object(m)
                }
            }
        }
    }

    unsafe extern "C" fn json_parse_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { unsafe { (*out).tag = CTag::Null; } return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { unsafe { (*out).tag = CTag::Null; } return 0; }
        let s = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => { let cv = json_to_cvalue(&v); unsafe { *out = cv; } }
            Err(_) => unsafe { (*out).tag = CTag::Null; },
        }
        0
    }

    unsafe extern "C" fn json_stringify_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { let cs = CString::new("null").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() } }; return 0; }
        let a0 = unsafe { *args };
        let v = unsafe { cvalue_to_json(&a0) };
        let s = serde_json::to_string(&v).unwrap_or_else(|_| "null".into());
        let cs = CString::new(s).unwrap(); let dup = dup_cstr(&cs);
        unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() } };
        0
    }

    // --- String ---
    unsafe extern "C" fn string_trim_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc == 0 { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let a0 = unsafe { *args };
        if a0.tag != CTag::String || a0.s.ptr.is_null() { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let s = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let t = s.trim().to_string();
        let cs = CString::new(t).unwrap(); let dup = dup_cstr(&cs);
        unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() } };
        0
    }

    unsafe extern "C" fn string_replace_typed(args: *const CValue, argc: usize, out: *mut CValue) -> i32 {
        if out.is_null() { return -1; }
        if args.is_null() || argc < 3 { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let a0 = unsafe { *args }; let a1 = unsafe { *args.add(1) }; let a2 = unsafe { *args.add(2) };
        if a0.tag != CTag::String || a1.tag != CTag::String || a2.tag != CTag::String || a0.s.ptr.is_null() || a1.s.ptr.is_null() || a2.s.ptr.is_null() { let cs = CString::new("").unwrap(); let dup = dup_cstr(&cs); unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: 0 } }; return 0; }
        let s = unsafe { std::ffi::CStr::from_ptr(a0.s.ptr).to_string_lossy().into_owned() };
        let from = unsafe { std::ffi::CStr::from_ptr(a1.s.ptr).to_string_lossy().into_owned() };
        let to = unsafe { std::ffi::CStr::from_ptr(a2.s.ptr).to_string_lossy().into_owned() };
        let r = s.replace(&from, &to);
        let cs = CString::new(r).unwrap(); let dup = dup_cstr(&cs);
        unsafe { (*out).tag = CTag::String; (*out).s = CStrView { ptr: dup, len: cs.as_bytes().len() } };
        0
    }

    macro_rules! treg { ($n:expr, $f:expr) => {{ let name = CString::new($n).unwrap(); unsafe { registrar(ctx, name.as_ptr(), $f) }; }} }

    treg!("util.echo_typed", util_echo_typed);
    treg!("util.say", util_say_typed);
    treg!("util.ask", util_ask_typed);
    treg!("env.get", env_get_typed);
    treg!("fs.read", fs_read_typed);
    treg!("fs.write", fs_write_typed);
    treg!("fs.delete", fs_delete_typed);
    treg!("fs.exists", fs_exists_typed);
    treg!("fs.make_dir", fs_make_dir_typed);
    treg!("fs.remove_dir", fs_remove_dir_typed);
    treg!("fs.copy", fs_copy_typed);
    treg!("fs.move", fs_move_typed);
    treg!("fs.glob", fs_glob_typed);
    treg!("env.list", env_list_typed);
    treg!("proc.exec", proc_exec_typed);
    treg!("fs.list_dir", fs_list_dir_typed);
    treg!("fs.stat", fs_stat_typed);
    treg!("time.current_time_millis", time_current_time_millis_typed);
    treg!("rand.int", rand_int_typed);
    treg!("rand.float", rand_float_typed);
    treg!("proc.which", proc_which_typed);
    treg!("env.set", env_set_typed);
    treg!("time.sleep", time_sleep_typed);
    treg!("proc.exit", proc_exit_typed);
    treg!("util.array.length", util_array_length_typed);
    treg!("util.array.empty", util_array_empty_typed);
    treg!("util.array.append", util_array_append_typed);
    treg!("util.array.extend", util_array_extend_typed);
    // New domains
    treg!("path.normalize", path_normalize_typed);
    treg!("path.resolve", path_resolve_typed);
    treg!("path.relativize", path_relativize_typed);
    treg!("json.parse", json_parse_typed);
    treg!("json.stringify", json_stringify_typed);
    treg!("string.trim", string_trim_typed);
    treg!("string.replace", string_replace_typed);
}
