//! Picture tests: a scene rendered offscreen is compared with its
//! reference in `tests/golden/<name>.png`. A missing reference is written
//! (and the test passes, saying so); `UPDATE_GOLDEN=1` rewrites them all
//! after a deliberate change of look. A mismatch leaves the picture it got
//! and a map of the differences in `target/golden/` for a look.

use std::path::PathBuf;

/// A channel may differ this much (antialiasing, rounding) before a pixel
/// counts as different, and this share of pixels may differ.
const CHANNEL_SLACK: u8 = 24;
const PIXEL_SHARE: f64 = 0.0003;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A surface's pixels as RGB24 rows (its own stride for that format).
fn pixels(surface: &cairo::ImageSurface) -> Vec<u8> {
    let mut copy = cairo::ImageSurface::create(cairo::Format::Rgb24, surface.width(), surface.height()).unwrap();
    {
        let cr = cairo::Context::new(&copy).unwrap();
        let _ = cr.set_source_surface(surface, 0.0, 0.0);
        let _ = cr.paint();
    }
    copy.flush();
    let data = copy.data().unwrap().to_vec();
    data
}

/// Compare `surface` with the reference `name`; panics on a mismatch.
pub fn check(name: &str, surface: &cairo::ImageSurface) {
    surface.flush();
    let reference = root().join("tests/golden").join(format!("{name}.png"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() || !reference.exists() {
        std::fs::create_dir_all(reference.parent().unwrap()).unwrap();
        surface.write_to_png(&mut std::fs::File::create(&reference).unwrap()).unwrap();
        eprintln!("golden: wrote {}", reference.display());
        return;
    }
    let mut want = cairo::ImageSurface::create_from_png(&mut std::fs::File::open(&reference).unwrap()).unwrap();
    let (w, h) = (surface.width(), surface.height());
    assert_eq!((want.width(), want.height()), (w, h), "{name}: the picture changed size");
    let (sa, sb) = (cairo::Format::Rgb24.stride_for_width(w as u32).unwrap() as usize, want.stride() as usize);
    // (read through a copy: the scene's surface may still be shared)
    let got = pixels(surface);
    let exp = pixels(&mut want);
    let mut diff = cairo::ImageSurface::create(cairo::Format::Rgb24, w, h).unwrap();
    let mut differ = 0usize;
    {
        let sd = diff.stride() as usize;
        let mut map = diff.data().unwrap();
        for y in 0..h as usize {
            for x in 0..w as usize {
                // (colour only: RGB24's fourth byte is unused)
                let a = &got[y * sa + x * 4..y * sa + x * 4 + 3];
                let b = &exp[y * sb + x * 4..y * sb + x * 4 + 3];
                let off = a.iter().zip(b).any(|(p, q)| p.abs_diff(*q) > CHANNEL_SLACK);
                if off {
                    differ += 1;
                }
                // the reference faint, the differences red
                let grey = b[0] / 4;
                let px = &mut map[y * sd + x * 4..y * sd + x * 4 + 4];
                px.copy_from_slice(&if off { [0, 0, 255, 255] } else { [grey, grey, grey, 255] });
            }
        }
    }
    let share = differ as f64 / (w as f64 * h as f64);
    if share > PIXEL_SHARE {
        let out = root().join("target/golden");
        std::fs::create_dir_all(&out).unwrap();
        surface.write_to_png(&mut std::fs::File::create(out.join(format!("{name}.png"))).unwrap()).unwrap();
        diff.write_to_png(&mut std::fs::File::create(out.join(format!("{name}-diff.png"))).unwrap()).unwrap();
        panic!("{name}: {differ} pixels ({:.2} %) differ from tests/golden/{name}.png; see target/golden/ (UPDATE_GOLDEN=1 accepts a deliberate change)", share * 100.0);
    }
}
