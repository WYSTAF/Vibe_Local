//! Real cross-platform hardware detection: system RAM, CPU feature flags,
//! GPU/VRAM (Vulkan / CUDA / Metal / CPU) and portable USB storage location.
//!
//! No fake fallback numbers: every value is read from the OS. When a sensor
//! or API is unavailable the field is `None` / empty, never invented.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[cfg(any(target_os = "linux", target_os = "android"))]
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
use crate::util::run_with_timeout;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub vram_mb: Option<u64>,
    /// Best backend this GPU supports on this machine: "cuda", "vulkan", "metal".
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortableInfo {
    pub available: bool,
    pub root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HardwareReport {
    pub os: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub cpu_flags: Vec<String>,
    pub ram_total_mb: u64,
    pub ram_free_mb: u64,
    pub gpus: Vec<GpuInfo>,
    pub preferred_backend: String,
    pub tier: String,
    pub portable: PortableInfo,
}

pub struct HardwareProbe;

impl HardwareProbe {
    pub fn scan() -> HardwareReport {
        let (ram_total_mb, ram_free_mb) = system_ram_mb();
        let cpu_flags = cpu_feature_flags();
        let gpus = detect_gpus();
        let preferred_backend = preferred_backend(&gpus);
        let mut report = HardwareReport {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cpu_model: cpu_model(),
            cpu_cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            cpu_flags,
            ram_total_mb,
            ram_free_mb,
            gpus,
            preferred_backend,
            tier: String::new(),
            portable: locate_portable(),
        };
        report.tier = crate::recommender::classify_tier(&report).id().to_string();
        report
    }
}

// ---------------------------------------------------------------------------
// RAM
// ---------------------------------------------------------------------------

pub fn system_ram_mb() -> (u64, u64) {
    #[cfg(target_os = "windows")]
    {
        windows_ram_mb()
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        parse_meminfo(&text)
    }
    #[cfg(target_os = "macos")]
    {
        macos_ram_mb()
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    )))]
    {
        (0, 0)
    }
}

/// Parse Linux `/proc/meminfo` content (kB units) into (total_mb, available_mb).
pub fn parse_meminfo(text: &str) -> (u64, u64) {
    let mut total = 0u64;
    let mut available = 0u64;
    let mut free = 0u64;
    let mut buffers = 0u64;
    let mut cached = 0u64;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("");
        let val: u64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match key {
            "MemTotal:" => total = val,
            "MemAvailable:" => available = val,
            "MemFree:" => free = val,
            "Buffers:" => buffers = val,
            "Cached:" => cached = val,
            _ => {}
        }
    }
    if available == 0 {
        available = free + buffers + cached;
    }
    (total / 1024, available / 1024)
}

#[cfg(target_os = "windows")]
fn windows_ram_mb() -> (u64, u64) {
    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    unsafe {
        let mut status: MemoryStatusEx = std::mem::zeroed();
        status.dw_length = std::mem::size_of::<MemoryStatusEx>() as u32;
        if GlobalMemoryStatusEx(&mut status) == 0 {
            return (0, 0);
        }
        (
            status.ull_total_phys / (1024 * 1024),
            status.ull_avail_phys / (1024 * 1024),
        )
    }
}

#[cfg(target_os = "macos")]
fn macos_ram_mb() -> (u64, u64) {
    let mut total: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let name = b"hw.memsize\0";
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast::<libc::c_char>(),
            (&mut total as *mut u64).cast::<libc::c_void>(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return (0, 0);
    }
    // Free memory from `vm_stat` (pages). Real read, no estimation.
    let mut free_mb = 0u64;
    if let Some(out) = run_with_timeout("vm_stat", &[], Duration::from_secs(5)) {
        let mut page_size = 4096u64;
        if let Some(first) = out.lines().next() {
            if let Some(n) = first
                .split("page size of ")
                .nth(1)
                .and_then(|s| s.split(' ').next())
                .and_then(|s| s.parse::<u64>().ok())
            {
                page_size = n;
            }
        }
        let mut pages = 0u64;
        for line in out.lines() {
            for key in ["Pages free:", "Pages inactive:", "Pages speculative:"] {
                if let Some(v) = line.strip_prefix(key) {
                    pages += v.trim().trim_end_matches('.').parse::<u64>().unwrap_or(0);
                }
            }
        }
        free_mb = pages * page_size / (1024 * 1024);
    }
    (total / (1024 * 1024), free_mb)
}

// ---------------------------------------------------------------------------
// CPU
// ---------------------------------------------------------------------------

pub fn cpu_feature_flags() -> Vec<String> {
    let mut flags = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        for (name, present) in [
            ("sse4.2", std::is_x86_feature_detected!("sse4.2")),
            ("avx", std::is_x86_feature_detected!("avx")),
            ("avx2", std::is_x86_feature_detected!("avx2")),
            ("fma", std::is_x86_feature_detected!("fma")),
            ("avx512f", std::is_x86_feature_detected!("avx512f")),
            ("avx512bw", std::is_x86_feature_detected!("avx512bw")),
            ("avx512vl", std::is_x86_feature_detected!("avx512vl")),
        ] {
            if present {
                flags.push(name.to_string());
            }
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        for (name, present) in [
            ("neon", std::arch::is_aarch64_feature_detected!("neon")),
            ("asimd", std::arch::is_aarch64_feature_detected!("asimd")),
            (
                "dotprod",
                std::arch::is_aarch64_feature_detected!("dotprod"),
            ),
            ("i8mm", std::arch::is_aarch64_feature_detected!("i8mm")),
            ("fp16", std::arch::is_aarch64_feature_detected!("fp16")),
        ] {
            if present {
                flags.push(name.to_string());
            }
        }
    }
    flags
}

pub fn cpu_model() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Some(name) = windows_registry_string(
            "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0",
            "ProcessorNameString",
        ) {
            return name.trim().to_string();
        }
        String::new()
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let text = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        for line in text.lines() {
            for key in ["model name", "Hardware", "Processor"] {
                if let Some(v) = line.strip_prefix(key) {
                    if let Some(v) = v.trim_start().strip_prefix(':') {
                        return v.trim().to_string();
                    }
                }
            }
        }
        String::new()
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 256];
        let mut size = buf.len();
        let name = b"machdep.cpu.brand_string\0";
        let rc = unsafe {
            libc::sysctlbyname(
                name.as_ptr().cast::<libc::c_char>(),
                buf.as_mut_ptr().cast::<libc::c_void>(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 {
            String::from_utf8_lossy(&buf[..size.saturating_sub(1)])
                .trim()
                .to_string()
        } else {
            String::new()
        }
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    )))]
    {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// GPU / VRAM
// ---------------------------------------------------------------------------

pub fn detect_gpus() -> Vec<GpuInfo> {
    #[cfg(target_os = "windows")]
    {
        windows_gpus()
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        linux_gpus()
    }
    #[cfg(target_os = "macos")]
    {
        macos_gpus()
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    )))]
    {
        Vec::new()
    }
}

pub fn preferred_backend(gpus: &[GpuInfo]) -> String {
    // Prefer the backend of the strongest (largest-VRAM) real GPU; fall back
    // to cuda > vulkan > metal when VRAM is unknown.
    let best = gpus
        .iter()
        .filter(|g| g.backend != "cpu")
        .max_by_key(|g| g.vram_mb.unwrap_or(0));
    if let Some(g) = best
        && g.vram_mb.unwrap_or(0) > 0
    {
        return g.backend.clone();
    }
    if gpus.iter().any(|g| g.backend == "cuda") {
        return "cuda".into();
    }
    if gpus.iter().any(|g| g.backend == "vulkan") {
        return "vulkan".into();
    }
    if gpus.iter().any(|g| g.backend == "metal") {
        return "metal".into();
    }
    "cpu".into()
}

#[cfg(target_os = "windows")]
fn windows_gpus() -> Vec<GpuInfo> {
    let has_vulkan = windows_lib_present("vulkan-1.dll");
    let has_cuda = windows_lib_present("nvcuda.dll");
    let mut gpus = Vec::new();
    // Accurate VRAM: registry HardwareInformation.qwMemorySize (QWORD) per
    // display adapter class subkey. Win32_VideoController.AdapterRAM is 32-bit
    // and truncates >=4GB cards, so it is deliberately not used.
    let class_key =
        "SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}";
    for sub in windows_registry_subkeys(class_key) {
        let path = format!("{class_key}\\{sub}");
        let Some(name) = windows_registry_string(&path, "DriverDesc") else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let lower = name.to_lowercase();
        // Skip virtual/remote display adapters — they are not inference GPUs.
        if [
            "spacedesk",
            "microsoft basic",
            "remote",
            "virtual",
            "parsec",
            "idd",
        ]
        .iter()
        .any(|k| lower.contains(k))
        {
            continue;
        }
        let vram_mb = windows_registry_qword(&path, "HardwareInformation.qwMemorySize")
            .or_else(|| windows_registry_binary_qword(&path, "HardwareInformation.MemorySize"))
            .map(|bytes| bytes / (1024 * 1024));
        let vendor = if lower.contains("nvidia") {
            "nvidia"
        } else if lower.contains("amd") || lower.contains("radeon") {
            "amd"
        } else if lower.contains("intel") {
            "intel"
        } else {
            "unknown"
        };
        let backend = if vendor == "nvidia" && has_cuda {
            "cuda"
        } else if has_vulkan {
            "vulkan"
        } else {
            "cpu"
        };
        gpus.push(GpuInfo {
            name,
            vendor: vendor.into(),
            vram_mb,
            backend: backend.into(),
        });
    }
    gpus
}

#[cfg(target_os = "windows")]
fn windows_lib_present(name: &str) -> bool {
    use std::os::windows::ffi::OsStrExt;
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> isize;
        fn FreeLibrary(module: isize) -> i32;
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = LoadLibraryW(wide.as_ptr());
        if handle == 0 {
            return false;
        }
        FreeLibrary(handle);
    }
    true
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn linux_gpus() -> Vec<GpuInfo> {
    let has_vulkan = unix_lib_present("libvulkan.so.1");
    let has_cuda = unix_lib_present("libcuda.so.1");
    let mut gpus = Vec::new();
    let drm = Path::new("/sys/class/drm");
    if let Ok(entries) = std::fs::read_dir(drm) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            // Only primary card nodes (card0, card1...), not connectors.
            if !fname.starts_with("card") || fname.contains('-') {
                continue;
            }
            let device = entry.path().join("device");
            let vendor_id = std::fs::read_to_string(device.join("vendor")).unwrap_or_default();
            let vendor_id = vendor_id.trim().to_string();
            if vendor_id.is_empty() {
                continue;
            }
            let device_id = std::fs::read_to_string(device.join("device"))
                .unwrap_or_default()
                .trim()
                .to_string();
            let vendor = match vendor_id.as_str() {
                "0x10de" => "nvidia",
                "0x1002" => "amd",
                "0x8086" => "intel",
                _ => "unknown",
            };
            let mut vram_mb = std::fs::read_to_string(device.join("mem_info_vram_total"))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|b| b / (1024 * 1024));
            if vram_mb.is_none() && vendor == "nvidia" {
                vram_mb = nvidia_smi_vram_mb();
            }
            let name = gpu_name_from_lspci(&vendor_id, &device_id)
                .unwrap_or_else(|| format!("{vendor} GPU [{vendor_id}:{device_id}]"));
            let backend = if vendor == "nvidia" && has_cuda {
                "cuda"
            } else if has_vulkan {
                "vulkan"
            } else {
                "cpu"
            };
            gpus.push(GpuInfo {
                name,
                vendor: vendor.into(),
                vram_mb,
                backend: backend.into(),
            });
        }
    }
    // Android SoC GPUs (Adreno / Mali) are not exposed through /sys/class/drm.
    if gpus.is_empty() {
        for path in [
            "/sys/class/kgsl/kgsl-3d0/gpu_model",
            "/sys/class/misc/mali0/device/gpuinfo",
        ] {
            if let Ok(name) = std::fs::read_to_string(path) {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    let backend = if has_vulkan { "vulkan" } else { "cpu" };
                    gpus.push(GpuInfo {
                        name,
                        vendor: "qualcomm/arm".into(),
                        vram_mb: None, // shared UMA memory on SoCs
                        backend: backend.into(),
                    });
                    break;
                }
            }
        }
    }
    gpus
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn unix_lib_present(name: &str) -> bool {
    let cname = std::ffi::CString::new(name).unwrap();
    unsafe {
        let handle = libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if handle.is_null() {
            return false;
        }
        libc::dlclose(handle);
    }
    true
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn gpu_name_from_lspci(vendor: &str, device: &str) -> Option<String> {
    let out = run_with_timeout("lspci", &["-nn"], Duration::from_secs(5))?;
    let needle = format!(
        "[{}:{}]",
        vendor.trim_start_matches("0x"),
        device.trim_start_matches("0x")
    );
    for line in out.lines() {
        if (line.contains("VGA") || line.contains("3D") || line.contains("Display"))
            && line.contains(&needle)
        {
            return Some(
                line.split_once(": ")
                    .map(|x| x.1)
                    .unwrap_or(line)
                    .trim()
                    .to_string(),
            );
        }
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn nvidia_smi_vram_mb() -> Option<u64> {
    let out = run_with_timeout(
        "nvidia-smi",
        &["--query-gpu=memory.total", "--format=csv,noheader,nounits"],
        Duration::from_secs(5),
    )?;
    out.lines().next()?.trim().parse::<u64>().ok()
}

#[cfg(target_os = "macos")]
fn macos_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    if let Some(out) = run_with_timeout(
        "system_profiler",
        &["SPDisplaysDataType", "-json"],
        Duration::from_secs(15),
    ) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&out) {
            if let Some(items) = json.get("SPDisplaysDataType").and_then(|v| v.as_array()) {
                for item in items {
                    let name = item
                        .get("sppci_model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Apple GPU")
                        .to_string();
                    let vram_mb = item
                        .get("spdisplays_vram")
                        .and_then(|v| v.as_str())
                        .and_then(parse_vram_string);
                    gpus.push(GpuInfo {
                        name,
                        vendor: "apple".into(),
                        vram_mb,
                        backend: "metal".into(),
                    });
                }
            }
        }
    }
    if gpus.is_empty() {
        // Metal is always available on supported macOS; name via sysctl.
        gpus.push(GpuInfo {
            name: cpu_model(),
            vendor: "apple".into(),
            vram_mb: None, // unified memory
            backend: "metal".into(),
        });
    }
    gpus
}

#[cfg(target_os = "macos")]
fn parse_vram_string(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, unit) = s.split_once(' ')?;
    let n: f64 = num.parse().ok()?;
    match unit.to_lowercase().as_str() {
        "gb" => Some((n * 1024.0) as u64),
        "mb" => Some(n as u64),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Windows registry helpers (advapi32)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod registry {
    use std::os::windows::ffi::OsStrExt;

    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const KEY_READ: u32 = 0x20019;
    const ERROR_SUCCESS: i32 = 0;

    unsafe extern "system" {
        fn RegOpenKeyExW(
            key: isize,
            sub: *const u16,
            options: u32,
            sam: u32,
            out: *mut isize,
        ) -> i32;
        fn RegQueryValueExW(
            key: isize,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegEnumKeyExW(
            key: isize,
            index: u32,
            name: *mut u16,
            name_len: *mut u32,
            reserved: *mut u32,
            class: *mut u16,
            class_len: *mut u32,
            last_write: *mut u64,
        ) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn open(subkey: &str) -> Option<isize> {
        let w = wide(subkey);
        let mut handle: isize = 0;
        let rc = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, w.as_ptr(), 0, KEY_READ, &mut handle) };
        if rc == ERROR_SUCCESS {
            Some(handle)
        } else {
            None
        }
    }

    pub fn query_string(subkey: &str, value: &str) -> Option<String> {
        let handle = open(subkey)?;
        let w = wide(value);
        let mut kind = 0u32;
        let mut len = 0u32;
        let rc = unsafe {
            RegQueryValueExW(
                handle,
                w.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if rc != ERROR_SUCCESS || len == 0 || len > 64 * 1024 {
            unsafe { RegCloseKey(handle) };
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let rc = unsafe {
            RegQueryValueExW(
                handle,
                w.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                buf.as_mut_ptr(),
                &mut len,
            )
        };
        unsafe { RegCloseKey(handle) };
        if rc != ERROR_SUCCESS {
            return None;
        }
        // REG_SZ / REG_EXPAND_SZ are UTF-16LE.
        let units: Vec<u16> = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16(&units).ok()
    }

    pub fn query_qword(subkey: &str, value: &str) -> Option<u64> {
        let handle = open(subkey)?;
        let w = wide(value);
        let mut kind = 0u32;
        let mut data = [0u8; 8];
        // cbData must carry the buffer capacity on input.
        let mut len = data.len() as u32;
        let rc = unsafe {
            RegQueryValueExW(
                handle,
                w.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                data.as_mut_ptr(),
                &mut len,
            )
        };
        unsafe { RegCloseKey(handle) };
        if rc != ERROR_SUCCESS || len != 8 {
            return None;
        }
        Some(u64::from_le_bytes(data))
    }

    pub fn subkeys(subkey: &str) -> Vec<String> {
        let Some(handle) = open(subkey) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in 0..64u32 {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    handle,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..name_len as usize]));
        }
        unsafe { RegCloseKey(handle) };
        out
    }
}

#[cfg(target_os = "windows")]
fn windows_registry_string(subkey: &str, value: &str) -> Option<String> {
    registry::query_string(subkey, value)
}

#[cfg(target_os = "windows")]
fn windows_registry_qword(subkey: &str, value: &str) -> Option<u64> {
    registry::query_qword(subkey, value)
}

#[cfg(target_os = "windows")]
fn windows_registry_binary_qword(subkey: &str, value: &str) -> Option<u64> {
    // HardwareInformation.MemorySize is REG_BINARY holding a LE u64.
    registry::query_qword(subkey, value)
}

#[cfg(target_os = "windows")]
fn windows_registry_subkeys(subkey: &str) -> Vec<String> {
    registry::subkeys(subkey)
}

// ---------------------------------------------------------------------------
// Portable USB storage
// ---------------------------------------------------------------------------

/// Locate portable (removable) storage and return its `portable_data` root.
/// Honors `VIBE_PORTABLE_ROOT` override on every platform.
pub fn locate_portable() -> PortableInfo {
    if let Ok(root) = std::env::var("VIBE_PORTABLE_ROOT") {
        let p = PathBuf::from(root);
        if p.is_dir() {
            return PortableInfo {
                available: true,
                root: Some(p.join("portable_data").to_string_lossy().into_owned()),
            };
        }
    }
    let candidates = removable_roots();
    for root in candidates {
        let data = root.join("portable_data");
        if data.is_dir() || std::fs::create_dir_all(&data).is_ok() {
            return PortableInfo {
                available: true,
                root: Some(data.to_string_lossy().into_owned()),
            };
        }
    }
    PortableInfo {
        available: false,
        root: None,
    }
}

fn removable_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        unsafe extern "system" {
            fn GetLogicalDrives() -> u32;
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        const DRIVE_REMOVABLE: u32 = 2;
        let mut roots = Vec::new();
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = char::from(b'A' + i as u8);
            let root = format!("{letter}:\\");
            let wide: Vec<u16> = std::ffi::OsStr::new(&root)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            if unsafe { GetDriveTypeW(wide.as_ptr()) } == DRIVE_REMOVABLE {
                roots.push(PathBuf::from(&root));
            }
        }
        roots
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut roots = Vec::new();
        let user = std::env::var("USER").unwrap_or_default();
        let bases = [
            format!("/media/{user}"),
            "/media".to_string(),
            format!("/run/media/{user}"),
            "/mnt".to_string(),
            "/storage".to_string(),
        ];
        for base in bases {
            if let Ok(entries) = std::fs::read_dir(&base) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() && is_removable_mount(&p) {
                        roots.push(p);
                    }
                }
            }
        }
        roots
    }
    #[cfg(target_os = "macos")]
    {
        let mut roots = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/Volumes") {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() && p.file_name().map(|n| n != "Macintosh HD").unwrap_or(true) {
                    roots.push(p);
                }
            }
        }
        roots
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    )))]
    {
        Vec::new()
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn is_removable_mount(path: &Path) -> bool {
    // A mount point listed in /proc/mounts whose backing device is not the
    // root filesystem device is treated as removable/external storage.
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let target = path.to_string_lossy();
    for line in mounts.lines() {
        let mut it = line.split_whitespace();
        let dev = it.next().unwrap_or("");
        let mnt = it.next().unwrap_or("");
        if mnt == target
            && (dev.starts_with("/dev/sd")
                || dev.starts_with("/dev/mmcblk")
                || dev.starts_with("/dev/nvme")
                || dev.contains("usb"))
        {
            return true;
        }
    }
    // /storage/* on Android is external/OTG by convention.
    target.starts_with("/storage/") && target != "/storage/emulated" && target != "/storage/self"
}

/// Default (non-portable) data directory, honoring `VIBE_DATA_DIR`.
pub fn default_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VIBE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("vibe-local");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Library/Application Support/vibe-local");
        }
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("vibe-local");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local/share/vibe-local");
        }
    }
    PathBuf::from("vibe_data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_parsing() {
        let text = "MemTotal:       16384000 kB\nMemFree:         1024000 kB\nMemAvailable:    8192000 kB\nBuffers:          512000 kB\nCached:          2048000 kB\n";
        let (total, avail) = parse_meminfo(text);
        assert_eq!(total, 16000);
        assert_eq!(avail, 8000);
    }

    #[test]
    fn meminfo_fallback_without_available() {
        let text =
            "MemTotal: 8192000 kB\nMemFree: 1024000 kB\nBuffers: 512000 kB\nCached: 2048000 kB\n";
        let (total, avail) = parse_meminfo(text);
        assert_eq!(total, 8000);
        assert_eq!(avail, 3500);
    }

    #[test]
    fn real_scan_is_sane() {
        let report = HardwareProbe::scan();
        assert!(report.ram_total_mb > 0, "RAM must be read from the OS");
        assert!(report.ram_free_mb <= report.ram_total_mb);
        assert!(report.cpu_cores >= 1);
        assert!(!report.os.is_empty());
        assert!(["cpu", "vulkan", "cuda", "metal"].contains(&report.preferred_backend.as_str()));
        assert!(!report.tier.is_empty());
    }

    #[test]
    fn backend_preference_order() {
        let g = |b: &str| GpuInfo {
            name: "x".into(),
            vendor: "x".into(),
            vram_mb: Some(8192),
            backend: b.into(),
        };
        assert_eq!(preferred_backend(&[g("vulkan"), g("cuda")]), "cuda");
        assert_eq!(preferred_backend(&[g("vulkan")]), "vulkan");
        assert_eq!(preferred_backend(&[g("metal")]), "metal");
        assert_eq!(preferred_backend(&[]), "cpu");
        // Larger VRAM wins over backend prestige (RX 580 8GB vs GT 730 2GB).
        let big_vulkan = GpuInfo {
            name: "RX 580".into(),
            vendor: "amd".into(),
            vram_mb: Some(8192),
            backend: "vulkan".into(),
        };
        let small_cuda = GpuInfo {
            name: "GT 730".into(),
            vendor: "nvidia".into(),
            vram_mb: Some(2048),
            backend: "cuda".into(),
        };
        assert_eq!(preferred_backend(&[small_cuda, big_vulkan]), "vulkan");
    }

    #[test]
    fn portable_override() {
        let dir = tempfile::tempdir().unwrap();
        // Safety: single-threaded test touching a process-global env var.
        unsafe { std::env::set_var("VIBE_PORTABLE_ROOT", dir.path()) };
        let info = locate_portable();
        unsafe { std::env::remove_var("VIBE_PORTABLE_ROOT") };
        assert!(info.available);
        assert!(info.root.unwrap().ends_with("portable_data"));
    }
}
