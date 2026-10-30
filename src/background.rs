//! Bounded background jobs. File I/O and decoding never run in a paint call.
use std::cell::RefCell;
use std::sync::{Arc, Mutex, mpsc};

use smithay_client_toolkit::reexports::calloop::channel::Sender;

pub struct Worker<K, V> {
    tx: mpsc::SyncSender<K>,
    rx: RefCell<mpsc::Receiver<(K, V)>>,
    wake: Arc<Mutex<Option<Sender<()>>>>,
}

impl<K: Send + Clone + 'static, V: Send + 'static> Worker<K, V> {
    pub fn new(name: &str, mut load: impl FnMut(K) -> V + Send + 'static) -> Self {
        let (tx, jobs) = mpsc::sync_channel::<K>(64);
        let (done, rx) = mpsc::channel();
        let wake = Arc::new(Mutex::new(None::<Sender<()>>));
        let notifier = wake.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while let Ok(key) = jobs.recv() {
                    let value = load(key.clone());
                    if done.send((key, value)).is_err() {
                        break;
                    }
                    if let Some(tx) = notifier.lock().unwrap().as_ref() {
                        let _ = tx.send(());
                    }
                }
            })
            .expect("asset worker");
        Self { tx, rx: RefCell::new(rx), wake }
    }

    pub fn set_waker(&self, tx: Sender<()>) {
        *self.wake.lock().unwrap() = Some(tx);
    }
    pub fn request(&self, key: K) -> bool {
        self.tx.try_send(key).is_ok()
    }
    pub fn drain(&self) -> Vec<(K, V)> {
        self.rx.borrow().try_iter().collect()
    }
}

/// Only owned pixel bytes cross threads; Cairo objects stay on their owner.
#[derive(Clone)]
pub struct Pixels {
    bytes: Vec<u8>,
    width: i32,
    height: i32,
    stride: i32,
}

impl Pixels {
    pub fn from_surface(surface: cairo::ImageSurface) -> Option<Self> {
        let mut bytes = Vec::new();
        surface.with_data(|data| bytes.extend_from_slice(data)).ok()?;
        Some(Self { bytes, width: surface.width(), height: surface.height(), stride: surface.stride() })
    }
    pub fn into_surface(self) -> Option<cairo::ImageSurface> {
        cairo::ImageSurface::create_for_data(self.bytes, cairo::Format::ARgb32, self.width, self.height, self.stride)
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_work_does_not_block_requests_and_queue_is_bounded() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = Worker::new("test-worker", move |key| {
            if key == 0 {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            key * 2
        });
        assert!(worker.request(0));
        started_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        for i in 1..=64 {
            assert!(worker.request(i));
        }
        assert!(!worker.request(65));
        assert!(worker.drain().is_empty());
        release_tx.send(()).unwrap();
    }

    #[test]
    fn pixels_cross_threads_without_sharing_cairo_objects() {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 3, 2).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.set_source_rgba(0.5, 0.25, 0.75, 0.5);
        cr.paint().unwrap();
        drop(cr);
        let pixels = Pixels::from_surface(surface).unwrap();
        let surface = std::thread::spawn(move || {
            let surface = pixels.into_surface().unwrap();
            assert_eq!((surface.width(), surface.height()), (3, 2));
            Pixels::from_surface(surface).unwrap()
        })
        .join()
        .unwrap()
        .into_surface()
        .unwrap();
        surface
            .with_data(|data| {
                assert!(data.chunks_exact(4).all(|p| u32::from_ne_bytes(p.try_into().unwrap()) >> 24 == 128))
            })
            .unwrap();
    }
}
