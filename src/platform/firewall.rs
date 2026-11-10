//! Per-application blocking through the Windows Filtering Platform.
//!
//! Filters are persistent, so a crash or reboot while blocked keeps the applications offline. WFP is used rather than
//! Windows Firewall rules because firewall rules do not apply to loopback, which is how applications reach a local proxy.

use std::net::IpAddr;
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

/// Everything that must be offline right now. The default value removes every block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub apps: Vec<PathBuf>,
    /// Lose only IPv6, for when the tunnel covers IPv4 and IPv6 would go around it.
    pub ipv6_only_apps: Vec<PathBuf>,
    pub sites: Vec<IpAddr>,
    /// VPN clients keep reaching blocked addresses: their server may share an address with a blocked site.
    pub exempt: Vec<PathBuf>,
}

/// Atomically makes `plan` the exact set of blocks.
/// Returns the paths that could not be blocked because the file no longer exists.
pub fn apply(plan: &Plan) -> Result<Vec<PathBuf>, String> {
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
        for app in &plan.apps {
            if !add_app(&engine, app)? {
                missing.push(app.clone());
            }
        }
        for app in &plan.ipv6_only_apps {
            if !add_app_ipv6(&engine, app)? {
                missing.push(app.clone());
            }
        }
        for address in &plan.sites {
            add_site(&engine, *address)?;
        }
        if !plan.sites.is_empty() {
            for client in &plan.exempt {
                exempt_client(&engine, client)?;
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

/// Inside the sublayer the highest weight decides, so a client's permit outranks a site's block.
const WEIGHT_SITE_BLOCK: u8 = 1;
const WEIGHT_CLIENT_PERMIT: u8 = 15;

unsafe fn add_filter(
    engine: &Engine,
    label: &str,
    layer: GUID,
    conditions: &mut [FWPM_FILTER_CONDITION0],
    action: FWP_ACTION_TYPE,
    weight: Option<u8>,
) -> Result<(), String> {
    let mut name = wide(format!("KillSwitch: {label}").as_ref());
    let mut provider_key = PROVIDER;
    let mut filter = FWPM_FILTER0 {
        displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() },
        flags: FWPM_FILTER_FLAG_PERSISTENT,
        providerKey: &mut provider_key,
        layerKey: layer,
        subLayerKey: SUBLAYER,
        numFilterConditions: conditions.len() as u32,
        filterCondition: conditions.as_mut_ptr(),
        ..Default::default()
    };
    match weight {
        Some(weight) => {
            filter.weight.r#type = FWP_UINT8;
            filter.weight.Anonymous.uint8 = weight;
        }
        None => filter.weight.r#type = FWP_EMPTY,
    }
    filter.action.r#type = action;
    check(unsafe { FwpmFilterAdd0(engine.0, &filter, None, None) }, "add WFP filter")
}

/// Calls `body` with a condition matching the executable. Returns false when the file does not exist.
unsafe fn with_app_condition(
    app: &Path,
    body: impl FnOnce(&mut FWPM_FILTER_CONDITION0) -> Result<(), String>,
) -> Result<bool, String> {
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
    let result = body(&mut condition);
    let mut memory = blob.cast();
    unsafe { FwpmFreeMemory0(&mut memory) };
    result.map(|()| true)
}

/// Returns false when the executable does not exist.
unsafe fn add_app(engine: &Engine, app: &Path) -> Result<bool, String> {
    let label = app.display().to_string();
    unsafe {
        with_app_condition(app, |condition| {
            for layer in [
                FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4,
                FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6,
            ] {
                add_filter(engine, &label, layer, std::slice::from_mut(condition), FWP_ACTION_BLOCK, None)?;
            }
            Ok(())
        })
    }
}

/// Loopback stays open: a local service on `::1` is not a route around the tunnel.
unsafe fn add_app_ipv6(engine: &Engine, app: &Path) -> Result<bool, String> {
    let label = format!("IPv6 {}", app.display());
    let not_loopback = FWPM_FILTER_CONDITION0 {
        fieldKey: FWPM_CONDITION_FLAGS,
        matchType: FWP_MATCH_FLAGS_NONE_SET,
        conditionValue: FWP_CONDITION_VALUE0 {
            r#type: FWP_UINT32,
            Anonymous: FWP_CONDITION_VALUE0_0 { uint32: FWP_CONDITION_FLAG_IS_LOOPBACK },
        },
    };
    unsafe {
        with_app_condition(app, |condition| {
            let mut conditions = [*condition, not_loopback];
            for layer in [FWPM_LAYER_ALE_AUTH_CONNECT_V6, FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6] {
                add_filter(engine, &label, layer, &mut conditions, FWP_ACTION_BLOCK, None)?;
            }
            Ok(())
        })
    }
}

unsafe fn exempt_client(engine: &Engine, client: &Path) -> Result<(), String> {
    let label = format!("allow {}", client.display());
    unsafe {
        with_app_condition(client, |condition| {
            for layer in [FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6] {
                add_filter(engine, &label, layer, std::slice::from_mut(condition), FWP_ACTION_PERMIT, Some(WEIGHT_CLIENT_PERMIT))?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

unsafe fn add_site(engine: &Engine, address: IpAddr) -> Result<(), String> {
    // Stays alive until the filter is added: the condition only points at it.
    let mut v6 = FWP_BYTE_ARRAY16::default();
    let (layer, value) = match address {
        IpAddr::V4(v4) => (
            FWPM_LAYER_ALE_AUTH_CONNECT_V4,
            FWP_CONDITION_VALUE0 { r#type: FWP_UINT32, Anonymous: FWP_CONDITION_VALUE0_0 { uint32: u32::from(v4) } },
        ),
        IpAddr::V6(address) => {
            v6.byteArray16 = address.octets();
            (
                FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                FWP_CONDITION_VALUE0 {
                    r#type: FWP_BYTE_ARRAY16_TYPE,
                    Anonymous: FWP_CONDITION_VALUE0_0 { byteArray16: &mut v6 },
                },
            )
        }
    };
    let mut condition =
        FWPM_FILTER_CONDITION0 { fieldKey: FWPM_CONDITION_IP_REMOTE_ADDRESS, matchType: FWP_MATCH_EQUAL, conditionValue: value };
    unsafe {
        add_filter(engine, &format!("site {address}"), layer, std::slice::from_mut(&mut condition), FWP_ACTION_BLOCK, Some(WEIGHT_SITE_BLOCK))
    }
}
