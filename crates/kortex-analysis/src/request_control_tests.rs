use super::*;
use std::time::{Duration, Instant};

#[test]
fn shared_control_preserves_normal_contracts_and_parent_cancellation_for_every_operation() {
    let (_temp, service) = indexed(&[(
        "a.py",
        "def target():\n    pass\ndef caller():\n    target()\n",
    )]);
    let requests = [
        ("status", json!({})),
        ("query", json!({"text":"target"})),
        ("files", json!({})),
        ("context", json!({"symbol":"target"})),
        ("impact", json!({"symbol":"target"})),
        ("trace", json!({"from":"caller","to":"target"})),
        ("graph", json!({"symbol":"target"})),
        ("routes", json!({})),
        (
            "check",
            json!({"rules":[{"id":"same","kind":"forbid_dependency","from":{"path":"a.py"},"to":{"path":"a.py"}}]}),
        ),
    ];
    for (op, args) in requests {
        let parent = RequestControl::new(Duration::from_secs(20)).unwrap();
        let mut expected = service.execute(op, &args).unwrap();
        let mut actual = service.execute_with_control(op, &args, &parent).unwrap();
        for output in [&mut expected, &mut actual] {
            output.as_object_mut().unwrap().remove("request_id");
            output.as_object_mut().unwrap().remove("timing_ms");
        }
        assert_eq!(actual, expected, "{op}");
        let restricted = parent.restrict_deadline(parent.deadline() + Duration::from_secs(60));
        assert_eq!(restricted.deadline(), parent.deadline());
        parent.cancel();
        assert_eq!(
            service
                .execute_with_control(op, &args, &restricted)
                .unwrap_err()
                .code,
            "CANCELLED",
            "{op}"
        );
        let expired = RequestControl::from_deadline(Instant::now());
        assert_eq!(
            service
                .execute_with_control(op, &args, &expired)
                .unwrap_err()
                .code,
            "TIMEOUT",
            "{op}"
        );
    }
}

#[test]
fn parent_short_deadline_is_not_restarted_by_freshness_ast_or_graph_work() {
    let source = (0..5000)
        .map(|n| format!("def node_{n}():\n    pass\n"))
        .collect::<String>();
    let (_temp, service) = indexed(&[("many.py", &source)]);
    let parent = RequestControl::from_deadline(Instant::now() + Duration::from_millis(1));
    let error = service
        .execute_with_control("routes", &json!({"timeout_ms":10000}), &parent)
        .unwrap_err();
    assert_eq!((error.code, error.exit), ("TIMEOUT", 9));
    let active = RequestControl::new(Duration::from_secs(10)).unwrap();
    assert!(service.freshness_with_control(&active).is_ok());
    active.cancel();
    assert_eq!(
        service.freshness_with_control(&active).unwrap_err().code,
        "CANCELLED"
    );
}
