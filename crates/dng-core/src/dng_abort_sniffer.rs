// Port of dng_abort_sniffer.h/.cpp — user-cancellation signaling.
//
// C++ virtuals become trait methods.  `Sniff` throws in C++; the Rust trait
// returns `DngResult<()>` (Err = cancel).  Sniffers are shared between host,
// streams and tasks, so they are passed as `Arc<dyn AbortSniffer>` (the C++
// raw-pointer semantics); see DESIGN.md.

use crate::dng_errors::DngResult;
use crate::dng_exceptions::user_cancel;
use crate::dng_types::real64;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

/// Thread priority level (`dng_priority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum DngPriority {
    Background = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    VeryHigh = 4,
}

pub const dng_priority_background: DngPriority = DngPriority::Background;
pub const dng_priority_low: DngPriority = DngPriority::Low;
pub const dng_priority_medium: DngPriority = DngPriority::Medium;
pub const dng_priority_high: DngPriority = DngPriority::High;
pub const dng_priority_very_high: DngPriority = DngPriority::VeryHigh;
pub const dng_priority_count: i32 = 5;
pub const dng_priority_minimum: DngPriority = DngPriority::Background;
pub const dng_priority_maximum: DngPriority = DngPriority::VeryHigh;

/// Port of `dng_abort_sniffer` (abstract base in C++).
pub trait AbortSniffer: Send + Sync {
    /// `Sniff ()` — check for pending user cancellation.  `Err(UserCancel)`
    /// replaces the C++ `ThrowUserCanceled`.
    fn sniff(&self) -> DngResult<()>;

    /// `Priority ()` / `SetPriority ()` — stored per sniffer in C++; trait
    /// implementors may keep their own.  Default: no-op storage accessor is
    /// not possible, so the port exposes explicit methods.
    fn priority(&self) -> DngPriority {
        DngPriority::Background
    }

    fn set_priority(&self, _priority: DngPriority) {}

    /// `ThreadSafe ()`.
    fn thread_safe(&self) -> bool {
        false
    }

    /// `SupportsPriorityWait ()`.
    fn supports_priority_wait(&self) -> bool {
        false
    }

    /// `SuggestedTimeBetweenSniffs ()` — default 0.1 s.
    fn suggested_time_between_sniffs(&self) -> real64 {
        0.1
    }

    /// `StartTask (name, fract)`.
    fn start_task(&self, _name: &str, _fract: real64) {}

    /// `EndTask ()`.
    fn end_task(&self) {}

    /// `UpdateProgress (fract)`.
    fn update_progress(&self, _fract: real64) {}
}

/// C++ `dng_abort_sniffer` stores `fPriority`; concrete implementors in Rust
/// that want the default storage can embed this struct.
#[derive(Debug, Default)]
pub struct SnifferPriority {
    priority: AtomicI32,
}

impl SnifferPriority {
    pub fn get(&self) -> DngPriority {
        match self.priority.load(Ordering::Relaxed) {
            1 => DngPriority::Low,
            2 => DngPriority::Medium,
            3 => DngPriority::High,
            4 => DngPriority::VeryHigh,
            _ => DngPriority::Background,
        }
    }

    pub fn set(&self, priority: DngPriority) {
        self.priority.store(priority as i32, Ordering::Relaxed);
    }
}

/// A sniffer that never aborts (C++ passes NULL; the Rust port uses this
/// shared null implementation where an object is required).
pub struct NullAbortSniffer;

impl AbortSniffer for NullAbortSniffer {
    fn sniff(&self) -> DngResult<()> {
        Ok(())
    }
}

/// `dng_abort_sniffer::SniffForAbort (sniffer)` — static convenience.
/// `None` never aborts (mirrors the NULL case).
pub fn sniff_for_abort(sniffer: Option<&Arc<dyn AbortSniffer>>) -> DngResult<()> {
    if let Some(sniffer) = sniffer {
        sniffer.sniff()?;
    }
    Ok(())
}

/// Port of `dng_set_minimum_priority` — RAII priority lowering.  The Rust
/// port records the request (the C++ implementation is OS-specific); see
/// PORTING_NOTES.md.
pub struct DngSetMinimumPriority {
    priority: DngPriority,
    #[allow(dead_code)]
    name: String,
}

impl DngSetMinimumPriority {
    pub fn new(priority: DngPriority, name: &str) -> Self {
        DngSetMinimumPriority {
            priority,
            name: name.to_string(),
        }
    }
}

impl Drop for DngSetMinimumPriority {
    fn drop(&mut self) {
        let _ = self.priority;
    }
}

/// Port of `dng_sniffer_task` — RAII named subtask scope.
pub struct DngSnifferTask {
    sniffer: Option<Arc<dyn AbortSniffer>>,
}

impl DngSnifferTask {
    pub fn new(sniffer: Option<Arc<dyn AbortSniffer>>, name: &str, fract: real64) -> Self {
        if let Some(sniffer) = &sniffer {
            sniffer.start_task(name, fract);
        }
        DngSnifferTask { sniffer }
    }

    /// `Sniff ()`.
    pub fn sniff(&self) -> DngResult<()> {
        crate::dng_abort_sniffer::sniff_for_abort(self.sniffer.as_ref())
    }

    /// `UpdateProgress (fract)`.
    pub fn update_progress(&self, fract: real64) {
        if let Some(sniffer) = &self.sniffer {
            sniffer.update_progress(fract);
        }
    }

    /// `UpdateProgress (done, total)`.
    pub fn update_progress_units(&self, done: crate::dng_types::uint32, total: crate::dng_types::uint32) {
        self.update_progress(done as real64 / total as real64);
    }

    /// `Finish ()`.
    pub fn finish(&self) {
        self.update_progress(1.0);
    }
}

impl Drop for DngSnifferTask {
    fn drop(&mut self) {
        if let Some(sniffer) = &self.sniffer {
            sniffer.end_task();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sniffer_never_aborts() {
        let sniffer: Arc<dyn AbortSniffer> = Arc::new(NullAbortSniffer);
        assert!(sniff_for_abort(Some(&sniffer)).is_ok());
        assert!(sniff_for_abort(None).is_ok());
    }

    struct CancelSniffer;

    impl AbortSniffer for CancelSniffer {
        fn sniff(&self) -> DngResult<()> {
            Err(user_cancel())
        }
    }

    #[test]
    fn cancel_sniffer_signals() {
        let sniffer: Arc<dyn AbortSniffer> = Arc::new(CancelSniffer);
        let err = sniff_for_abort(Some(&sniffer)).unwrap_err();
        assert_eq!(err, crate::dng_errors::DngError::UserCancel);
    }

    #[test]
    fn sniffer_task_scope() {
        use std::sync::atomic::{AtomicU32, Ordering};
        struct Counting {
            starts: AtomicU32,
            ends: AtomicU32,
        }
        impl AbortSniffer for Counting {
            fn sniff(&self) -> DngResult<()> {
                Ok(())
            }
            fn start_task(&self, _name: &str, _fract: real64) {
                self.starts.fetch_add(1, Ordering::Relaxed);
            }
            fn end_task(&self) {
                self.ends.fetch_add(1, Ordering::Relaxed);
            }
        }
        let counting = Arc::new(Counting {
            starts: AtomicU32::new(0),
            ends: AtomicU32::new(0),
        });
        {
            let task = DngSnifferTask::new(Some(counting.clone()), "test", 0.5);
            task.update_progress(0.5);
            task.finish();
        }
        assert_eq!(counting.starts.load(Ordering::Relaxed), 1);
        assert_eq!(counting.ends.load(Ordering::Relaxed), 1);
    }
}
