//! Per-application blocking through the Windows Filtering Platform.
//!
//! Filters are persistent, so a crash or reboot while blocked keeps the applications offline. WFP is used rather than
//! Windows Firewall rules because firewall rules do not apply to loopback, which is how applications reach a local proxy.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
use windows::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows::core::{GUID, PCWSTR, PWSTR};

const PROVIDER: GUID = GUID::from_u128(0x6f1c2a54_8d3e_4b7a_9c21_5e0a7d4b1f01);
const SUBLAYER: GUID = GUID::from_u128(0x6f1c2a54_8d3e_4b7a_9c21_5e0a7d4b1f02);
const RPC_C_AUTHN_WINNT: u32 = 10;
const FWP_E_ALREADY_EXISTS: u32 = 0x8032_0009;
const ENUM_BATCH: u32 = 256;

/// Development mode: the whole app behaves normally, but no filter is ever written.
pub static DRY_RUN: AtomicBool = AtomicBool::new(false);

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

fn check(code: u32, what: &str) -> Result<(), String> {
    if code == ERROR_SUCCESS.0 {
        Ok(())
    } else {
        Err(format!("{what}: 0x{code:08X}"))
    }
}

fn check_new(code: u32, what: &str) -> Result<(), String> {
    if code == FWP_E_ALREADY_EXISTS { Ok(()) } else { check(code, what) }
}

/// Closing the engine aborts any transaction that was not committed.
struct Engine(HANDLE);

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            FwpmEngineClose0(self.0);
        }
    }
}

/// Atomically makes `apps` the exact set of blocked applications; an empty slice removes every block.
/// Returns the paths that could not be blocked because the file no longer exists.
pub fn apply(apps: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if DRY_RUN.load(Ordering::SeqCst) {
        return Ok(Vec::new());
    }
    unsafe {
        let mut handle = HANDLE::default();
        check(
            FwpmEngineOpen0(PCWSTR::null(), RPC_C_AUTHN_WINNT, None, None, &mut handle),
            "open WFP engine",
        )?;
        let engine = Engine(handle);
        check(FwpmTransactionBegin0(engine.0, 0), "begin WFP transaction")?;
        register(&engine)?;
        delete_all(&engine)?;
        let mut missing = Vec::new();
        for app in apps {
            if !add_app(&engine, app)? {
                missing.push(app.clone());
            }
        }
        check(FwpmTransactionCommit0(engine.0), "commit WFP transaction")?;
        Ok(missing)
    }
}

unsafe fn register(engine: &Engine) -> Result<(), String> {
    let mut name = wide("KillSwitch".as_ref());
    let display = FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() };
    let provider = FWPM_PROVIDER0 {
        providerKey: PROVIDER,
        displayData: display,
        flags: FWPM_PROVIDER_FLAG_PERSISTENT,
        ..Default::default()
    };
    check_new(unsafe { FwpmProviderAdd0(engine.0, &provider, None) }, "add WFP provider")?;

    let mut provider_key = PROVIDER;
    let sublayer = FWPM_SUBLAYER0 {
        subLayerKey: SUBLAYER,
        displayData: display,
        flags: FWPM_SUBLAYER_FLAG_PERSISTENT,
        providerKey: &mut provider_key,
        weight: 0xFFFF,
        ..Default::default()
    };
    check_new(unsafe { FwpmSubLayerAdd0(engine.0, &sublayer, None) }, "add WFP sublayer")
}

unsafe fn delete_all(engine: &Engine) -> Result<(), String> {
    let mut enum_handle = HANDLE::default();
    check(unsafe { FwpmFilterCreateEnumHandle0(engine.0, None, &mut enum_handle) }, "enumerate WFP filters")?;
    let mut keys = Vec::new();
    let result = loop {
        let mut entries: *mut *mut FWPM_FILTER0 = std::ptr::null_mut();
        let mut count = 0u32;
        let code = unsafe { FwpmFilterEnum0(engine.0, enum_handle, ENUM_BATCH, &mut entries, &mut count) };
        if let Err(error) = check(code, "read WFP filters") {
            break Err(error);
        }
        if count > 0 {
            for filter in unsafe { std::slice::from_raw_parts(entries, count as usize) } {
                let filter = unsafe { &**filter };
                if filter.subLayerKey == SUBLAYER {
                    keys.push(filter.filterKey);
                }
            }
        }
        let mut memory = entries.cast();
        unsafe { FwpmFreeMemory0(&mut memory) };
        if count < ENUM_BATCH {
            break Ok(());
        }
    };
    unsafe { FwpmFilterDestroyEnumHandle0(engine.0, enum_handle) };
    result?;
    for key in keys {
        check(unsafe { FwpmFilterDeleteByKey0(engine.0, &key) }, "delete WFP filter")?;
    }
    Ok(())
}

/// Returns false when the executable does not exist.
unsafe fn add_app(engine: &Engine, app: &Path) -> Result<bool, String> {
    if !app.exists() {
        return Ok(false);
    }
    let path = wide(app.as_os_str());
    let mut blob: *mut FWP_BYTE_BLOB = std::ptr::null_mut();
    check(
        unsafe { FwpmGetAppIdFromFileName0(PCWSTR(path.as_ptr()), &mut blob) },
        &format!("get app id for {}", app.display()),
    )?;

    let mut condition = FWPM_FILTER_CONDITION0 {
        fieldKey: FWPM_CONDITION_ALE_APP_ID,
        matchType: FWP_MATCH_EQUAL,
        conditionValue: FWP_CONDITION_VALUE0 {
            r#type: FWP_BYTE_BLOB_TYPE,
            Anonymous: FWP_CONDITION_VALUE0_0 { byteBlob: blob },
        },
    };
    let mut name = wide(format!("KillSwitch: {}", app.display()).as_ref());
    let mut provider_key = PROVIDER;
    let mut result = Ok(true);
    for layer in [
        FWPM_LAYER_ALE_AUTH_CONNECT_V4,
        FWPM_LAYER_ALE_AUTH_CONNECT_V6,
        FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4,
        FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6,
    ] {
        let mut filter = FWPM_FILTER0 {
            displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() },
            flags: FWPM_FILTER_FLAG_PERSISTENT,
            providerKey: &mut provider_key,
            layerKey: layer,
            subLayerKey: SUBLAYER,
            numFilterConditions: 1,
            filterCondition: &mut condition,
            ..Default::default()
        };
        filter.weight.r#type = FWP_EMPTY;
        filter.action.r#type = FWP_ACTION_BLOCK;
        if let Err(error) = check(unsafe { FwpmFilterAdd0(engine.0, &filter, None, None) }, "add WFP filter") {
            result = Err(error);
            break;
        }
    }
    let mut memory = blob.cast();
    unsafe { FwpmFreeMemory0(&mut memory) };
    result
}
