//! Starts KillSwitch at logon through Task Scheduler, the one autostart mechanism that can elevate without a UAC prompt.

use super::firewall::DRY_RUN;
use crate::config;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;

const TASK_NAME: &str = "KillSwitch";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const ARGUMENT: &str = "--autostart";

fn schtasks(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("schtasks").args(args).creation_flags(CREATE_NO_WINDOW).output()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Given as XML because `schtasks` flags can't lift the defaults that stop a task on battery and after 72 hours.
fn task_xml(exe: &Path, user: &str) -> String {
    let (exe, user) = (escape(&exe.display().to_string()), escape(user));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>{ARGUMENT}</Arguments></Exec></Actions>
</Task>
"#
    )
}

fn create() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").map_err(|error| error.to_string())?,
        std::env::var("USERNAME").map_err(|error| error.to_string())?
    );
    let file = config::data_path("autostart-task.xml");
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(task_xml(&exe, &user).encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&file, bytes).map_err(|error| error.to_string())?;
    let path = file.to_string_lossy().into_owned();
    let output = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &path, "/F"]);
    let _ = std::fs::remove_file(&file);
    let output = output.map_err(|error| error.to_string())?;
    if output.status.success() { Ok(()) } else { Err(format!("schtasks: {}", output.status)) }
}

fn remove() -> Result<(), String> {
    let exists = schtasks(&["/Query", "/TN", TASK_NAME]).map_err(|error| error.to_string())?.status.success();
    if !exists {
        return Ok(());
    }
    let output = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]).map_err(|error| error.to_string())?;
    if output.status.success() { Ok(()) } else { Err(format!("schtasks: {}", output.status)) }
}

pub fn set(enabled: bool) -> Result<(), String> {
    if DRY_RUN.load(Ordering::SeqCst) {
        return Ok(());
    }
    if enabled { create() } else { remove() }
}

#[cfg(test)]
mod tests {
    use super::task_xml;
    use std::path::Path;

    #[test]
    fn task_starts_hidden_and_elevated_and_never_expires() {
        let xml = task_xml(Path::new(r"C:\Tools & Co\KillSwitch.exe"), r"PC\user");
        assert!(xml.contains(r"<Command>C:\Tools &amp; Co\KillSwitch.exe</Command>"));
        assert!(xml.contains("<Arguments>--autostart</Arguments>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
    }
}
