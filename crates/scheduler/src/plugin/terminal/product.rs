/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

use super::*;
use crate::{
    plan::Plan,
    schedule::{Recurrence, Schedule},
    task::{Create, Creator, Effect, Notification},
};
mod history;
use serde_json::json;

fn plan() -> Plan {
    Plan::create(
        "task".into(),
        Create {
            title: "Review".into(),
            intent_body: "Inspect".into(),
            schedule: Schedule::Interval {
                every_seconds: 60,
                start_at: 100_123,
            },
            effect: Effect::Notify(Notification::Local),
            max_fires: None,
            expires_at: None,
        },
        Creator::User,
        "UTC".into(),
        1000,
    )
    .unwrap()
}
fn values(fields: Vec<Field>) -> BTreeMap<String, Value> {
    fields
        .into_iter()
        .map(|field| {
            (
                field.id,
                match field.control {
                    Control::Text { value, .. } | Control::Choice { value, .. } => json!(value),
                    Control::Toggle { value } => json!(value),
                },
            )
        })
        .collect()
}

#[test]
fn schedule_type_limits_and_explicit_snooze_apply_to_the_actual_plan() {
    let original = plan();
    for kind in ["once", "interval", "daily", "weekly", "monthly", "cron"] {
        let mut fields = values(timing::fields(&original.task.schedule, "UTC", "zh-TW").unwrap());
        fields.insert("schedule".into(), json!(kind));
        let schedule = timing::update(&original.task.schedule, "UTC", fields).unwrap();
        let changed = original
            .update(
                Update {
                    schedule: Some(schedule.clone()),
                    ..Default::default()
                },
                1000,
            )
            .unwrap();
        assert_eq!(changed.task.schedule, schedule);
        assert!(
            changed
                .task
                .next_fire_at
                .is_some_and(|next| next >= 100_123)
        );
        if kind != "cron" {
            assert_eq!(changed.task.next_fire_at, Some(100_123));
        }
        if kind == "weekly" {
            assert!(matches!(
                schedule,
                Schedule::Calendar {
                    recurrence: Recurrence::Weekly,
                    ..
                }
            ));
        }
    }
    let mut constrained = original.clone();
    constrained.task.expires_at = Some(3_600_123);
    let mut fields =
        values(settings::limit_fields(None, constrained.task.expires_at, "UTC").unwrap());
    fields.insert("max_fires".into(), json!("3"));
    let Mutation::Update { patch, .. } = settings::mutation(
        settings::Kind::Limits,
        &constrained.task,
        "UTC",
        "save_limits",
        fields,
    )
    .unwrap() else {
        panic!("update")
    };
    let constrained = constrained.update(patch, 1000).unwrap();
    assert_eq!(constrained.task.max_fires, Some(3));
    assert_eq!(
        constrained.task.expires_at,
        Some(3_600_123),
        "unchanged date keeps milliseconds"
    );
    let fields = BTreeMap::from([
        ("max_fires".into(), json!("")),
        ("expires".into(), json!("")),
    ]);
    let Mutation::Update { patch, .. } = settings::mutation(
        settings::Kind::Limits,
        &constrained.task,
        "UTC",
        "save_limits",
        fields,
    )
    .unwrap() else {
        panic!("update")
    };
    let cleared = constrained.update(patch, 1000).unwrap();
    assert_eq!(
        (cleared.task.max_fires, cleared.task.expires_at),
        (None, None)
    );
    for (unit, amount, delay) in [
        ("milliseconds", "1", 1),
        ("seconds", "17", 17_000),
        ("days", "7", 604_800_000),
    ] {
        let fields = BTreeMap::from([
            ("delay".into(), json!(amount)),
            ("unit".into(), json!(unit)),
        ]);
        let Mutation::Snooze { delay_ms, .. } = settings::mutation(
            settings::Kind::Snooze,
            &original.task,
            "UTC",
            "snooze",
            fields,
        )
        .unwrap() else {
            panic!("snooze")
        };
        assert_eq!(delay_ms, delay);
        assert_eq!(
            original.snooze(delay_ms, 1000).unwrap().task.next_fire_at,
            Some(100_123 + delay)
        );
    }
    for (amount, unit) in [
        ("0", "minutes"),
        ("8", "days"),
        ("1", "unknown"),
        ("9223372036854775807", "days"),
    ] {
        assert!(
            settings::mutation(
                settings::Kind::Snooze,
                &original.task,
                "UTC",
                "snooze",
                BTreeMap::from([
                    ("delay".into(), json!(amount)),
                    ("unit".into(), json!(unit))
                ])
            )
            .is_err()
        );
    }
    for locale in ["en", "zh-CN", "zh-TW"] {
        for kind in [settings::Kind::Limits, settings::Kind::Snooze] {
            settings::page(kind, &original.task, 1, "UTC", locale)
                .unwrap()
                .view(locale)
                .validate()
                .unwrap();
        }
    }
}

#[test]
fn selected_session_captures_model_workspace_permissions_and_exact_consent_target() {
    let session: maka_plugins::session::View = serde_json::from_value(json!({
        "sessionId":"session-original", "revision":7, "name":"Model session", "boundaryRevision":3,
        "workspace":{"target":{"kind":"project","projectId":"project-original"},"hostCwd":"/resolved/project"},
        "target":{"kind":"model","model":{"connection_id":"connection-original","connection_slug":"provider","model":"model-original"},"thinkingLevel":"high"},
        "sandboxMode":"read-only", "approvalPolicy":{"kind":"on-request"}, "collaborationMode":"plan", "behavior":"default", "boundTools":["Read","Glob"]
    })).unwrap();
    let effect = target::from_session(target::Selection::Run, &session).unwrap();
    let Effect::AgentRun { execution } = &effect else {
        panic!("execution")
    };
    assert_eq!(execution.cwd, session.workspace.host_cwd);
    assert_eq!(execution.project_id.as_deref(), Some("project-original"));
    assert_eq!(execution.llm_connection_id, "connection-original");
    assert_eq!(execution.model, "model-original");
    assert_eq!(
        execution.thinking_level,
        Some(maka_runtime::execution::ThinkingLevel::High)
    );
    assert_eq!(execution.sandbox_mode, session.sandbox_mode);
    assert_eq!(execution.approval_policy, session.approval_policy);
    assert_eq!(execution.collaboration_mode, session.collaboration_mode);
    assert_eq!(execution.orchestration_mode, session.behavior);
    assert_eq!(execution.bound_tools, session.bound_tools);
    let mut legacy = serde_json::to_value(execution).unwrap();
    legacy.as_object_mut().unwrap().remove("boundTools");
    let legacy: crate::task::ExecutionTemplate = serde_json::from_value(legacy).unwrap();
    assert!(legacy.bound_tools.is_none());
    assert!(
        serde_json::to_value(legacy)
            .unwrap()
            .get("boundTools")
            .is_none()
    );
    let consent = target::consent(&effect, uuid::Uuid::new_v4(), "zh-CN");
    consent.validate().unwrap();
    assert!(
        matches!(consent.target, maka_plugins::authorization::Target::Workspace {workspace: maka_runtime::execution::WorkspaceTarget::Project {ref project_id}, sandbox_mode} if project_id == "project-original" && sandbox_mode == session.sandbox_mode)
    );
    let resume = target::from_session(target::Selection::Resume, &session).unwrap();
    assert_eq!(
        resume,
        Effect::SessionResume {
            session_id: "session-original".into()
        }
    );
    assert!(
        matches!(target::consent(&resume, uuid::Uuid::new_v4(), "en").target, maka_plugins::authorization::Target::Session {session_id} if session_id == "session-original")
    );
    let selector = target::Selector::capture(target::Selection::Run, &session).unwrap();
    let chosen: Route = serde_json::from_value(target::selected(None, selector.clone())).unwrap();
    assert!(
        matches!(chosen.creation, Some(create::Route::Form {target: saved, ..}) if saved == selector)
    );
    let mut unavailable = session;
    unavailable.tool_profile = Some(maka_runtime::execution::ToolProfile::HeadlessCodingV1);
    assert!(target::from_session(target::Selection::Run, &unavailable).is_err());
    assert!(target::from_session(target::Selection::Resume, &unavailable).is_err());
}
