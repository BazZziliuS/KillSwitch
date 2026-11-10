fn main() {
    println!("cargo:rerun-if-changed=assets/KillSwitch.ico");
    winresource::WindowsResource::new().set_icon("assets/KillSwitch.ico").compile().expect("embed the application icon");
}
