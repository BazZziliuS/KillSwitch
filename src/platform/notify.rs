//! Windows toast notifications. An unpackaged app has to register a display name before it may post them.

use std::sync::Once;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::core::{HSTRING, w};

const APP_ID: &str = "BazZziliuS.KillSwitch";

fn register() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        let name: Vec<u16> = "KillSwitch".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                w!(r"Software\Classes\AppUserModelId\BazZziliuS.KillSwitch"),
                w!("DisplayName"),
                REG_SZ.0,
                Some(name.as_ptr().cast()),
                (name.len() * size_of::<u16>()) as u32,
            );
        }
    });
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape(title),
        escape(body)
    )
}

fn post(title: &str, body: &str) -> windows::core::Result<()> {
    register();
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_xml(title, body)))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)
}

/// Posts from a separate thread, so a stalled notification service can never hold up the protection loop.
pub fn send(title: String, body: String) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        }
        if let Err(error) = post(&title, &body) {
            eprintln!("notification failed: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::toast_xml;

    #[test]
    fn network_names_cannot_break_the_markup() {
        let xml = toast_xml("<Title>", "Cafe & <b>");
        assert!(xml.contains("&lt;Title&gt;") && xml.contains("Cafe &amp; &lt;b&gt;"));
    }
}
