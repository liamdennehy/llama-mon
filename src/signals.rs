//! Dedicated OS signal handling thread.
//!
//! Uses `signal_hook::flag` to register blocking hooks in a spawned thread.
//! Each signal sets an `AtomicUsize` flag that the reactor checks at well-defined
//! boundaries. This avoids holding the reactor task hostage on `current_thread`
//! and scales cleanly to additional signals.
//!
//! Signal mapping:
//! - 0: no signal
//! - 1: SIGINT / SIGTERM — graceful shutdown
//! - 2: SIGUSR1 — dump model state

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Atomic flag set by the signal handler thread.
pub struct SignalFlag {
    flag: Arc<AtomicUsize>,
}

impl SignalFlag {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Check and clear the signal flag. Returns the signal action if set,
    /// or `None` if no signal arrived.
    pub fn take(&self) -> Option<usize> {
        let val = self.flag.swap(0, Ordering::SeqCst);
        if val == 0 { None } else { Some(val) }
    }

    /// Check if any signal is pending.
    pub fn has_pending(&self) -> bool {
        self.flag.load(Ordering::SeqCst) != 0
    }
}

/// Spawn the signal handler thread.
///
/// Registers blocking hooks for SIGINT, SIGTERM, SIGUSR1.
/// The thread runs until the returned `JoinHandle` is dropped.
pub fn spawn_handler(flags: &SignalFlag) -> std::thread::JoinHandle<()> {
    let flag = Arc::clone(&flags.flag);
    std::thread::spawn(move || {
        // Use signal_hook::flag to register blocking hooks.
        // These run in the signal handler context (no async), so we just
        // set the atomic flag — the actual work happens on the async runtime.
        signal_hook::flag::register_usize(signal_hook::consts::SIGINT, Arc::clone(&flag), 1)
            .expect("signal_hook::register failed");
        signal_hook::flag::register_usize(signal_hook::consts::SIGTERM, Arc::clone(&flag), 1)
            .expect("signal_hook::register failed");
        signal_hook::flag::register_usize(signal_hook::consts::SIGUSR1, Arc::clone(&flag), 2)
            .expect("signal_hook::register failed");

        // Block forever — the thread lives for the lifetime of the process.
        loop {
            std::thread::park();
        }
    })
}
