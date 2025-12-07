//! file: core/src/vm/inprocess.rs
//! description: in-process dynamic library plugin adapter using `libloading`.
//!
//! This adapter loads a shared library at runtime and resolves a Rust-friendly
//! registration symbol (`mainstage_register`) so the plugin can register
//! handlers directly without a JSON bridge.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::Seek;
use std::os::raw::c_char;
use std::path::Path;
use std::sync::Arc;

use crate::vm::plugin::{Plugin, PluginMetadata};
use crate::vm::value::{Value as VmValue, json_to_value, values_to_json_array};
use async_trait::async_trait;
use libloading::{Library, Symbol};
use serde_json::Value as JsonValue;

// C ABI types: plugin provides JSON handler functions as C string in/out.
type CJsonHandler = unsafe extern "C" fn(input_json: *const c_char) -> *mut c_char;
type CFreeFn = unsafe extern "C" fn(ptr: *mut c_char);

// Core provides a C ABI registrar callback: plugin calls this per function.
type CRegistrar =
    unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CJsonHandler);

// Symbol signature the plugin must export.
type RegisterFn = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, registrar: CRegistrar);

// Optional typed ABI (prototype): pass typed values via C structs instead of JSON.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum CTag {
    Null = 0,
    Bool = 1,
    Int = 2,
    Float = 3,
    String = 4,
    Array = 5,
    Object = 6,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CStrView {
    ptr: *const c_char,
    len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CArrayView {
    ptr: *const CValue,
    len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CObjectEntry {
    key: CStrView,
    value: CValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CObjectView {
    ptr: *const CObjectEntry,
    len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CValue {
    tag: CTag,
    b: bool,
    i: i64,
    f: f64,
    s: CStrView,
    arr: CArrayView,
    obj: CObjectView,
}

type CTypedHandler =
    unsafe extern "C" fn(args: *const CValue, argc: usize, out: *mut CValue) -> i32;
type CTypedRegistrar =
    unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CTypedHandler);
type TypedRegisterFn = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, registrar: CTypedRegistrar);
type CFreeValueFn = unsafe extern "C" fn(val: *const CValue);

pub struct InProcessPlugin {
    _lib: Arc<Library>,
    name: String,
    handlers: HashMap<String, Box<dyn Fn(JsonValue) -> JsonValue + Send + Sync>>,
    typed_handlers: HashMap<String, CTypedHandler>,
    free_fn: Option<CFreeFn>,
    free_value_fn: Option<CFreeValueFn>,
}

impl InProcessPlugin {
    pub fn new(path: &Path) -> Result<Self, String> {
        // Validate path exists and is a file before attempting to load.
        if !path.exists() {
            return Err(format!("library path does not exist: {}", path.display()));
        }
        match std::fs::metadata(path) {
            Ok(m) if m.is_file() => {}
            _ => return Err(format!("library path is not a file: {}", path.display())),
        }

        unsafe {
            let lib = Library::new(path).map_err(|e| {
                // Best-effort: try to detect binary format and suggest arch mismatches
                let mut hint = String::new();
                if let Ok(ba) = guess_binary_arch(path) {
                    let host = std::env::consts::ARCH.to_string();
                    if ba != host {
                        hint = format!(" Detected binary arch '{}', host arch '{}'.", ba, host);
                    }
                }
                format!(
                    "failed to load library {}: {}. Hint: verify the file is a valid shared library for this OS/architecture and that it exports the expected symbols.{}",
                    path.display(), e, hint
                )
            })?;

            // Determine a default plugin name from file stem
            let cname = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("inproc")
                .to_string();

            // Resolve mainstage_register and let the plugin register handlers via C ABI.
            let reg_sym: Symbol<RegisterFn> = lib.get(b"mainstage_register\0").map_err(|e| {
                format!(
                    "missing symbol 'mainstage_register' in {}: {}. Ensure the plugin exports 'mainstage_register' with extern \"C\" and #[no_mangle].",
                    path.display(), e
                )
            })?;
            let reg_fn = *reg_sym;
            // Collect handlers via a C ABI registrar callback.
            #[derive(Default)]
            struct HostCtx {
                handlers: HashMap<String, Box<dyn Fn(JsonValue) -> JsonValue + Send + Sync>>,
                free_fn: Option<CFreeFn>,
            }
            let mut host_ctx = HostCtx::default();
            // Optional plugin-provided free function to release returned C strings.
            let plugin_free_sym: Symbol<CFreeFn> = lib.get(b"mainstage_free\0").map_err(|e| {
                format!(
                    "missing symbol 'mainstage_free' in {}: {}. In-process plugins must export 'mainstage_free' to release returned C strings.",
                    path.display(), e
                )
            })?;
            host_ctx.free_fn = Some(*plugin_free_sym);
            // Optional deep free for typed values (arrays/objects)
            let free_value_fn: Option<CFreeValueFn> =
                match lib.get::<CFreeValueFn>(b"mainstage_free_value\0") {
                    Ok(sym) => Some(*sym),
                    Err(_) => None,
                };
            // Context is a raw pointer to our handlers map.
            #[allow(clippy::not_unsafe_ptr_arg_deref)]
            unsafe extern "C" fn host_registrar(
                ctx: *mut std::ffi::c_void,
                name: *const c_char,
                handler: CJsonHandler,
            ) {
                // Safety: ctx is a &mut HostCtx passed from Rust.
                let host = unsafe { &mut *(ctx as *mut HostCtx) };
                let cname = if name.is_null() {
                    "".to_string()
                } else {
                    unsafe { CStr::from_ptr(name).to_string_lossy().into_owned() }
                };
                // Wrap the C handler in a safe Rust closure.
                let h: CJsonHandler = handler;
                let free_fn = host.free_fn;
                let f = move |j: JsonValue| {
                    let input = serde_json::to_string(&j).unwrap_or_else(|_| "null".to_string());
                    let c_in = CString::new(input).unwrap();
                    let out_ptr = unsafe { h(c_in.as_ptr()) };
                    if out_ptr.is_null() {
                        return JsonValue::Null;
                    }
                    let out = unsafe { CStr::from_ptr(out_ptr).to_string_lossy().into_owned() };
                    if let Some(free_fn) = free_fn {
                        unsafe { free_fn(out_ptr) };
                    } else {
                        // Fallback: attempt libc::free (may be unsafe across CRTs on Windows)
                        unsafe { libc::free(out_ptr as *mut libc::c_void) };
                    }
                    serde_json::from_str::<JsonValue>(&out).unwrap_or(JsonValue::Null)
                };
                let name_key = cname.clone();
                host.handlers.insert(name_key, Box::new(f));
            }
            let ctx_ptr = &mut host_ctx as *mut _ as *mut std::ffi::c_void;
            reg_fn(ctx_ptr, host_registrar);

            // Try optional typed registrar; if present, collect typed handlers.
            let mut typed_handlers: HashMap<String, CTypedHandler> = HashMap::new();
            if let Ok(tsym) = lib.get::<TypedRegisterFn>(b"mainstage_register_typed\0") {
                let typed_reg_fn = *tsym;
                #[allow(clippy::not_unsafe_ptr_arg_deref)]
                unsafe extern "C" fn host_typed_registrar(
                    ctx: *mut std::ffi::c_void,
                    name: *const c_char,
                    handler: CTypedHandler,
                ) {
                    let map = unsafe { &mut *(ctx as *mut HashMap<String, CTypedHandler>) };
                    let cname = if name.is_null() {
                        "".to_string()
                    } else {
                        unsafe { CStr::from_ptr(name).to_string_lossy().into_owned() }
                    };
                    map.insert(cname, handler);
                }
                let map_ptr = &mut typed_handlers as *mut _ as *mut std::ffi::c_void;
                typed_reg_fn(map_ptr, host_typed_registrar);
            }
            /// Try to guess the binary architecture from file headers (PE/ELF/Mach-O).
            fn guess_binary_arch(path: &Path) -> Result<String, String> {
                use std::io::Read;
                let mut f = std::fs::File::open(path).map_err(|e| format!("open failed: {}", e))?;
                let mut buf = [0u8; 64];
                let n = f
                    .read(&mut buf)
                    .map_err(|e| format!("read failed: {}", e))?;
                if n >= 4 && &buf[0..4] == b"\x7fELF" {
                    // ELF: e_ident[4] is class: 1=32,2=64. e_machine at offset 18 (little-endian)
                    if n > 18 {
                        let class = buf[4];
                        let emachine = u16::from_le_bytes([buf[18], buf[19]]);
                        match (class, emachine) {
                            (2, 62) => return Ok("x86_64".to_string()),
                            (1, 3) => return Ok("x86".to_string()),
                            (_, 183) => return Ok("aarch64".to_string()),
                            _ => return Ok(format!("elf-emu-{}", emachine)),
                        }
                    }
                }
                if n >= 2 && &buf[0..2] == b"MZ" {
                    // PE header: at offset 0x3c is e_lfanew (u32 LE)
                    let mut f2 =
                        std::fs::File::open(path).map_err(|e| format!("open2 failed: {}", e))?;
                    let mut hdr = [0u8; 64];
                    f2.read_exact(&mut hdr).ok();
                    let e_lfanew =
                        u32::from_le_bytes([hdr[0x3c], hdr[0x3d], hdr[0x3e], hdr[0x3f]]) as usize;
                    let mut pehdr = vec![0u8; 8];
                    f2.seek(std::io::SeekFrom::Start(e_lfanew as u64)).ok();
                    f2.read_exact(&mut pehdr).ok();
                    // machine is at offset e_lfanew + 4 (IMAGE_FILE_HEADER.Machine is u16)
                    let mut mh = [0u8; 2];
                    f2.read_exact(&mut mh).ok();
                    let machine = u16::from_le_bytes(mh);
                    match machine {
                        0x8664 => return Ok("x86_64".to_string()),
                        0x014c => return Ok("x86".to_string()),
                        0xAA64 => return Ok("aarch64".to_string()),
                        _ => return Ok(format!("pe-0x{:x}", machine)),
                    }
                }
                if n >= 4 {
                    let magic = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                    // Mach-O / Fat headers: 0xFEEDFACE, 0xFEEDFACF, 0xCEFAEDFE, 0xCFFAEDFE, 0xCAFEBABE
                    match magic {
                        0xFEEDFACF | 0xFEEDFACE | 0xCAFEBABE => {
                            // best effort: assume 64-bit for FEEDFACF
                            if magic == 0xFEEDFACF {
                                return Ok("x86_64".to_string());
                            } else {
                                return Ok("unknown-mach-o".to_string());
                            }
                        }
                        _ => {}
                    }
                }
                Err("unknown binary format".to_string())
            }

            Ok(InProcessPlugin {
                _lib: Arc::new(lib),
                name: cname,
                handlers: host_ctx.handlers,
                typed_handlers,
                free_fn: host_ctx.free_fn,
                free_value_fn,
            })
        }
    }

    /// Return the union of registered JSON and typed handler names.
    pub fn list_registered_functions(&self) -> Vec<String> {
        let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for k in self.handlers.keys() {
            names.insert(k.clone());
        }
        for k in self.typed_handlers.keys() {
            names.insert(k.clone());
        }
        let mut out: Vec<String> = names.into_iter().collect();
        out.sort();
        out
    }
}

#[async_trait]
impl Plugin for InProcessPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    async fn call(&self, func: &str, args: Vec<VmValue>) -> Result<VmValue, String> {
        // Prefer typed handler when available and args are convertible (prototype supports scalars/strings).
        if let Some(&th) = self.typed_handlers.get(func) {
            // Try to convert args; if any unsupported, fallback to JSON path.
            let mut cargs: Vec<CValue> = Vec::with_capacity(args.len());
            // Hold CStrings so their memory lives through the call
            let mut string_bufs: Vec<CString> = Vec::new();
            // Track heap allocations (arrays/objects) to free after call
            let mut alloc_ptrs: Vec<*mut libc::c_void> = Vec::new();

            fn str_view_from(s: &str, buf: &mut Vec<CString>) -> Result<CStrView, String> {
                let cs = CString::new(s).map_err(|_| "string contains NUL".to_string())?;
                let ptr = cs.as_ptr();
                let len = s.len();
                buf.push(cs);
                Ok(CStrView { ptr, len })
            }

            fn vm_to_cvalue(
                v: &VmValue,
                buf: &mut Vec<CString>,
                allocs: &mut Vec<*mut libc::c_void>,
            ) -> Result<CValue, String> {
                Ok(match v {
                    VmValue::Null => CValue {
                        tag: CTag::Null,
                        b: false,
                        i: 0,
                        f: 0.0,
                        s: CStrView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        arr: CArrayView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        obj: CObjectView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                    },
                    VmValue::Bool(b) => CValue {
                        tag: CTag::Bool,
                        b: *b,
                        i: 0,
                        f: 0.0,
                        s: CStrView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        arr: CArrayView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        obj: CObjectView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                    },
                    VmValue::Int(i) => CValue {
                        tag: CTag::Int,
                        b: false,
                        i: *i,
                        f: 0.0,
                        s: CStrView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        arr: CArrayView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        obj: CObjectView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                    },
                    VmValue::Float(fv) => CValue {
                        tag: CTag::Float,
                        b: false,
                        i: 0,
                        f: *fv,
                        s: CStrView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        arr: CArrayView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        obj: CObjectView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                    },
                    VmValue::Str(s) | VmValue::Symbol(s) => {
                        let sv = str_view_from(s, buf)?;
                        CValue {
                            tag: CTag::String,
                            b: false,
                            i: 0,
                            f: 0.0,
                            s: sv,
                            arr: CArrayView {
                                ptr: std::ptr::null(),
                                len: 0,
                            },
                            obj: CObjectView {
                                ptr: std::ptr::null(),
                                len: 0,
                            },
                        }
                    }
                    VmValue::Array(a) => {
                        let n = a.len();
                        if n == 0 {
                            CValue {
                                tag: CTag::Array,
                                b: false,
                                i: 0,
                                f: 0.0,
                                s: CStrView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                                arr: CArrayView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                                obj: CObjectView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                            }
                        } else {
                            let bytes = std::mem::size_of::<CValue>() * n;
                            let ptr = unsafe { libc::malloc(bytes) as *mut CValue };
                            if ptr.is_null() {
                                return Err("malloc failed for array".to_string());
                            }
                            allocs.push(ptr as *mut libc::c_void);
                            for (i, item) in a.iter().enumerate().take(n) {
                                let cv = vm_to_cvalue(item, buf, allocs)?;
                                unsafe {
                                    *ptr.add(i) = cv;
                                }
                            }
                            CValue {
                                tag: CTag::Array,
                                b: false,
                                i: 0,
                                f: 0.0,
                                s: CStrView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                                arr: CArrayView { ptr, len: n },
                                obj: CObjectView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                            }
                        }
                    }
                    VmValue::Object(m) => {
                        let n = m.len();
                        if n == 0 {
                            CValue {
                                tag: CTag::Object,
                                b: false,
                                i: 0,
                                f: 0.0,
                                s: CStrView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                                arr: CArrayView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                                obj: CObjectView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                            }
                        } else {
                            let bytes = std::mem::size_of::<CObjectEntry>() * n;
                            let ptr = unsafe { libc::malloc(bytes) as *mut CObjectEntry };
                            if ptr.is_null() {
                                return Err("malloc failed for object".to_string());
                            }
                            allocs.push(ptr as *mut libc::c_void);
                            for (i, (k, v)) in m.iter().enumerate() {
                                let ksv = str_view_from(k, buf)?;
                                let cv = vm_to_cvalue(v, buf, allocs)?;
                                unsafe {
                                    *ptr.add(i) = CObjectEntry {
                                        key: ksv,
                                        value: cv,
                                    };
                                }
                            }
                            CValue {
                                tag: CTag::Object,
                                b: false,
                                i: 0,
                                f: 0.0,
                                s: CStrView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                                arr: CArrayView {
                                    ptr: std::ptr::null(),
                                    len: 0,
                                },
                                obj: CObjectView { ptr, len: n },
                            }
                        }
                    }
                })
            }

            let mut convertible = true;
            for a in &args {
                match vm_to_cvalue(a, &mut string_bufs, &mut alloc_ptrs) {
                    Ok(cv) => cargs.push(cv),
                    Err(_) => {
                        convertible = false;
                        break;
                    }
                }
            }
            if convertible {
                let mut out: CValue = CValue {
                    tag: CTag::Null,
                    b: false,
                    i: 0,
                    f: 0.0,
                    s: CStrView {
                        ptr: std::ptr::null(),
                        len: 0,
                    },
                    arr: CArrayView {
                        ptr: std::ptr::null(),
                        len: 0,
                    },
                    obj: CObjectView {
                        ptr: std::ptr::null(),
                        len: 0,
                    },
                };
                let rc = unsafe { th(cargs.as_ptr(), cargs.len(), &mut out as *mut CValue) };
                if rc != 0 {
                    return Err(format!("typed handler returned error code {}", rc));
                }
                // Convert result back to VmValue; deep-free arrays/objects via plugin when available
                fn convert_value(val: &CValue) -> Result<VmValue, String> {
                    Ok(match val.tag {
                        CTag::Null => VmValue::Null,
                        CTag::Bool => VmValue::Bool(val.b),
                        CTag::Int => VmValue::Int(val.i),
                        CTag::Float => VmValue::Float(val.f),
                        CTag::String => {
                            if val.s.ptr.is_null() || val.s.len == 0 {
                                VmValue::Str(String::new())
                            } else {
                                let s = unsafe {
                                    CStr::from_ptr(val.s.ptr).to_string_lossy().into_owned()
                                };
                                VmValue::Str(s)
                            }
                        }
                        CTag::Array => {
                            if val.arr.ptr.is_null() || val.arr.len == 0 {
                                VmValue::Array(Vec::new())
                            } else {
                                let mut out = Vec::with_capacity(val.arr.len);
                                for i in 0..val.arr.len {
                                    let elem = unsafe { *val.arr.ptr.add(i) };
                                    out.push(convert_value(&elem)?);
                                }
                                VmValue::Array(out)
                            }
                        }
                        CTag::Object => {
                            if val.obj.ptr.is_null() || val.obj.len == 0 {
                                VmValue::Object(std::collections::HashMap::new())
                            } else {
                                let mut map = std::collections::HashMap::with_capacity(val.obj.len);
                                for i in 0..val.obj.len {
                                    let entry = unsafe { *val.obj.ptr.add(i) };
                                    let key = if entry.key.ptr.is_null() || entry.key.len == 0 {
                                        String::new()
                                    } else {
                                        unsafe {
                                            CStr::from_ptr(entry.key.ptr)
                                                .to_string_lossy()
                                                .into_owned()
                                        }
                                    };
                                    let v = convert_value(&entry.value)?;
                                    map.insert(key, v);
                                }
                                VmValue::Object(map)
                            }
                        }
                    })
                }
                let result = convert_value(&out)?;
                // Free memory: strings freed individually only for top-level String; arrays/objects deep-free via plugin
                match out.tag {
                    CTag::String => {
                        if !out.s.ptr.is_null()
                            && let Some(f) = self.free_fn
                        {
                            unsafe { f(out.s.ptr as *mut c_char) };
                        }
                    }
                    CTag::Array | CTag::Object => {
                        if let Some(fv) = self.free_value_fn {
                            unsafe { fv(&out as *const CValue) };
                        }
                    }
                    _ => {}
                }
                // Free argument allocations owned by core (arrays/objects)
                for p in alloc_ptrs {
                    unsafe { libc::free(p) };
                }
                return Ok(result);
            }
        }

        // JSON fallback path remains the default.
        let j = values_to_json_array(&args);
        let handler = self
            .handlers
            .get(func)
            .ok_or_else(|| format!("function not found: {}", func))?;
        let out_json: JsonValue = handler(j);
        let vm_val = json_to_value(&out_json);
        Ok(vm_val)
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::default()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
