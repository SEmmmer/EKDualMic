use anyhow::Result;
use audio_capture::list_capture_devices;
use audio_output::list_render_devices;
use common_types::AudioDeviceInfo;

fn main() -> Result<()> {
    let capture_devices = list_capture_devices()?;
    let render_devices = list_render_devices()?;

    print_section("Capture Devices", &capture_devices);
    println!();
    print_section("Render Devices", &render_devices);

    Ok(())
}

fn print_section(title: &str, devices: &[AudioDeviceInfo]) {
    println!("{title}:");
    if devices.is_empty() {
        println!("  <none>");
        return;
    }

    for device in devices {
        let default_tag = if device.is_default { " [default]" } else { "" };
        println!("  - {}{}", device.name, default_tag);
        println!("    id: {}", device.id);
    }
}
