#![cfg_attr(
    all(target_os = "windows", not(feature = "console")),
    windows_subsystem = "windows"
)]

fn main() -> anyhow::Result<()> {
    displaymux_app_lib::run()
}
