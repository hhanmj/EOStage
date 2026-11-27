use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use insar_core::{reduce_task_state, validate_task_state, TaskSelectionState, TaskStateCommand};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    initial: TaskSelectionState,
    valid_steps: Vec<Step>,
    cancel_scenario: Step,
    error_commands: Vec<ErrorStep>,
    invalid_states: Vec<TaskSelectionState>,
}

#[derive(Deserialize)]
struct Step {
    command: TaskStateCommand,
    expected: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct ErrorStep {
    command: TaskStateCommand,
    code: String,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_pure_logic/task_state.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_expected_fields(state: &TaskSelectionState, expected: &BTreeMap<String, Value>) {
    let serialized = serde_json::to_value(state).unwrap();
    for (key, expected_value) in expected {
        assert_eq!(&serialized[key], expected_value, "field: {key}");
    }
}

fn assert_core_invariants(state: &TaskSelectionState) {
    assert!(state.active_ids.is_disjoint(&state.paused_ids));
    assert!(state.active_ids.is_disjoint(&state.queued_ids));
    assert!(state.paused_ids.is_disjoint(&state.queued_ids));
    assert!(state.active_ids.len() <= state.concurrency);
    validate_task_state(state).unwrap();
}

#[test]
fn reducer_matches_handoff_fixture_and_keeps_frozen_selection() {
    let fixture = fixture();
    let original_candidates = fixture.initial.candidate_ids.clone();
    let original_selected = fixture.initial.selected_ids.clone();
    let mut state = fixture.initial;

    for step in fixture.valid_steps {
        state = reduce_task_state(&state, &step.command).unwrap();
        assert_expected_fields(&state, &step.expected);
        assert_eq!(state.candidate_ids, original_candidates);
        assert_eq!(state.selected_ids, original_selected);
        assert_core_invariants(&state);
    }
}

#[test]
fn reducer_cancels_only_requested_items() {
    let fixture = fixture();
    let next = reduce_task_state(&fixture.initial, &fixture.cancel_scenario.command).unwrap();

    assert_expected_fields(&next, &fixture.cancel_scenario.expected);
    assert_core_invariants(&next);
}

#[test]
fn reducer_rejects_unknown_items_and_invalid_input_states() {
    let fixture = fixture();
    for step in fixture.error_commands {
        let error = reduce_task_state(&fixture.initial, &step.command).unwrap_err();
        assert_eq!(error.code, step.code);
    }
    for state in fixture.invalid_states {
        let error = validate_task_state(&state).unwrap_err();
        assert_eq!(error.code, "V3_INVALID_TASK_STATE");
    }
}

#[test]
fn reducer_does_not_mutate_another_task_with_the_same_item_ids() {
    let fixture = fixture();
    let first = fixture.initial;
    let mut second = first.clone();
    second.task_id = "task-2".to_string();
    let before = second.clone();

    let next = reduce_task_state(
        &first,
        &TaskStateCommand::PauseItems {
            item_ids: BTreeSet::from(["a".to_string()]),
        },
    )
    .unwrap();

    assert!(next.paused_ids.contains("a"));
    assert_eq!(second, before);
}

#[test]
fn scheduler_claims_only_queued_items_within_capacity_and_retries_failures() {
    let state = TaskSelectionState {
        task_id: "task-scheduler".to_string(),
        candidate_ids: BTreeSet::from_iter(["a", "b", "c"].map(str::to_string)),
        selected_ids: BTreeSet::from_iter(["a", "b", "c"].map(str::to_string)),
        active_ids: BTreeSet::new(),
        paused_ids: BTreeSet::new(),
        queued_ids: BTreeSet::from_iter(["a", "b"].map(str::to_string)),
        completed_ids: BTreeSet::new(),
        failed_ids: BTreeSet::from(["c".to_string()]),
        skipped_ids: BTreeSet::new(),
        cancelled_ids: BTreeSet::new(),
        task_paused: false,
        concurrency: 2,
    };
    let active = reduce_task_state(
        &state,
        &TaskStateCommand::ActivateItems {
            item_ids: BTreeSet::from_iter(["a", "b"].map(str::to_string)),
        },
    )
    .unwrap();
    assert_eq!(
        active.active_ids,
        BTreeSet::from_iter(["a", "b"].map(str::to_string))
    );
    assert!(active.queued_ids.is_empty());

    let over_capacity = reduce_task_state(
        &active,
        &TaskStateCommand::RetryItems {
            item_ids: BTreeSet::from(["c".to_string()]),
        },
    )
    .unwrap();
    assert!(over_capacity.queued_ids.contains("c"));
    let error = reduce_task_state(
        &over_capacity,
        &TaskStateCommand::ActivateItems {
            item_ids: BTreeSet::from(["c".to_string()]),
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "V3_INVALID_TASK_STATE");
}
