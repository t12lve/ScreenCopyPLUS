#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod app;
mod file_manager;
mod process;
mod scrcpy;
mod types;

use app::ScreenCopyApp;
use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ScreenCopyPLUS - Android Mirroring & Easy Transfer")
            .with_inner_size([1020.0, 720.0])
            .with_min_inner_size([750.0, 500.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "ScreenCopyPLUS",
        native_options,
        Box::new(|cc| Ok(Box::new(ScreenCopyApp::new(cc)))),
    )
}
