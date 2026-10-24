//! island-shell: a Dynamic-Island-style notch for SwayFX, drawn by one
//! Wayland layer-shell client. Without arguments it runs the island; with a
//! command it talks to the running one (see `ctl`).

mod anim;
mod calc;
mod calendar;
mod clipboard;
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
mod floating;
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
mod recorder;
mod notify_kinds;
mod sway;
mod sound;
mod tasks;
mod timer_page;
mod touchid;
mod weather;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "run") {
        run(&args[1..]);
    }
    // a job belongs to whoever started it (the calling shell or script):
    // when that process ends without ending the job, the island does
    if args.first().is_some_and(|a| a == "task" || a == "progress") && !args.get(1).is_some_and(|a| a.starts_with("--pid=")) {
        args.insert(1, format!("--pid={}", std::os::unix::process::parent_id()));
    }
    // one command is one line
    for arg in &mut args {
        if arg.contains(['\n', '\r']) { *arg = arg.replace(['\n', '\r'], " "); }
    }
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

/// `island-shell run [--title <title>] [--] <command> [args…]`: run a
/// command as a live activity — a turning ring beside the clock while it
/// runs, a tick or a cross with its exit code when it ends. The command's
/// input, output and exit status pass through untouched; without a running
/// island it simply runs.
fn run(args: &[String]) -> ! {
    use std::os::unix::process::ExitStatusExt;
    let mut title = None;
    let mut rest = args;
    loop {
        match rest {
            [flag, value, tail @ ..] if flag == "--title" || flag == "-t" => {
                title = Some(value.clone());
                rest = tail;
            }
            [dashes, tail @ ..] if dashes == "--" => {
                rest = tail;
                break;
            }
            _ => break,
        }
    }
    let Some((program, arguments)) = rest.split_first() else {
        eprintln!("usage: island-shell run [--title <title>] [--] <command> [args…]");
        std::process::exit(2);
    };
    // "cargo build", "rsync", "make install": the program and its verb
    let title = title.unwrap_or_else(|| {
        let name = std::path::Path::new(program).file_name().map_or(program.clone(), |n| n.to_string_lossy().into_owned());
        match arguments.first().filter(|a| !a.starts_with('-') && !a.contains('/') && a.len() <= 24) {
            Some(verb) => format!("{name} {verb}"),
            None => name,
        }
    });
    let pid = std::process::id();
    let id = format!("run-{pid}");
    let _ = ctl::send(&format!("task --pid={pid} start {id} {}", title.replace(['\n', '\r'], " ")));
    let (verb, detail, code) = match std::process::Command::new(program).args(arguments).status() {
        Ok(status) if status.success() => ("done", String::new(), 0),
        Ok(status) => match (status.code(), status.signal()) {
            (Some(code), _) => ("fail", format!("код {code}"), code),
            (None, Some(signal)) => ("fail", "прервано".to_string(), 128 + signal),
            (None, None) => ("fail", String::new(), 1),
        },
        Err(e) => {
            eprintln!("island-shell: {program}: {e}");
            ("fail", "не удалось запустить".to_string(), 127)
        }
    };
    let _ = ctl::send(&format!("task {verb} {id} {detail}"));
    std::process::exit(code);
}
