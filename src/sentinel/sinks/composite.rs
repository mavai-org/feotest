//! Composite verdict sink: fans every lifecycle call out to a list of
//! wrapped sinks and isolates their failures.

use crate::sentinel::sinks::{SinkError, VerdictSink};
use crate::verdict::VerdictRecord;

/// Wraps a collection of sinks and dispatches every lifecycle call to
/// each one in turn.
///
/// If a wrapped sink returns a [`SinkError`] from any lifecycle method,
/// the composite logs the error to stderr and continues with the
/// remaining sinks. This is the failure-isolation contract the SN03
/// specification calls for: a webhook endpoint going offline must not
/// stop verdicts reaching the console or the on-disk audit log.
///
/// The composite itself always returns `Ok(())` from every lifecycle
/// method. A sink that genuinely fails its own lifecycle is visible to
/// the operator via the stderr log — not through an error returned
/// upstream.
#[derive(Default)]
pub struct CompositeVerdictSink {
    sinks: Vec<Box<dyn VerdictSink>>,
}

impl CompositeVerdictSink {
    /// Creates an empty composite sink.
    #[must_use]
    pub const fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    /// Appends a sink to the composite.
    #[must_use]
    pub fn push(mut self, sink: Box<dyn VerdictSink>) -> Self {
        self.sinks.push(sink);
        self
    }

    /// Number of wrapped sinks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// Whether the composite wraps no sinks.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

fn log_sink_error(phase: &str, index: usize, err: &SinkError) {
    eprintln!("verdict sink #{index} failed during {phase}: {err}");
}

impl VerdictSink for CompositeVerdictSink {
    fn initialize(&mut self) -> Result<(), SinkError> {
        for (index, sink) in self.sinks.iter_mut().enumerate() {
            if let Err(err) = sink.initialize() {
                log_sink_error("initialize", index, &err);
            }
        }
        Ok(())
    }

    fn accept(&mut self, verdict: &VerdictRecord) -> Result<(), SinkError> {
        for (index, sink) in self.sinks.iter_mut().enumerate() {
            if let Err(err) = sink.accept(verdict) {
                log_sink_error("accept", index, &err);
            }
        }
        Ok(())
    }

    fn finalize(&mut self) -> Result<(), SinkError> {
        for (index, sink) in self.sinks.iter_mut().enumerate() {
            if let Err(err) = sink.finalize() {
                log_sink_error("finalize", index, &err);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity, TestIntent,
    };
    use crate::verdict::{FunctionalDimension, Verdict, VerdictRecord};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct CountingSink {
        initializes: Arc<AtomicUsize>,
        accepts: Arc<AtomicUsize>,
        finalizes: Arc<AtomicUsize>,
    }

    impl VerdictSink for CountingSink {
        fn initialize(&mut self) -> Result<(), SinkError> {
            self.initializes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn accept(&mut self, _verdict: &VerdictRecord) -> Result<(), SinkError> {
            self.accepts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn finalize(&mut self) -> Result<(), SinkError> {
            self.finalizes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FailingSink;
    impl VerdictSink for FailingSink {
        fn initialize(&mut self) -> Result<(), SinkError> {
            Err(SinkError::InitialisationFailed("boom".into()))
        }
        fn accept(&mut self, _verdict: &VerdictRecord) -> Result<(), SinkError> {
            Err(SinkError::DeliveryFailed("boom".into()))
        }
        fn finalize(&mut self) -> Result<(), SinkError> {
            Err(SinkError::FinalisationFailed("boom".into()))
        }
    }

    fn sample_verdict() -> VerdictRecord {
        let exec = ExecutionSummary::new(
            10,
            10,
            10,
            0,
            TerminationInfo::new(TerminationReason::Completed),
            CostSummary::new(Duration::from_millis(10), 100, 10),
        );
        VerdictRecord::builder(
            TestIdentity::new("composite-test"),
            Verdict::Pass,
            TestIntent::Verification,
            exec,
            FunctionalDimension::new(10, 0, vec![]),
        )
        .build()
    }

    #[test]
    fn failing_sink_does_not_prevent_success_sink_from_receiving_verdicts() {
        let accepts = Arc::new(AtomicUsize::new(0));
        let ok_sink = CountingSink {
            accepts: Arc::clone(&accepts),
            ..Default::default()
        };

        let mut composite = CompositeVerdictSink::new()
            .push(Box::new(FailingSink))
            .push(Box::new(ok_sink));

        composite.initialize().expect("initialize is infallible");
        composite
            .accept(&sample_verdict())
            .expect("accept infallible");
        composite
            .accept(&sample_verdict())
            .expect("accept infallible");
        composite.finalize().expect("finalize infallible");

        assert_eq!(
            accepts.load(Ordering::SeqCst),
            2,
            "successful sink should still see every verdict"
        );
    }

    #[test]
    fn lifecycle_calls_reach_every_sink_in_order() {
        let inits_a = Arc::new(AtomicUsize::new(0));
        let inits_b = Arc::new(AtomicUsize::new(0));
        let accepts_a = Arc::new(AtomicUsize::new(0));
        let accepts_b = Arc::new(AtomicUsize::new(0));
        let finalizes_a = Arc::new(AtomicUsize::new(0));
        let finalizes_b = Arc::new(AtomicUsize::new(0));

        let sink_a = CountingSink {
            initializes: Arc::clone(&inits_a),
            accepts: Arc::clone(&accepts_a),
            finalizes: Arc::clone(&finalizes_a),
        };
        let sink_b = CountingSink {
            initializes: Arc::clone(&inits_b),
            accepts: Arc::clone(&accepts_b),
            finalizes: Arc::clone(&finalizes_b),
        };

        let mut composite = CompositeVerdictSink::new()
            .push(Box::new(sink_a))
            .push(Box::new(sink_b));

        composite.initialize().unwrap();
        composite.accept(&sample_verdict()).unwrap();
        composite.finalize().unwrap();

        assert_eq!(inits_a.load(Ordering::SeqCst), 1);
        assert_eq!(inits_b.load(Ordering::SeqCst), 1);
        assert_eq!(accepts_a.load(Ordering::SeqCst), 1);
        assert_eq!(accepts_b.load(Ordering::SeqCst), 1);
        assert_eq!(finalizes_a.load(Ordering::SeqCst), 1);
        assert_eq!(finalizes_b.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_composite_is_a_harmless_no_op() {
        let mut composite = CompositeVerdictSink::new();
        assert!(composite.is_empty());
        assert_eq!(composite.len(), 0);
        composite.initialize().unwrap();
        composite.accept(&sample_verdict()).unwrap();
        composite.finalize().unwrap();
    }
}
