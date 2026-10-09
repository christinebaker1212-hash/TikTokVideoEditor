//! Background work (sync, tracking, export, audio loading) with progress.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub struct Job<T> {
    rx: mpsc::Receiver<anyhow::Result<T>>,
    progress: Arc<Mutex<(f32, String)>>,
    cancel: Arc<AtomicBool>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(
        ctx: &egui::Context,
        f: impl FnOnce(&dyn Fn(f32, &str), &AtomicBool) -> anyhow::Result<T> + Send + 'static,
    ) -> Job<T> {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(Mutex::new((0.0, String::new())));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c, ctx) = (progress.clone(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let ctx2 = ctx.clone();
            let report = move |f: f32, msg: &str| {
                if let Ok(mut g) = p.lock() {
                    *g = (f, msg.to_string());
                }
                ctx2.request_repaint();
            };
            let result = f(&report, &c);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Job { rx, progress, cancel }
    }

    /// The result, once the job has finished.
    pub fn poll(&self) -> Option<anyhow::Result<T>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("background task crashed"))),
        }
    }

    pub fn progress(&self) -> (f32, String) {
        self.progress.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Poll an optional job slot; returns the result and clears the slot when done.
pub fn take<T: Send + 'static>(slot: &mut Option<Job<T>>) -> Option<anyhow::Result<T>> {
    let r = slot.as_ref()?.poll()?;
    *slot = None;
    Some(r)
}
