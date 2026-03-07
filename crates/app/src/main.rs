#[cfg(windows)]
fn main() -> eframe::Result<()> {
    let _ = app::init_logging("info");
    app::gui::run_native()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The GUI target is Windows only. Run `cargo run -p app` on Windows.");
}
