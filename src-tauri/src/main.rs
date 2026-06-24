// On Windows, prevent a console window from appearing in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    dpi_bypass_lib::run();
}
