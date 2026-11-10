//! Watches DNS lookups made through the Windows DNS client, which is how a wildcard rule learns the domains apps use.
//!
//! Filters only see IP addresses and a pattern can't be resolved in advance, so the names have to be observed. Requires
//! administrator rights. Nothing here sends anything to the network.

use std::net::IpAddr;
use std::sync::OnceLock;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::core::{GUID, PWSTR, w};

/// Microsoft-Windows-DNS-Client.
const PROVIDER: GUID = GUID::from_u128(0x1C95126E_7EEA_49A9_A3FE_A378B03DDB4D);
const SESSION_NAME: windows::core::PCWSTR = w!("KillSwitchDns");
const NAME_CAPACITY: usize = 64;
const EVENT_QUERY_COMPLETED: u16 = 3008;
/// Query type (u32), options (u64) and status (u32) sit between the name and the results.
const STATUS_OFFSET: usize = 12;
const RESULTS_OFFSET: usize = 16;
const INVALID_TRACE: u64 = u64::MAX;

pub type Handler = Box<dyn Fn(&str, Vec<IpAddr>) + Send + Sync>;

static HANDLER: OnceLock<Handler> = OnceLock::new();

#[repr(C)]
struct Properties {
    base: EVENT_TRACE_PROPERTIES,
    name: [u16; NAME_CAPACITY],
}

fn properties() -> Properties {
    let mut properties = Properties { base: EVENT_TRACE_PROPERTIES::default(), name: [0; NAME_CAPACITY] };
    properties.base.Wnode.BufferSize = size_of::<Properties>() as u32;
    properties.base.LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    properties
}

fn read_wide(data: &[u8]) -> Option<(String, &[u8])> {
    let units: Vec<u16> = data.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).take_while(|unit| *unit != 0).collect();
    let consumed = (units.len() + 1) * 2;
    (consumed <= data.len()).then(|| (String::from_utf16_lossy(&units), &data[consumed..]))
}

/// The layout of event 3008: name, type, options, status, then the answers separated by semicolons.
fn parse_query(data: &[u8]) -> Option<(String, Vec<IpAddr>)> {
    let (name, rest) = read_wide(data)?;
    let status = rest.get(STATUS_OFFSET..RESULTS_OFFSET)?;
    if status != [0; 4] {
        return None;
    }
    let (results, _) = read_wide(&rest[RESULTS_OFFSET..])?;
    let addresses = results.split(';').filter_map(|entry| entry.trim().parse::<IpAddr>().ok()).map(|address| address.to_canonical()).collect();
    Some((name.trim_end_matches('.').to_lowercase(), addresses))
}

unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    let record = unsafe { &*record };
    if record.EventHeader.EventDescriptor.Id != EVENT_QUERY_COMPLETED || record.UserData.is_null() {
        return;
    }
    let data = unsafe { std::slice::from_raw_parts(record.UserData as *const u8, usize::from(record.UserDataLength)) };
    if let (Some((name, addresses)), Some(handler)) = (parse_query(data), HANDLER.get()) {
        handler(&name, addresses);
    }
}

/// Ends the trace session, which would otherwise outlive this process.
pub fn stop() {
    let mut properties = properties();
    unsafe {
        let _ = ControlTraceW(CONTROLTRACE_HANDLE::default(), SESSION_NAME, &mut properties.base, EVENT_TRACE_CONTROL_STOP);
    }
}

/// Delivers completed lookups to `handler`; blocks for as long as the session lasts.
pub fn run(handler: Handler) -> Result<(), String> {
    HANDLER.set(handler).map_err(|_| "already running".to_owned())?;
    unsafe {
        let mut session = CONTROLTRACE_HANDLE::default();
        let mut settings = properties();
        settings.base.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        settings.base.Wnode.ClientContext = 1;
        settings.base.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        let mut status = StartTraceW(&mut session, SESSION_NAME, &mut settings.base);
        if status == ERROR_ALREADY_EXISTS {
            // Left behind by a process that crashed.
            stop();
            status = StartTraceW(&mut session, SESSION_NAME, &mut settings.base);
        }
        if status != ERROR_SUCCESS {
            return Err(format!("start trace: {}", status.0));
        }
        let status = EnableTraceEx2(session, &PROVIDER, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, TRACE_LEVEL_INFORMATION as u8, u64::MAX, 0, 0, None);
        if status != ERROR_SUCCESS {
            stop();
            return Err(format!("enable provider: {}", status.0));
        }
        let mut logfile = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(SESSION_NAME.as_ptr() as *mut u16), ..Default::default() };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        let trace = OpenTraceW(&mut logfile);
        if trace.Value == INVALID_TRACE {
            stop();
            return Err("open trace".to_owned());
        }
        let status = ProcessTrace(&[trace], None, None);
        let _ = CloseTrace(trace);
        if status == ERROR_SUCCESS { Ok(()) } else { Err(format!("process trace: {}", status.0)) }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_query;
    use std::net::IpAddr;

    fn wide(text: &str) -> Vec<u8> {
        text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
    }

    fn event(name: &str, status: u32, results: &str) -> Vec<u8> {
        let mut data = wide(name);
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&status.to_le_bytes());
        data.extend(wide(results));
        data
    }

    #[test]
    fn reads_names_and_addresses() {
        let (name, addresses) = parse_query(&event("WWW.Example.COM.", 0, "type:  5 alias.example.net;93.184.216.34;::ffff:1.2.3.4;2606:2800::1;")).unwrap();
        assert_eq!(name, "www.example.com");
        let expected: Vec<IpAddr> = ["93.184.216.34", "1.2.3.4", "2606:2800::1"].iter().map(|text| text.parse().unwrap()).collect();
        assert_eq!(addresses, expected);
    }

    #[test]
    fn failed_or_truncated_lookups_are_ignored() {
        assert_eq!(parse_query(&event("example.com", 9003, "")), None);
        assert_eq!(parse_query(&wide("example.com")), None);
        assert_eq!(parse_query(&[1, 0, 2]), None);
    }
}
