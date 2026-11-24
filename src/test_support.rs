use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// A full tree of `fanout` children per node down to `depth` levels below the
/// root, shaped like `crossplane beta trace -o json` output. `extras` adds the
/// uid, external-name annotation and composition revision reference that the
/// app-level benchmarks need.
pub(crate) fn synthetic_trace(depth: usize, fanout: usize, extras: bool) -> Value {
    fn node(counter: &mut usize, level: usize, depth: usize, fanout: usize, extras: bool) -> Value {
        let id = *counter;
        *counter += 1;
        let kind = id % 12;
        let children: Vec<Value> = if level < depth {
            (0..fanout)
                .map(|_| node(counter, level + 1, depth, fanout, extras))
                .collect()
        } else {
            Vec::new()
        };
        let name = if id.is_multiple_of(100) {
            format!("hit-{id}")
        } else {
            format!("res-{id}")
        };
        let mut object = json!({
            "apiVersion": "example.io/v1",
            "kind": format!("Kind{kind}"),
            "metadata": {"name": name, "namespace": format!("ns-{}", id % 50)},
            "status": {"conditions": [
                {
                    "type": "Ready",
                    "status": if id.is_multiple_of(2) { "False" } else { "True" },
                    "reason": "Waiting"
                },
                {"type": "Synced", "status": "True"}
            ]}
        });
        if extras {
            object["metadata"]["uid"] = json!(format!("uid-{id}"));
            object["metadata"]["annotations"] =
                json!({"crossplane.io/external-name": format!("external-{id}")});
            object["spec"] = json!({
                "compositionRef": {"name": format!("comp-{kind}")},
                "compositionUpdatePolicy": "Manual",
                "compositionRevisionRef": {"name": format!("comp-{kind}-1")},
            });
        }
        json!({"object": object, "children": children})
    }
    node(&mut 0, 0, depth, fanout, extras)
}

/// Runs `run` `iterations` times, prints the average and first duration, and
/// returns the average.
pub(crate) fn bench_avg(label: &str, iterations: u32, mut run: impl FnMut()) -> Duration {
    let mut first = Duration::ZERO;
    let mut total = Duration::ZERO;
    for iteration in 0..iterations {
        let started = Instant::now();
        run();
        let elapsed = started.elapsed();
        if iteration == 0 {
            first = elapsed;
        }
        total += elapsed;
    }
    let average = total / iterations;
    println!("{label}: {average:?}/run avg, first {first:?}");
    average
}
