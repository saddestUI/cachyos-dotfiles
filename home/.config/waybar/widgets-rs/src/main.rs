mod common;
mod widgets;

use gtk::prelude::*;

fn main() {
    let Some(name) = std::env::args().nth(1) else {
        eprintln!("Usage: waybar-widgets <calendar|language|audio|brightness|network|bluetooth|power>");
        std::process::exit(1);
    };

    let valid = matches!(
        name.as_str(),
        "calendar" | "language" | "audio" | "brightness" | "network" | "bluetooth" | "power" | "system"
    );
    if !valid {
        eprintln!("Unknown widget: {name}");
        std::process::exit(1);
    }

    if common::toggle_or_replace(&name) {
        return;
    }

    glib::unix_signal_add_local(libc::SIGTERM, || {
        common::close_app();
        glib::ControlFlow::Break
    });

    if gtk::init().is_err() {
        common::cleanup_pid();
        eprintln!("Failed to initialize GTK");
        std::process::exit(1);
    }

    common::load_css();

    let win = match name.as_str() {
        "calendar" => widgets::calendar::build(),
        "language" => widgets::language::build(),
        "audio" => widgets::audio::build(),
        "brightness" => widgets::brightness::build(),
        "network" => widgets::network::build(),
        "bluetooth" => widgets::bluetooth::build(),
        "power" => widgets::power::build(),
        "system" => widgets::system::build(),
        _ => unreachable!(),
    };

    win.show_all();
    gtk::main();
    common::cleanup_pid();
}
