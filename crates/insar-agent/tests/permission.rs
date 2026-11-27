//! The permission-mode gate (A1 design §3.1, Owner 2026-10-26) as a pure
//! decision: the three modes against the coordinator's matrix of tier 1/2
//! steps, and how stored W3e modes are read.
use insar_agent::permission::{
    decide, valid_output_dir, CardReason, OutputDir, PermissionMode, Step, Tier, Verdict,
    CONFIRMED_TOOLS, DISCLOSED_TOOLS, STEERING_COMMANDS,
};
use serde_json::json;

const MODES: [PermissionMode; 3] = [
    PermissionMode::ConfirmChanges,
    PermissionMode::AutoApprove,
    PermissionMode::FullAccess,
];

/// A create_task whose facts are complete: the default download directory,
/// sizes recorded, nothing already queued, accounts ready, batch frozen by
/// this run.
fn complete() -> Step {
    Step::CreateTask {
        output_dir: OutputDir::Known,
        size_known: true,
        reopens_existing_task: false,
        accounts_ready: true,
        selection_from_run: true,
    }
}
fn create_without(reason: CardReason) -> Step {
    let Step::CreateTask {
        mut output_dir,
        mut size_known,
        mut reopens_existing_task,
        mut accounts_ready,
        mut selection_from_run,
    } = complete()
    else {
        unreachable!()
    };
    match reason {
        CardReason::OutputDirUnknown => output_dir = OutputDir::Unknown,
        CardReason::OutputDirUnverified => output_dir = OutputDir::ModelSuggested,
        CardReason::SizeUnknown => size_known = false,
        CardReason::ReopensExistingTask => reopens_existing_task = true,
        CardReason::AccountNotReady => accounts_ready = false,
        CardReason::SelectionNotFromRun => selection_from_run = false,
        _ => unreachable!(),
    }
    Step::CreateTask {
        output_dir,
        size_known,
        reopens_existing_task,
        accounts_ready,
        selection_from_run,
    }
}
fn control(archive: bool, agent_task: bool) -> Step {
    Step::TaskControl {
        archive,
        agent_task,
        ledger_edit: false,
    }
}
fn card(reasons: Vec<CardReason>) -> Verdict {
    Verdict::Card { reasons }
}

#[test]
fn every_mode_keeps_tier_0_direct_and_refuses_the_forbidden_tier() {
    for mode in MODES {
        assert_eq!(decide(mode, Tier::Read, &[]), Verdict::Direct, "{mode:?}");
        assert_eq!(
            decide(mode, Tier::Forbidden, &[complete()]),
            Verdict::Refuse,
            "{mode:?}: the forbidden tier is refused in every mode"
        );
    }
    assert_eq!(Tier::of_tool("credential_save_cdse"), Tier::Forbidden);
    assert_eq!(Tier::of_tool("shell_exec"), Tier::Forbidden);
    assert_eq!(Tier::of_tool("task_overview"), Tier::Read);
    assert_eq!(Tier::of_tool("admin_boundary_search"), Tier::Read);
    assert_eq!(Tier::of_tool("search_start"), Tier::Disclose);
    assert_eq!(Tier::of_tool("working_aoi_set"), Tier::Disclose);
    assert_eq!(Tier::of_tool("create_task"), Tier::Confirm);
    assert_eq!(Tier::of_tool("task_action"), Tier::Confirm);
    assert_eq!(Tier::of_tool("update_download_task"), Tier::Confirm);
}

/// Matrix rows "search_start" and "working_aoi_set": automatic and disclosed in every mode.
#[test]
fn a_search_runs_without_a_click_in_every_mode() {
    for mode in MODES {
        assert_eq!(
            decide(mode, Tier::Disclose, &[Step::Search]),
            Verdict::Auto,
            "{mode:?}"
        );
        assert_eq!(
            decide(mode, Tier::Disclose, &[Step::WorkspaceAoi]),
            Verdict::Auto,
            "{mode:?}"
        );
        assert_eq!(
            decide(mode, Tier::Disclose, &[Step::WorkspaceAoi, Step::Search]),
            Verdict::Auto,
            "{mode:?}"
        );
    }
}

/// Matrix row "create_task, complete": card / auto / auto.
#[test]
fn a_complete_create_task_needs_the_card_only_under_confirm_changes() {
    assert_eq!(
        decide(PermissionMode::ConfirmChanges, Tier::Confirm, &[complete()]),
        card(vec![])
    );
    assert_eq!(
        decide(PermissionMode::AutoApprove, Tier::Confirm, &[complete()]),
        Verdict::Auto
    );
    assert_eq!(
        decide(PermissionMode::FullAccess, Tier::Confirm, &[complete()]),
        Verdict::Auto
    );
}

/// Matrix row "create_task, incomplete": card / card naming the reason /
/// auto (full access asks only for an unknown directory).
#[test]
fn an_incomplete_create_task_needs_the_card_under_auto_approve_naming_the_reason() {
    for reason in [
        CardReason::SizeUnknown,
        CardReason::ReopensExistingTask,
        CardReason::AccountNotReady,
        CardReason::SelectionNotFromRun,
    ] {
        let step = [create_without(reason)];
        assert_eq!(
            decide(PermissionMode::ConfirmChanges, Tier::Confirm, &step),
            card(vec![reason]),
            "{reason:?}"
        );
        assert_eq!(
            decide(PermissionMode::AutoApprove, Tier::Confirm, &step),
            card(vec![reason]),
            "{reason:?}"
        );
        assert_eq!(
            decide(PermissionMode::FullAccess, Tier::Confirm, &step),
            Verdict::Auto,
            "{reason:?}"
        );
    }
}

/// Matrix: "目录未知仍弹卡（任何模式）"; review ruling 2026-10-26: a
/// directory the model supplied is never known, so it is confirmed on the
/// card in every mode too.
#[test]
fn an_unknown_or_model_suggested_output_directory_needs_the_card_in_every_mode() {
    for reason in [
        CardReason::OutputDirUnknown,
        CardReason::OutputDirUnverified,
    ] {
        let step = [create_without(reason)];
        for mode in MODES {
            assert_eq!(
                decide(mode, Tier::Confirm, &step),
                card(vec![reason]),
                "{mode:?} {reason:?}"
            );
        }
    }
}

/// Review ruling 2026-10-26: update_download_task under auto_approve runs
/// without a click only for task- and item-level pause/resume/retry/cancel;
/// a ledger edit waits for the card; full access runs it.
#[test]
fn ledger_edits_need_the_card_under_auto_approve_and_run_under_full_access() {
    let edit = [Step::TaskControl {
        archive: false,
        agent_task: true,
        ledger_edit: true,
    }];
    assert_eq!(
        decide(PermissionMode::AutoApprove, Tier::Confirm, &edit),
        card(vec![CardReason::LedgerEditRequiresConfirm])
    );
    assert_eq!(
        decide(PermissionMode::ConfirmChanges, Tier::Confirm, &edit),
        card(vec![CardReason::LedgerEditRequiresConfirm])
    );
    assert_eq!(
        decide(PermissionMode::FullAccess, Tier::Confirm, &edit),
        Verdict::Auto
    );
    assert_eq!(
        STEERING_COMMANDS,
        [
            "pause_task",
            "resume_task",
            "retry_task",
            "cancel_task",
            "pause_items",
            "resume_items",
            "retry_items",
            "cancel_items"
        ]
    );
}

/// Review ruling 2026-10-26: a tier 1/2 tool the gate has no facts for waits
/// for the card in every mode, and the gate's tool lists are exactly the
/// catalog's disclose and plan_confirm tools.
#[test]
fn unmapped_tools_need_the_card_in_every_mode_and_the_mapping_covers_the_catalog() {
    for mode in MODES {
        for tier in [Tier::Disclose, Tier::Confirm] {
            assert_eq!(
                decide(mode, tier, &[Step::Unmapped]),
                card(vec![CardReason::ToolRequiresConfirm]),
                "{mode:?}"
            );
        }
    }
    let tools = insar_agent::catalog::catalog().tools;
    let of = |confirmation: &str| {
        let mut ids: Vec<String> = tools
            .iter()
            .filter(|tool| tool.confirmation == confirmation)
            .map(|tool| tool.tool_id.clone())
            .collect();
        ids.sort();
        ids
    };
    let sorted = |ids: &[&str]| {
        let mut ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
        ids.sort();
        ids
    };
    assert_eq!(of("disclose"), sorted(DISCLOSED_TOOLS));
    assert_eq!(of("plan_confirm"), sorted(CONFIRMED_TOOLS));
    assert!(tools.iter().all(|tool| matches!(
        tool.confirmation.as_str(),
        "none" | "disclose" | "plan_confirm"
    )));
}

/// Matrix row "task_action pause/resume/retry/cancel on an Agent task" (and
/// update_download_task on one): card / auto / auto.
#[test]
fn controls_of_agent_tasks_run_without_a_click_except_under_confirm_changes() {
    let step = [control(false, true)];
    assert_eq!(
        decide(PermissionMode::ConfirmChanges, Tier::Confirm, &step),
        card(vec![])
    );
    assert_eq!(
        decide(PermissionMode::AutoApprove, Tier::Confirm, &step),
        Verdict::Auto
    );
    assert_eq!(
        decide(PermissionMode::FullAccess, Tier::Confirm, &step),
        Verdict::Auto
    );
}

/// Matrix row "task_action archive": card / card / auto.
#[test]
fn archive_needs_the_card_under_auto_approve_and_runs_under_full_access() {
    let step = [control(true, true)];
    assert_eq!(
        decide(PermissionMode::AutoApprove, Tier::Confirm, &step),
        card(vec![CardReason::ArchiveRequiresConfirm])
    );
    assert_eq!(
        decide(PermissionMode::ConfirmChanges, Tier::Confirm, &step),
        card(vec![CardReason::ArchiveRequiresConfirm])
    );
    assert_eq!(
        decide(PermissionMode::FullAccess, Tier::Confirm, &step),
        Verdict::Auto
    );
}

/// Matrix row "task control of the user's own task": card / card / auto.
#[test]
fn controls_of_user_tasks_need_the_card_under_auto_approve_and_run_under_full_access() {
    let step = [control(false, false)];
    assert_eq!(
        decide(PermissionMode::AutoApprove, Tier::Confirm, &step),
        card(vec![CardReason::TargetsUserTask])
    );
    assert_eq!(
        decide(PermissionMode::ConfirmChanges, Tier::Confirm, &step),
        card(vec![CardReason::TargetsUserTask])
    );
    assert_eq!(
        decide(PermissionMode::FullAccess, Tier::Confirm, &step),
        Verdict::Auto
    );
    assert_eq!(
        decide(
            PermissionMode::AutoApprove,
            Tier::Confirm,
            &[control(true, false)]
        ),
        card(vec![
            CardReason::TargetsUserTask,
            CardReason::ArchiveRequiresConfirm
        ])
    );
}

#[test]
fn a_plan_waits_for_the_card_when_any_step_needs_it_and_names_each_reason_once() {
    let steps = [
        Step::Search,
        complete(),
        create_without(CardReason::SizeUnknown),
        create_without(CardReason::SizeUnknown),
        control(false, false),
    ];
    assert_eq!(
        decide(PermissionMode::AutoApprove, Tier::Confirm, &steps),
        card(vec![CardReason::SizeUnknown, CardReason::TargetsUserTask])
    );
    // Under full access the same facts do not need a click.
    assert_eq!(
        decide(PermissionMode::FullAccess, Tier::Confirm, &steps),
        Verdict::Auto
    );
    // A search next to a complete create_task follows the create_task.
    assert_eq!(
        decide(
            PermissionMode::AutoApprove,
            Tier::Confirm,
            &[Step::Search, complete()]
        ),
        Verdict::Auto
    );
    assert_eq!(
        decide(
            PermissionMode::ConfirmChanges,
            Tier::Confirm,
            &[Step::Search, complete()]
        ),
        card(vec![])
    );
}

#[test]
fn modes_are_three_plain_names_and_stored_w3e_modes_read_under_the_new_names() {
    assert_eq!(PermissionMode::default(), PermissionMode::ConfirmChanges);
    for (mode, name) in MODES
        .into_iter()
        .zip(["confirm_changes", "auto_approve", "full_access"])
    {
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(name));
        assert_eq!(mode.as_str(), name);
        assert_eq!(
            serde_json::from_value::<PermissionMode>(json!(name)).unwrap(),
            mode
        );
    }
    // The command payload takes the new names only.
    for old in [
        json!("per_item"),
        json!("within_quota"),
        json!({"kind": "full_access"}),
        json!({"kind": "within_quota", "max_bytes": 1, "max_tasks": 1, "output_dir": "D:/x"}),
    ] {
        assert!(
            serde_json::from_value::<PermissionMode>(old.clone()).is_err(),
            "{old}"
        );
    }
    // What W3e stored is read under the new names; its limits are dropped.
    for (stored, mode) in [
        (
            json!({"kind": "per_item"}),
            Some(PermissionMode::ConfirmChanges),
        ),
        (
            json!({"kind": "within_quota", "max_bytes": 20_000_000_000u64, "max_tasks": 10,
                "output_dir": "D:/data"}),
            Some(PermissionMode::AutoApprove),
        ),
        (
            json!({"kind": "full_access"}),
            Some(PermissionMode::FullAccess),
        ),
        (json!("auto_approve"), Some(PermissionMode::AutoApprove)),
        (json!({"kind": "auto"}), None),
        (json!(7), None),
    ] {
        assert_eq!(PermissionMode::from_stored(&stored), mode, "{stored}");
    }
}

/// Review ruling 2026-10-26: only local drive-letter paths on Windows.
#[cfg(windows)]
#[test]
fn an_output_directory_is_a_drive_letter_path_without_dot_segments() {
    for good in [r"C:\Data\out", "D:/data/out", r"e:\"] {
        assert!(valid_output_dir(good), "{good}");
    }
    for bad in [
        "",
        "relative/out",
        "/home/user/out",
        r"\\server\share\out",
        "//server/share/out",
        r"\\?\C:\Data\out",
        r"\\.\C:\Data",
        r"C:Data\out",
        r"C:\Data\..\escape",
        "C:/Data/./out",
        r" C:\Data",
        "C:\\Data\n",
    ] {
        assert!(!valid_output_dir(bad), "{bad:?}");
    }
}

/// The Unix counterpart of the 2026-10-26 ruling: only local absolute paths.
#[cfg(not(windows))]
#[test]
fn an_output_directory_is_an_absolute_unix_path_without_dot_segments() {
    for good in ["/home/user/out", "/data/数据/out", "/", "/development/out"] {
        assert!(valid_output_dir(good), "{good}");
    }
    for bad in [
        "",
        "relative/out",
        "~/out",
        r"C:\Data\out",
        "D:/data/out",
        "//server/share/out",
        r"/home/user\out",
        "/home/user/../escape",
        "/home/./out",
        " /home/user",
        "/home/user\n",
        "/dev/shm/out",
        "/proc/self/cwd",
        "/sys/kernel",
        "/dev",
    ] {
        assert!(!valid_output_dir(bad), "{bad:?}");
    }
}
