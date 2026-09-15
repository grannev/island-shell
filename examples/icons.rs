// Dev tool: render every notification icon into a grid PNG so the vectors
// can be eyeballed without waiting for real notifications.
//   cargo run --example icons -- /tmp/icons.png [seconds-into-the-entrance]
#[path = "../src/anim.rs"]
mod anim;
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/notify_kinds.rs"]
mod notify;

use notify::AppKind;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "/tmp/icons.png".into());
    let t: f64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let kinds = [AppKind::Telegram, AppKind::System, AppKind::Firefox, AppKind::Discord, AppKind::Mail, AppKind::Bell, AppKind::Critical, AppKind::Editor];
    let (cell, size) = (160.0, 120.0);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, (cell * kinds.len() as f64) as i32, cell as i32).unwrap();
    let cr = cairo::Context::new(&surface).unwrap();
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.paint();
    for (i, kind) in kinds.iter().enumerate() {
        icons::draw(&cr, *kind, cell * (i as f64 + 0.5), cell / 2.0, size, t, 1.0);
    }
    surface.write_to_png(&mut std::fs::File::create(&path).unwrap()).unwrap();
    println!("{path}");
}
