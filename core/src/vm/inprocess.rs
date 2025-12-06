//! file: core/src/vm/inprocess.rs
//! description: in-process dynamic library plugin adapter using `libloading`.
//!
//! This adapter loads a shared library at runtime and resolves a Rust-friendly
//! registration symbol (`mainstage_register`) so the plugin can register
//! handlers directly without a JSON bridge.

use std::io::Seek;
use std::path::Path;
use std::sync::Arc;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::vm::plugin::{Plugin, PluginMetadata};
use crate::vm::value::{Value as VmValue, json_to_value, values_to_json_array};
use async_trait::async_trait;
use libloading::{Library, Symbol};
use serde_json::Value as JsonValue;

// C ABI types: plugin provides JSON handler functions as C string in/out.
type CJsonHandler = unsafe extern "C" fn(input_json: *const c_char) -> *mut c_char;
type CFreeFn = unsafe extern "C" fn(ptr: *mut c_char);

// Core provides a C ABI registrar callback: plugin calls this per function.
type CRegistrar = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CJsonHandler);

// Symbol signature the plugin must export.
type RegisterFn = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, registrar: CRegistrar);

pub struct InProcessPlugin {
    _lib: Arc<Library>,
    name: String,
    handlers: HashMap<String, Box<dyn Fn(JsonValue) -> JsonValue + Send + Sync>>,
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
            let cname = path.file_stem().and_then(|s| s.to_str()).unwrap_or("inproc").to_string();

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
            // Context is a raw pointer to our handlers map.
            #[allow(clippy::not_unsafe_ptr_arg_deref)]
            unsafe extern "C" fn host_registrar(ctx: *mut std::ffi::c_void, name: *const c_char, handler: CJsonHandler) {
                // Safety: ctx is a &mut HostCtx passed from Rust.
                let host = unsafe { &mut *(ctx as *mut HostCtx) };
                let cname = if name.is_null() { "".to_string() } else {
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
            })
        }
    }
}

#[async_trait]
impl Plugin for InProcessPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    async fn call(&self, func: &str, args: Vec<VmValue>) -> Result<VmValue, String> {
        // Serialize args to JSON array and invoke registered handler.
        let j = values_to_json_array(&args);
        let handler = self.handlers.get(func).ok_or_else(|| format!("function not found: {}", func))?;
        let out_json: JsonValue = handler(j);
        let vm_val = json_to_value(&out_json);
        Ok(vm_val)
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::default()
    }
}
