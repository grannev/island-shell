// Dev tool: list what the launcher would show, with the icon each entry
// resolves to.  cargo run --example apps -- [query]
#[path = "../src/apps.rs"]
mod apps;
#[path = "../src/iconload.rs"]
mod iconload;

fn main() {
    let query = std::env::args().nth(1).unwrap_or_default();
    let all = apps::scan();
    let icons = iconload::Icons::new();
    println!("{} entries", all.len());
    for app in apps::search(&all, &query, 12) {
        let icon = app.icon.as_deref().unwrap_or("");
        let surface = icons.get(icon);
        println!(
            "{:<28} {:<28} icon={:<24} {}",
            app.name,
            app.subtitle.chars().take(26).collect::<String>(),
            icon,
            surface.map(|s| format!("{}x{}", s.width(), s.height())).unwrap_or_else(|| "—".into())
        );
    }
}
