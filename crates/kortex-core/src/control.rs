//! Cooperative request cancellation. Checkpoints cannot interrupt a kernel I/O
//! call or an indivisible native inference operation already in progress.
use crate::{Error, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct RequestControl {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

impl RequestControl {
    pub fn new(timeout: Duration) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Error::new("INVALID_ARGS", "request timeout is not representable", 2))?;
        Ok(Self::from_deadline(deadline))
    }

    pub fn from_deadline(deadline: Instant) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline,
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Preserve cancellation propagation while allowing only a shorter deadline.
    pub fn restrict_deadline(&self, deadline: Instant) -> Self {
        Self {
            cancelled: Arc::clone(&self.cancelled),
            deadline: self.deadline.min(deadline),
        }
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(Error::new("CANCELLED", "request cancelled", 9));
        }
        if Instant::now() >= self.deadline {
            return Err(Error::new("TIMEOUT", "request deadline exceeded", 9));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_cancellation_across_threads() {
        let control = RequestControl::new(Duration::from_secs(60)).unwrap();
        let worker = control.clone();
        std::thread::spawn(move || worker.cancel()).join().unwrap();
        assert!(control.is_cancelled());
        let error = control.check().unwrap_err();
        assert_eq!((error.code, error.exit), ("CANCELLED", 9));
    }

    #[test]
    fn past_deadline_fails_without_sleep_or_deadline_extension() {
        let deadline = Instant::now();
        let control = RequestControl::from_deadline(deadline);
        assert_eq!(control.deadline(), deadline);
        assert_eq!(control.remaining(), Duration::ZERO);
        let error = control.check().unwrap_err();
        assert_eq!((error.code, error.exit), ("TIMEOUT", 9));
    }

    #[test]
    fn independent_requests_do_not_share_cancel_state() {
        let first = RequestControl::new(Duration::from_secs(60)).unwrap();
        let second = RequestControl::new(Duration::from_secs(60)).unwrap();
        first.cancel();
        assert!(first.check().is_err());
        second.check().unwrap();
        assert!(second.remaining() <= Duration::from_secs(60));
    }

    #[test]
    fn restricted_deadline_never_extends_or_loses_parent_cancellation() {
        let parent = RequestControl::new(Duration::from_secs(60)).unwrap();
        let later = parent.deadline() + Duration::from_secs(60);
        assert_eq!(
            parent.restrict_deadline(later).deadline(),
            parent.deadline()
        );
        let child = parent.restrict_deadline(Instant::now());
        assert_eq!(child.check().unwrap_err().code, "TIMEOUT");
        parent.cancel();
        assert_eq!(child.check().unwrap_err().code, "CANCELLED");
    }

    #[test]
    fn cancellation_has_deterministic_precedence_over_expiry() {
        let control = RequestControl::from_deadline(Instant::now());
        control.cancel();
        assert_eq!(control.check().unwrap_err().code, "CANCELLED");
    }
}
