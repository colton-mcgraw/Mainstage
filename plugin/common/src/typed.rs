use std::os::raw::{c_char, c_void};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CTag {
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
pub struct CStrView {
    pub ptr: *const c_char,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CArrayView {
    pub ptr: *const CValue,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CObjectEntry {
    pub key: CStrView,
    pub value: CValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CObjectView {
    pub ptr: *const CObjectEntry,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CValue {
    pub tag: CTag,
    pub b: bool,
    pub i: i64,
    pub f: f64,
    pub s: CStrView,
    pub arr: CArrayView,
    pub obj: CObjectView,
}

pub type CTypedHandler =
    unsafe extern "C" fn(args: *const CValue, argc: usize, out: *mut CValue) -> i32;
pub type CTypedRegistrar =
    unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char, handler: CTypedHandler);

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn cvalue_null(out: *mut CValue) {
    unsafe {
        (*out).tag = CTag::Null;
        (*out).b = false;
        (*out).i = 0;
        (*out).f = 0.0;
        (*out).s = CStrView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).arr = CArrayView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).obj = CObjectView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn cvalue_bool(out: *mut CValue, v: bool) {
    unsafe {
        (*out).tag = CTag::Bool;
        (*out).b = v;
        (*out).i = 0;
        (*out).f = 0.0;
        (*out).s = CStrView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).arr = CArrayView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).obj = CObjectView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn cvalue_int(out: *mut CValue, v: i64) {
    unsafe {
        (*out).tag = CTag::Int;
        (*out).i = v;
        (*out).b = false;
        (*out).f = 0.0;
        (*out).s = CStrView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).arr = CArrayView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).obj = CObjectView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn cvalue_float(out: *mut CValue, v: f64) {
    unsafe {
        (*out).tag = CTag::Float;
        (*out).f = v;
        (*out).b = false;
        (*out).i = 0;
        (*out).s = CStrView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).arr = CArrayView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).obj = CObjectView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn cvalue_string(out: *mut CValue, ptr: *const c_char, len: usize) {
    unsafe {
        (*out).tag = CTag::String;
        (*out).s = CStrView { ptr, len };
        (*out).b = false;
        (*out).i = 0;
        (*out).f = 0.0;
        (*out).arr = CArrayView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).obj = CObjectView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn cvalue_array(out: *mut CValue, ptr: *const CValue, len: usize) {
    unsafe {
        (*out).tag = CTag::Array;
        (*out).arr = CArrayView { ptr, len };
        (*out).obj = CObjectView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).b = false;
        (*out).i = 0;
        (*out).f = 0.0;
        (*out).s = CStrView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn cvalue_object(out: *mut CValue, ptr: *const CObjectEntry, len: usize) {
    unsafe {
        (*out).tag = CTag::Object;
        (*out).obj = CObjectView { ptr, len };
        (*out).arr = CArrayView {
            ptr: std::ptr::null(),
            len: 0,
        };
        (*out).b = false;
        (*out).i = 0;
        (*out).f = 0.0;
        (*out).s = CStrView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn deep_copy_value(src: &CValue) -> CValue {
    unsafe {
        match src.tag {
            CTag::Null => CValue {
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
            CTag::Bool => CValue {
                tag: CTag::Bool,
                b: src.b,
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
            CTag::Int => CValue {
                tag: CTag::Int,
                b: false,
                i: src.i,
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
            CTag::Float => CValue {
                tag: CTag::Float,
                b: false,
                i: 0,
                f: src.f,
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
            CTag::String => {
                if src.s.ptr.is_null() || src.s.len == 0 {
                    return CValue {
                        tag: CTag::String,
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
                }
                let bytes = src.s.len + 1;
                let ptr = libc::malloc(bytes) as *mut c_char;
                if ptr.is_null() {
                    return CValue {
                        tag: CTag::String,
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
                }
                std::ptr::copy_nonoverlapping(src.s.ptr as *const u8, ptr as *mut u8, src.s.len);
                *ptr.add(src.s.len) = 0;
                CValue {
                    tag: CTag::String,
                    b: false,
                    i: 0,
                    f: 0.0,
                    s: CStrView {
                        ptr,
                        len: src.s.len,
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
            }
            CTag::Array => {
                if src.arr.ptr.is_null() || src.arr.len == 0 {
                    return CValue {
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
                    };
                }
                let bytes = std::mem::size_of::<CValue>() * src.arr.len;
                let ptr = libc::malloc(bytes) as *mut CValue;
                if ptr.is_null() {
                    return CValue {
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
                    };
                }
                for i in 0..src.arr.len {
                    let cv = deep_copy_value(&*src.arr.ptr.add(i));
                    *ptr.add(i) = cv;
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
                    arr: CArrayView {
                        ptr,
                        len: src.arr.len,
                    },
                    obj: CObjectView {
                        ptr: std::ptr::null(),
                        len: 0,
                    },
                }
            }
            CTag::Object => {
                if src.obj.ptr.is_null() || src.obj.len == 0 {
                    return CValue {
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
                    };
                }
                let bytes = std::mem::size_of::<CObjectEntry>() * src.obj.len;
                let ptr = libc::malloc(bytes) as *mut CObjectEntry;
                if ptr.is_null() {
                    return CValue {
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
                    };
                }
                for i in 0..src.obj.len {
                    let e = &*src.obj.ptr.add(i);
                    let key = deep_copy_value(&CValue {
                        tag: CTag::String,
                        b: false,
                        i: 0,
                        f: 0.0,
                        s: e.key,
                        arr: CArrayView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                        obj: CObjectView {
                            ptr: std::ptr::null(),
                            len: 0,
                        },
                    });
                    let val = deep_copy_value(&e.value);
                    (*ptr.add(i)).key = key.s;
                    (*ptr.add(i)).value = val;
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
                    obj: CObjectView {
                        ptr,
                        len: src.obj.len,
                    },
                }
            }
        }
    }
}

/// # Safety
/// This function is unsafe because it operates on raw pointers and assumes that the input
/// memory is valid and was allocated in a compatible manner. The caller must ensure that
/// the `val` pointer is valid and points to a properly constructed `CValue`.
pub unsafe fn free_value(val: *const CValue) {
    unsafe {
        if val.is_null() {
            return;
        }
        let v = &*val;
        match v.tag {
            CTag::String => {
                if !v.s.ptr.is_null() {
                    libc::free(v.s.ptr as *mut c_void);
                }
            }
            CTag::Array => {
                if !v.arr.ptr.is_null() {
                    for i in 0..v.arr.len {
                        free_value(v.arr.ptr.add(i));
                    }
                    libc::free(v.arr.ptr as *mut c_void);
                }
            }
            CTag::Object => {
                if !v.obj.ptr.is_null() {
                    for i in 0..v.obj.len {
                        let e = &*v.obj.ptr.add(i);
                        if !e.key.ptr.is_null() {
                            libc::free(e.key.ptr as *mut c_void);
                        }
                        free_value(&e.value);
                    }
                    libc::free(v.obj.ptr as *mut c_void);
                }
            }
            _ => {}
        }
    }
}
