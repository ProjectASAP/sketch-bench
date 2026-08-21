use super::*;

// ---------- the row's closures, in the framework's terms ----------
//
// A wrapper hands back closures over a sketch and nothing else: how many units
// of work one pass covers, and what it answered, are read here.

/// A pass that only does work: its unit count is the work it covers, which the
/// row knows because it handed the stream over.
pub(super) fn timed(passes: Vec<sketch_bench::wrappers::Pass>, work: u64) -> Measurement {
    passes
        .into_iter()
        .map(|pass| {
            Box::new(move || {
                let footprint = pass();
                Box::new(move || RunOutcome {
                    work,
                    memory_bytes: Some(footprint as u64),
                    ..Default::default()
                }) as Report
            }) as CorePass
        })
        .collect()
}

/// A pass driven one unit at a time, so the clock can be read per call.
pub(super) fn stepped(passes: Vec<StepPass>) -> Measurement {
    passes
        .into_iter()
        .map(|pass| {
            let StepPass {
                steps,
                step,
                footprint,
            } = pass;
            Box::new(move || {
                let latency_ns = Some(record_calls(steps, step));
                Box::new(move || RunOutcome {
                    work: steps as u64,
                    memory_bytes: Some(footprint() as u64),
                    latency_ns,
                    ..Default::default()
                }) as Report
            }) as CorePass
        })
        .collect()
}

/// A pass that answers: the comparator scores what it said, here, once the
/// clock has stopped.
pub(super) fn answered<A: 'static>(
    passes: Vec<QueryPass<A>>,
    score: Score<A>,
    metric: Metric,
) -> Measurement {
    passes
        .into_iter()
        .map(|pass| {
            let score = score.clone();
            Box::new(move || {
                let (answers, footprint) = pass();
                Box::new(move || RunOutcome {
                    work: answers.len() as u64,
                    memory_bytes: Some(footprint as u64),
                    scores: if metric == Metric::Accuracy {
                        score(&answers)
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                }) as Report
            }) as CorePass
        })
        .collect()
}
