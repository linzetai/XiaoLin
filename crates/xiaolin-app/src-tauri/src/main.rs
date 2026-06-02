#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(unused_crate_dependencies)]

fn main() {
    let _ = fix_path_env::fix();
    xiaolin_app_lib::run();
}
