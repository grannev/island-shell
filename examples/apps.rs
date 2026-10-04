// Dev tool: list what the launcher would show, with the icon each entry
// resolves to.  cargo run --example apps -- [query]
#[path = "../src/apps.rs"]
mod apps;
#[path = "../src/background.rs"]
mod background;
#[path = "../src/iconload.rs"]
mod iconload;
#[path = "../src/launch.rs"]
mod launch;

/// Icon size the launcher's rows draw at on this display (57 px at 1.5).
const PX: i32 = 57;

fn main() {
    let query = std::env::args().nth(1).unwrap_or_default();
    let all = apps::scan();
    let icons = iconload::Icons::new();
    println!("{} entries", all.len());
    let shown = apps::search(&all, &query, 12);

    // Icons load on a worker now: ask for every row, then wait until the
    // worker goes quiet (a name the theme has no icon for never answers).
    for app in &shown {
        icons.get_scaled(app.icon.as_deref().unwrap_or(""), PX);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut quiet = 0;
    while quiet < 10 && std::time::Instant::now() < deadline {
        quiet = if icons.drain() { 0 } else { quiet + 1 };
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    for app in &shown {
        let icon = app.icon.as_deref().unwrap_or("");
        let surface = icons.get_scaled(icon, PX);
        println!(
            "{:<28} {:<28} icon={:<24} {}",
            app.name,
            app.subtitle.chars().take(26).collect::<String>(),
            icon,
            surface.map(|s| format!("{}x{}", s.width(), s.height())).unwrap_or_else(|| "—".into())
        );
    }
}
