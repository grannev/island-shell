//! island-shell: a Dynamic-Island-style notch for SwayFX, drawn by one
//! Wayland layer-shell client. Without arguments it runs the island; with a
//! command it talks to the running one (see `ctl`).

mod anim;
mod app;
mod apps;
mod activity;
mod art;
mod ctl;
mod dbus;
mod editor;
mod editor_output;
mod eq;
mod icons;
mod iconload;
mod background;
mod files;
mod island;
mod launcher;
mod menu_style;
mod motion;
mod launch;
mod notify;
mod notice_center;
mod notification_server;
mod nvim;
mod network;
mod connections;
mod control;
mod player;
mod quick;
mod notify_kinds;
mod sway;
mod touchid;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {
            if let Err(e) = app::run() {
                eprintln!("island-shell: {e}");
                std::process::exit(1);
            }
        }
        words if !words.is_empty() && ctl::Command::parse(&words.join(" ")).is_some() => match ctl::send(&words.join(" ")) {
            Ok(reply) => {
                if reply != "ok" {
                    println!("{reply}");
                }
                if reply.starts_with("error") {
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("island-shell: {e}");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!("{}", ctl::USAGE);
            std::process::exit(2);
        }
    }
}
