//! Desktop optical project editor and analysis interface.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod gui;
fn main() {
    gui::main();
}
