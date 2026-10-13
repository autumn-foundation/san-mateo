//! The workflows with the harvest test tools (AC21).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autumn_harvest::WorkflowEvent;
use autumn_harvest::failure::{ActivityFailure, IntoActivityErrorString};
use autumn_harvest::testing::{ReplayStatus, TestRunOutcome, WorkflowTestEnv};
use autumn_harvest::{SimulatorResult, WorkflowSimulator};
use common::*;
use san_mateo::transport::HttpResponse;
use san_mateo::{RecordsRequest, SanMateo};
use san_mateo_harvest::{
    RefreshDueInput, RefreshDueOutput, RunSyncInput, SyncLoopInput, SyncLoopOutput,
    SyncRunOutput, san_mateo_refresh_sweep_info, san_mateo_sync_loop_info,
};
use serde_json::{Value, json};

const RUN_SYNC: &str = "san_mateo_run_sync";
const REFRESH_DUE: &str = "san_mateo_refresh_due";

fn loop_input(frequency_secs: u64, iterations: u32) -> SyncLoopInput {
    SyncLoopInput::new(
        ENV,
        &crm(),
        "contacts",
        Duration::from_secs(frequency_secs),
    )
    .iterations_before_continue(iterations)
}

fn ok_run(added: u64) -> Value {
    json!({
        "sync_name": "contacts",
        "provider_config_key": "crm",
        "connection_id": "c-1",
        "success": true,
        "counts": {"added": added, "updated": 0, "deleted": 0}
    })
}

fn failed(error_type: &str, retryable: bool) -> String {
    let f = if retryable {
        ActivityFailure::retryable(error_type, "the run failed")
    } else {
        ActivityFailure::non_retryable(error_type, "the run failed")
    };
    f.into_error_payload()
}

fn scheduled(outcome: &TestRunOutcome, name: &str) -> usize {
    outcome
        .events()
        .iter()
        .filter(|e| matches!(e, WorkflowEvent::ActivityScheduled { name: n, .. } if n == name))
        .count()
}

fn timers(outcome: &TestRunOutcome) -> Vec<u64> {
    outcome
        .events()
        .iter()
        .filter_map(|e| match e {
            WorkflowEvent::TimerStarted { duration_secs, .. } => Some(*duration_secs),
            _ => None,
        })
        .collect()
}

fn continued(outcome: &TestRunOutcome) -> Option<SyncLoopInput> {
    outcome.events().iter().find_map(|e| match e {
        WorkflowEvent::WorkflowContinuedAsNew { input, .. } => {
            Some(serde_json::from_value(input.clone()).unwrap())
        }
        _ => None,
    })
}

async fn assert_replays(outcome: &TestRunOutcome, handler: autumn_harvest::WorkflowHandlerFn) {
    let report = outcome.replay_check(handler).await;
    assert!(
        matches!(report.status, ReplayStatus::ReplaySucceeded),
        "{report}"
    );
}

// ------------------------------------------------------------- sync loop

#[tokio::test]
async fn sync_loop_runs_waits_and_continues_as_new() {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let seen = inputs.clone();
    let handler = san_mateo_sync_loop_info().handler;
    let outcome = WorkflowTestEnv::new()
        .mock_activity(RUN_SYNC, move |input| {
            seen.lock().unwrap().push(input);
            Ok(ok_run(2))
        })
        .run(handler, serde_json::to_value(loop_input(300, 3)).unwrap())
        .await;
    let next: SyncLoopInput = serde_json::from_value(outcome.result.clone().unwrap()).unwrap();
    assert_eq!(continued(&outcome), Some(next.clone()));
    assert_eq!(next.total_runs, 3);
    assert_eq!(next.total_failures, 0);
    assert_eq!(next.sync_key, loop_input(300, 3).sync_key);
    assert_eq!(next.iterations_before_continue, Some(3));
    assert_eq!(next.frequency_secs, 300);
    assert_eq!(scheduled(&outcome, RUN_SYNC), 3);
    assert_eq!(timers(&outcome), vec![300, 300, 300]);
    assert_eq!(outcome.elapsed().num_seconds(), 900);
    assert!(matches!(
        outcome.events().last(),
        Some(WorkflowEvent::WorkflowContinuedAsNew { .. })
    ));
    // Each run gets the ids only.
    let inputs = inputs.lock().unwrap().clone();
    assert_eq!(inputs.len(), 3);
    for input in inputs {
        let run: RunSyncInput = serde_json::from_value(input).unwrap();
        assert_eq!(run, RunSyncInput::new(ENV, &crm(), "contacts"));
    }
    assert_replays(&outcome, handler).await;
}

#[tokio::test]
async fn a_short_frequency_becomes_the_minimum() {
    let outcome = WorkflowTestEnv::new()
        .mock_activity(RUN_SYNC, |_| Ok(ok_run(0)))
        .run(
            san_mateo_sync_loop_info().handler,
            serde_json::to_value(loop_input(5, 2)).unwrap(),
        )
        .await;
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    assert_eq!(timers(&outcome), vec![30, 30]);
}

#[tokio::test]
async fn the_default_iteration_count_is_used() {
    let mut input = loop_input(60, 1);
    input.iterations_before_continue = None;
    let outcome = WorkflowTestEnv::new()
        .mock_activity(RUN_SYNC, |_| Ok(ok_run(0)))
        .run(
            san_mateo_sync_loop_info().handler,
            serde_json::to_value(input).unwrap(),
        )
        .await;
    let next = continued(&outcome).unwrap();
    assert_eq!(
        next.total_runs,
        u64::from(san_mateo_harvest::workflows::DEFAULT_ITERATIONS_BEFORE_CONTINUE)
    );
    assert_eq!(next.iterations_before_continue, None);
}

// `mock_activity` records each mock failure with the error type `Error`.
// `mock_activity_retries` decodes typed failures, as the worker does. Each
// sequence is one activity call, and each item is one attempt.

#[tokio::test]
async fn failed_runs_do_not_stop_the_loop() {
    let handler = san_mateo_sync_loop_info().handler;
    let mut input = loop_input(60, 3);
    input.total_runs = 10;
    input.total_failures = 4;
    let sync_failed = || Err(failed(san_mateo_harvest::SYNC_FAILED_ERROR, true));
    let outcome = WorkflowTestEnv::new()
        // Three attempts fail, so the call fails.
        .mock_activity_retries(RUN_SYNC, vec![sync_failed(), sync_failed(), sync_failed()])
        // A non-retryable failure ends the call at once.
        .mock_activity_retries(RUN_SYNC, vec![Err(failed("san_mateo.conflict", false))])
        // A retry passes, so the call succeeds.
        .mock_activity_retries(RUN_SYNC, vec![sync_failed(), Ok(ok_run(1))])
        .run(handler, serde_json::to_value(input).unwrap())
        .await;
    let next = continued(&outcome).unwrap();
    assert_eq!(next.total_runs, 11);
    assert_eq!(next.total_failures, 6);
    assert_eq!(timers(&outcome).len(), 3);
    let failed_types: Vec<&str> = outcome
        .events()
        .iter()
        .filter_map(|e| match e {
            WorkflowEvent::ActivityFailed { error_type, .. } => Some(error_type.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        failed_types,
        [san_mateo_harvest::SYNC_FAILED_ERROR, "san_mateo.conflict"]
    );
    assert_replays(&outcome, handler).await;
}

#[tokio::test]
async fn a_gone_connection_stops_the_loop() {
    let handler = san_mateo_sync_loop_info().handler;
    let outcome = WorkflowTestEnv::new()
        .mock_activity_retries(RUN_SYNC, vec![Ok(ok_run(1))])
        .mock_activity_retries(
            RUN_SYNC,
            vec![Err(failed("san_mateo.unknown_connection", false))],
        )
        .run(handler, serde_json::to_value(loop_input(60, 5)).unwrap())
        .await;
    let out: SyncLoopOutput = serde_json::from_value(outcome.result.clone().unwrap()).unwrap();
    assert_eq!(
        out,
        SyncLoopOutput {
            total_runs: 1,
            total_failures: 1,
            stopped_by: Some("san_mateo.unknown_connection".into()),
        }
    );
    assert!(continued(&outcome).is_none());
    assert_eq!(scheduled(&outcome, RUN_SYNC), 2);
    assert_eq!(timers(&outcome), vec![60]);
    assert_replays(&outcome, handler).await;
}

#[tokio::test]
async fn an_input_with_no_sync_key_gets_one() {
    let mut input = serde_json::to_value(loop_input(60, 1)).unwrap();
    input.as_object_mut().unwrap().remove("sync_key");
    let outcome = WorkflowTestEnv::new()
        .mock_activity(RUN_SYNC, |_| Ok(ok_run(0)))
        .run(san_mateo_sync_loop_info().handler, input)
        .await;
    let next = continued(&outcome).unwrap();
    assert_eq!(next.sync_key, loop_input(60, 1).sync_key);
}

#[tokio::test]
async fn an_input_with_an_empty_id_fails_with_no_run() {
    for field in ["environment", "provider_config_key", "connection_id", "sync_name"] {
        let mut input = serde_json::to_value(loop_input(60, 1)).unwrap();
        input[field] = json!("");
        let outcome = WorkflowTestEnv::new()
            .mock_activity(RUN_SYNC, |_| Ok(ok_run(0)))
            .run(san_mateo_sync_loop_info().handler, input)
            .await;
        assert!(outcome.result.is_err(), "{field}");
        assert_eq!(scheduled(&outcome, RUN_SYNC), 0);
        let error_type = outcome.events().iter().find_map(|e| match e {
            WorkflowEvent::WorkflowFailed { error_type, .. } => error_type.clone(),
            _ => None,
        });
        assert_eq!(
            error_type.as_deref(),
            Some(san_mateo_harvest::INVALID_INPUT_ERROR),
            "{field}"
        );
    }
}

#[tokio::test]
async fn a_harvest_deadline_makes_the_loop_continue_early() {
    // With a 100 s run budget, harvest asks for `continue_as_new` after 80 s.
    let handler = san_mateo_sync_loop_info().handler;
    let outcome = WorkflowTestEnv::new()
        .with_execution_timeout(autumn_harvest::chrono::Duration::seconds(100))
        .mock_activity(RUN_SYNC, |_| Ok(ok_run(0)))
        .run(handler, serde_json::to_value(loop_input(30, 100)).unwrap())
        .await;
    let next = continued(&outcome).unwrap();
    assert_eq!(next.total_runs, 3);
    assert_eq!(timers(&outcome), vec![30, 30, 30]);
    assert_replays(&outcome, handler).await;
}

#[tokio::test]
async fn a_cancelled_loop_fails_as_a_workflow_failure() {
    let outcome = WorkflowTestEnv::new()
        .with_cancellation("operator stop")
        .mock_activity(RUN_SYNC, |_| Ok(ok_run(0)))
        .run(
            san_mateo_sync_loop_info().handler,
            serde_json::to_value(loop_input(60, 3)).unwrap(),
        )
        .await;
    assert!(outcome.result.is_err(), "{:?}", outcome.result);
    assert!(continued(&outcome).is_none());
}

// --------------------------------------------------------- refresh sweep

#[tokio::test]
async fn refresh_sweep_runs_one_sweep_with_defaults() {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let seen = inputs.clone();
    let handler = san_mateo_refresh_sweep_info().handler;
    let outcome = WorkflowTestEnv::new()
        .mock_activity(REFRESH_DUE, move |input| {
            seen.lock().unwrap().push(input);
            Ok(json!({"refreshed": 3, "failed": 1, "skipped": 2}))
        })
        .run(handler, Value::Null)
        .await;
    let out: RefreshDueOutput = serde_json::from_value(outcome.result.clone().unwrap()).unwrap();
    assert_eq!(
        out,
        RefreshDueOutput {
            refreshed: 3,
            failed: 1,
            skipped: 2,
        }
    );
    let inputs = inputs.lock().unwrap().clone();
    assert_eq!(
        inputs,
        vec![serde_json::to_value(RefreshDueInput::default()).unwrap()]
    );
    assert_eq!(scheduled(&outcome, REFRESH_DUE), 1);
    assert_replays(&outcome, handler).await;

    let outcome = WorkflowTestEnv::new()
        .mock_activity(REFRESH_DUE, |input| {
            assert_eq!(input, json!({"within_secs": 60, "limit": 100}));
            Ok(json!({"refreshed": 0, "failed": 0, "skipped": 0}))
        })
        .run(handler, json!({"within_secs": 60}))
        .await;
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
}

#[tokio::test]
async fn a_failed_sweep_fails_the_workflow_with_the_activity_error_type() {
    let outcome = WorkflowTestEnv::new()
        .mock_activity_retries(REFRESH_DUE, vec![Err(failed("san_mateo.store_error", true))])
        .run(san_mateo_refresh_sweep_info().handler, json!({}))
        .await;
    assert!(outcome.result.is_err());
    let error_type = outcome.events().iter().find_map(|e| match e {
        WorkflowEvent::WorkflowFailed { error_type, .. } => error_type.clone(),
        _ => None,
    });
    assert_eq!(error_type.as_deref(), Some("san_mateo.store_error"));
}

// ------------------------------------------------------------ end to end

/// Runs an activity body on the test runtime from a synchronous mock.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

/// A mock that runs the real `san_mateo_run_sync` work on a San Mateo service.
fn real_run_sync(san: SanMateo) -> impl Fn(Value) -> Result<Value, String> + Send + Sync {
    move |input| {
        let input: RunSyncInput = serde_json::from_value(input).map_err(|e| e.to_string())?;
        block_on(input.run(&san))
            .map(|out: SyncRunOutput| serde_json::to_value(out).unwrap())
            .map_err(IntoActivityErrorString::into_error_payload)
    }
}

fn simulator_continued(result: &SimulatorResult) -> SyncLoopInput {
    serde_json::from_value(result.final_output.clone().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sync_loop_runs_a_real_sync_end_to_end() {
    let h = ready().await;
    h.http.reply(
        "GET",
        &format!("{CRM}/v3/contacts"),
        HttpResponse::json(200, &json!([{"id": "1"}, {"id": "2"}, {"id": "3"}])),
    );
    let result = WorkflowSimulator::new(san_mateo_sync_loop_info().handler)
        .with_activity_info(&san_mateo_harvest::san_mateo_run_sync_info())
        .mock_activity(RUN_SYNC, real_run_sync(h.san.clone()))
        .run(serde_json::to_value(loop_input(600, 2)).unwrap())
        .await;
    let next = simulator_continued(&result);
    assert_eq!(next.total_runs, 2);
    assert_eq!(next.total_failures, 0);
    assert_eq!(h.contacts_runs.load(Ordering::SeqCst), 2);
    let page = h
        .san
        .list_records(
            ENV,
            &crm(),
            RecordsRequest {
                model: "Contact".into(),
                ..RecordsRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.records.len(), 3);
    // The history holds ids and counts only.
    let history = serde_json::to_string(&result.history).unwrap();
    assert_no_secrets(&history);
    assert!(history.contains(r#""added":3"#), "{history}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_real_sync_is_retried_by_the_activity_policy() {
    let h = ready().await;
    // Each attempt runs the real `broken` sync, which always fails.
    let result = WorkflowSimulator::new(san_mateo_sync_loop_info().handler)
        .with_activity_info(&san_mateo_harvest::san_mateo_run_sync_info())
        .mock_activity(RUN_SYNC, {
            let run = real_run_sync(h.san.clone());
            move |mut input| {
                input["sync_name"] = json!("broken");
                run(input)
            }
        })
        .run(serde_json::to_value(loop_input(600, 1)).unwrap())
        .await;
    let next = simulator_continued(&result);
    assert_eq!(next.total_runs, 0);
    assert_eq!(next.total_failures, 1);
    let attempts: Vec<(u32, &str)> = result
        .history
        .iter()
        .filter_map(|e| match e {
            WorkflowEvent::ActivityFailed {
                attempt,
                error_type,
                ..
            } => Some((*attempt, error_type.as_str())),
            _ => None,
        })
        .collect();
    let max = san_mateo_harvest::san_mateo_run_sync_info()
        .default_retry_policy
        .unwrap()
        .max_attempts;
    assert_eq!(attempts.len(), usize::try_from(max).unwrap());
    assert!(
        attempts
            .iter()
            .all(|(_, t)| *t == san_mateo_harvest::SYNC_FAILED_ERROR)
    );
    assert_no_secrets(&serde_json::to_string(&result.history).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sync_loop_stops_end_to_end_when_the_connection_is_gone() {
    let h = ready().await;
    h.san.delete_connection(ENV, &crm()).await.unwrap();
    let result = WorkflowSimulator::new(san_mateo_sync_loop_info().handler)
        .with_activity_info(&san_mateo_harvest::san_mateo_run_sync_info())
        .mock_activity(RUN_SYNC, real_run_sync(h.san.clone()))
        .run(serde_json::to_value(loop_input(600, 2)).unwrap())
        .await;
    let out: SyncLoopOutput = serde_json::from_value(result.final_output.unwrap()).unwrap();
    assert_eq!(out.stopped_by.as_deref(), Some("san_mateo.unknown_connection"));
    assert_eq!(out.total_runs, 0);
    assert_eq!(out.total_failures, 1);
    // A non-retryable failure gets one attempt only.
    let failures = result
        .history
        .iter()
        .filter(|e| matches!(e, WorkflowEvent::ActivityFailed { .. }))
        .count();
    assert_eq!(failures, 1);
}
