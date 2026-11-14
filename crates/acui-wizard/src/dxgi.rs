// SPDX-License-Identifier: GPL-3.0-only
//! This computer's DXGI adapters, read only, for the graphics card option ([`crate::gpu`]): listed
//! as the Runtime lists them (Runtime `crates/vision-ffi/src/adapters.rs`, module `system`),
//! `IDXGIFactory1::EnumAdapters1` in order with each adapter's `DXGI_ADAPTER_DESC1`, and the
//! kernel's adapter type and PCI location through `D3DKMTOpenAdapterFromLuid`,
//! `D3DKMTQueryAdapterInfo` and `D3DKMTCloseAdapter`; besides what the Runtime reads, the
//! adapter's description and its dedicated video memory, which the options page shows. No
//! Direct3D device is created and nothing is written.
//!
//! `dxgi.dll` and `gdi32.dll` are loaded from System32 only (`LOAD_LIBRARY_SEARCH_SYSTEM32`), as
//! the Runtime loads them: the wizard is often started from a download folder, where a library
//! of the same name must never be picked up. They are loaded on the first listing and stay loaded
//! for the process. DXGI is called through the same minimal vtable the Runtime calls it through,
//! so the wizard needs no further Windows bindings for it.
//!
//! On any other platform the listing fails loud.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::ptr::null_mut;
    use std::sync::OnceLock;

    use crate::gpu::{Adapter, AdapterType, Kernel, KernelCall, PciAddress};
    use crate::text::Said;

    /// The most adapters one listing reads (the Runtime's `MAX_DXGI_ADAPTERS`).
    const MAX_ADAPTERS: u32 = 64;
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;
    const DXGI_ERROR_NOT_FOUND: i32 = 0x887A_0002_u32 as i32;
    /// `KMTQUERYADAPTERINFOTYPE` values (`d3dkmthk.h`).
    const KMTQAITYPE_ADAPTERADDRESS: u32 = 6;
    const KMTQAITYPE_ADAPTERTYPE: u32 = 15;
    /// IUnknown 0-2, IDXGIObject 3-6, IDXGIFactory 7-11, IDXGIFactory1 12 `EnumAdapters1`.
    const FACTORY1_ENUM_ADAPTERS1: usize = 12;
    /// IUnknown 0-2, IDXGIObject 3-6, IDXGIAdapter 7-9, IDXGIAdapter1 10 `GetDesc1`.
    const ADAPTER1_GET_DESC1: usize = 10;
    const UNKNOWN_RELEASE: usize = 2;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    /// `GUID`; read by DXGI through its pointer only.
    #[repr(C)]
    struct Guid {
        _data1: u32,
        _data2: u16,
        _data3: u16,
        _data4: [u8; 8],
    }

    /// `IID_IDXGIFactory1` {770aae78-f26f-4dba-a829-253c83d1b387}.
    const IID_IDXGI_FACTORY1: Guid = Guid {
        _data1: 0x770a_ae78,
        _data2: 0xf26f,
        _data3: 0x4dba,
        _data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
    };

    /// `LUID`; handed to the kernel as it came from DXGI.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Luid {
        _low_part: u32,
        _high_part: i32,
    }

    /// `DXGI_ADAPTER_DESC1`; the system memory sizes are not read.
    #[repr(C)]
    struct AdapterDesc1 {
        description: [u16; 128],
        vendor_id: u32,
        device_id: u32,
        sub_sys_id: u32,
        revision: u32,
        dedicated_video_memory: usize,
        _dedicated_system_memory: usize,
        _shared_system_memory: usize,
        adapter_luid: Luid,
        flags: u32,
    }

    /// `D3DKMT_OPENADAPTERFROMLUID`: the LUID in, the adapter handle out.
    #[repr(C)]
    struct OpenAdapterFromLuid {
        _adapter_luid: Luid,
        adapter: u32,
    }

    /// `D3DKMT_QUERYADAPTERINFO`; read by the kernel through its pointer only.
    #[repr(C)]
    struct QueryAdapterInfo {
        _adapter: u32,
        _query_type: u32,
        _private_driver_data: *mut c_void,
        _private_driver_data_size: u32,
    }

    /// `D3DKMT_CLOSEADAPTER`; read by the kernel through its pointer only.
    #[repr(C)]
    struct CloseAdapter {
        _adapter: u32,
    }

    /// `D3DKMT_ADAPTERADDRESS`.
    #[repr(C)]
    #[derive(Default)]
    struct AdapterAddress {
        bus_number: u32,
        device_number: u32,
        function_number: u32,
    }

    type CreateFactory1 = unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    type EnumAdapters1 = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    type GetDesc1 = unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> i32;
    type OpenFromLuid = unsafe extern "system" fn(*mut OpenAdapterFromLuid) -> i32;
    type QueryInfo = unsafe extern "system" fn(*const QueryAdapterInfo) -> i32;
    type Close = unsafe extern "system" fn(*const CloseAdapter) -> i32;

    #[derive(Clone, Copy)]
    struct Api {
        create_factory: CreateFactory1,
        open_adapter: OpenFromLuid,
        query_adapter: QueryInfo,
        close_adapter: Close,
    }

    /// The four exports, found on the first listing; the libraries are never freed.
    static API: OnceLock<Api> = OnceLock::new();

    fn failed(detail: impl std::fmt::Display) -> Said {
        Said::new("读不出这台电脑的显卡", "the graphics cards of this computer cannot be read").with(detail)
    }

    fn hresult(call: &str, status: i32) -> Said {
        failed(format!("{call} failed with HRESULT 0x{:08x}", status as u32))
    }

    fn system32(name: &str) -> Result<*mut c_void, Said> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a system library loaded from System32 only; `wide` is NUL-terminated UTF-16
        // that outlives the call, and the reserved handle is null as documented.
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            return Err(failed(format!("{name} did not load from System32: {}", io::Error::last_os_error())));
        }
        Ok(module)
    }

    /// The export `symbol` (NUL-terminated) of `module` as a function of type `F`.
    ///
    /// # Safety
    /// `F` is a function pointer type, the export's documented signature.
    unsafe fn export<F: Copy>(module: *mut c_void, library: &str, symbol: &[u8]) -> Result<F, Said> {
        // SAFETY: `module` is a loaded library and `symbol` a NUL-terminated name.
        let address = unsafe { GetProcAddress(module, symbol.as_ptr()) };
        if address.is_null() {
            let name = String::from_utf8_lossy(symbol.strip_suffix(b"\0").unwrap_or(symbol)).into_owned();
            return Err(failed(format!("{library} lacks {name}: {}", io::Error::last_os_error())));
        }
        // SAFETY: a function pointer and a data pointer have the same size on Windows; the
        // caller names the export's signature.
        Ok(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&address) })
    }

    fn api() -> Result<Api, Said> {
        if let Some(api) = API.get() {
            return Ok(*api);
        }
        let dxgi = system32("dxgi.dll")?;
        let gdi32 = system32("gdi32.dll")?;
        // SAFETY: each type is the export's documented signature; neither library is ever
        // freed, so the pointers stay valid for the process.
        let api = unsafe {
            Api {
                create_factory: export(dxgi, "dxgi.dll", b"CreateDXGIFactory1\0")?,
                open_adapter: export(gdi32, "gdi32.dll", b"D3DKMTOpenAdapterFromLuid\0")?,
                query_adapter: export(gdi32, "gdi32.dll", b"D3DKMTQueryAdapterInfo\0")?,
                close_adapter: export(gdi32, "gdi32.dll", b"D3DKMTCloseAdapter\0")?,
            }
        };
        Ok(*API.get_or_init(|| api))
    }

    /// One COM object this module owns a reference to.
    struct Com(*mut c_void);

    impl Com {
        /// The function in slot `index` of the object's vtable.
        ///
        /// # Safety
        /// `index` is a slot of the object's vtable whose function has type `F`.
        unsafe fn method<F: Copy>(&self, index: usize) -> F {
            // SAFETY: a COM object starts with its vtable pointer; the caller names a valid slot.
            unsafe {
                let vtable = *(self.0 as *const *const *const c_void);
                let entry: *const c_void = *vtable.add(index);
                std::mem::transmute_copy::<*const c_void, F>(&entry)
            }
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: slot 2 of every COM vtable is IUnknown::Release; this value owns one
            // reference, released once.
            unsafe {
                let release: Release = self.method(UNKNOWN_RELEASE);
                release(self.0);
            }
        }
    }

    fn description(units: &[u16]) -> String {
        let end = units.iter().position(|unit| *unit == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end]).trim().to_owned()
    }

    /// Every DXGI adapter, in `EnumAdapters1` order. A listing that cannot be read fails; an
    /// adapter whose kernel type cannot be read is listed with the failure.
    pub fn adapters() -> Result<Vec<Adapter>, Said> {
        let api = api()?;
        let mut factory: *mut c_void = null_mut();
        // SAFETY: a valid IID and a writable out pointer.
        let status = unsafe { (api.create_factory)(&IID_IDXGI_FACTORY1, &mut factory) };
        if status < 0 || factory.is_null() {
            return Err(hresult("CreateDXGIFactory1", status));
        }
        let factory = Com(factory);
        let mut adapters = Vec::new();
        for index in 0..=MAX_ADAPTERS {
            let mut adapter: *mut c_void = null_mut();
            // SAFETY: slot 12 of IDXGIFactory1 is EnumAdapters1; the out pointer is writable.
            let status = unsafe {
                let enum_adapters: EnumAdapters1 = factory.method(FACTORY1_ENUM_ADAPTERS1);
                enum_adapters(factory.0, index, &mut adapter)
            };
            if status == DXGI_ERROR_NOT_FOUND {
                return Ok(adapters);
            }
            if status < 0 || adapter.is_null() {
                return Err(hresult("IDXGIFactory1::EnumAdapters1", status));
            }
            let adapter = Com(adapter);
            if index == MAX_ADAPTERS {
                return Err(failed(format!("DXGI lists more than {MAX_ADAPTERS} adapters")));
            }
            let mut desc = AdapterDesc1 {
                description: [0; 128],
                vendor_id: 0,
                device_id: 0,
                sub_sys_id: 0,
                revision: 0,
                dedicated_video_memory: 0,
                _dedicated_system_memory: 0,
                _shared_system_memory: 0,
                adapter_luid: Luid { _low_part: 0, _high_part: 0 },
                flags: 0,
            };
            // SAFETY: slot 10 of IDXGIAdapter1 is GetDesc1; `desc` is writable.
            let status = unsafe {
                let get_desc: GetDesc1 = adapter.method(ADAPTER1_GET_DESC1);
                get_desc(adapter.0, &mut desc)
            };
            if status < 0 {
                return Err(hresult("IDXGIAdapter1::GetDesc1", status));
            }
            adapters.push(Adapter {
                dxgi_index: index,
                name: description(&desc.description),
                vendor_id: desc.vendor_id,
                device_id: desc.device_id,
                subsys_id: desc.sub_sys_id,
                revision: desc.revision,
                dedicated_video_memory: desc.dedicated_video_memory as u64,
                dxgi_flags: desc.flags,
                kernel: kernel(api, desc.adapter_luid),
            });
        }
        Err(failed(format!("DXGI lists more than {MAX_ADAPTERS} adapters")))
    }

    /// The kernel's adapter type and PCI location of the adapter with `luid`, or the call that
    /// failed: the Runtime's own reading (`kernel_reading`).
    fn kernel(api: Api, luid: Luid) -> Kernel {
        let mut open = OpenAdapterFromLuid { _adapter_luid: luid, adapter: 0 };
        // SAFETY: a valid in/out structure for the call's duration.
        let status = unsafe { (api.open_adapter)(&mut open) };
        if status < 0 {
            return Kernel::Failed { call: KernelCall::OpenAdapterFromLuid, ntstatus: status };
        }
        let mut adapter_type = 0u32;
        let type_status = query(api, open.adapter, KMTQAITYPE_ADAPTERTYPE, &mut adapter_type);
        let mut address = AdapterAddress::default();
        let address_status = query(api, open.adapter, KMTQAITYPE_ADAPTERADDRESS, &mut address);
        let close = CloseAdapter { _adapter: open.adapter };
        // SAFETY: the handle came from D3DKMTOpenAdapterFromLuid and is closed once.
        let close_status = unsafe { (api.close_adapter)(&close) };
        if type_status < 0 {
            return Kernel::Failed { call: KernelCall::QueryAdapterType, ntstatus: type_status };
        }
        if close_status < 0 {
            return Kernel::Failed { call: KernelCall::CloseAdapter, ntstatus: close_status };
        }
        Kernel::Read {
            adapter_type: AdapterType(adapter_type),
            pci_address: (address_status >= 0).then_some(PciAddress {
                bus: address.bus_number,
                device: address.device_number,
                function: address.function_number,
            }),
        }
    }

    /// One `D3DKMTQueryAdapterInfo` into `data` (a `u32` or a `D3DKMT_ADAPTERADDRESS`); its
    /// NTSTATUS.
    fn query<T>(api: Api, adapter: u32, query_type: u32, data: &mut T) -> i32 {
        let info = QueryAdapterInfo {
            _adapter: adapter,
            _query_type: query_type,
            _private_driver_data: (data as *mut T).cast(),
            _private_driver_data_size: std::mem::size_of::<T>() as u32,
        };
        // SAFETY: `data` is writable for its whole size for the call's duration.
        unsafe { (api.query_adapter)(&info) }
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::gpu::Adapter;
    use crate::text::Said;

    pub fn adapters() -> Result<Vec<Adapter>, Said> {
        Err(Said::new("读不出这台电脑的显卡", "the graphics cards of this computer cannot be read")
            .with_said(&Said::new("只在 Windows 上可用", "available on Windows only")))
    }
}

pub use imp::adapters;
