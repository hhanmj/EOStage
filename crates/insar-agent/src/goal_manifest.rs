use crate::history;
use insar_store::TaskStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

const MAX_GOAL_ITEMS: usize = 12;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalItemStatus {
    Pending,
    Ready,
    PendingUser,
    CapabilityMissing,
}

impl GoalItemStatus {
    fn from_str(raw: &str) -> Option<Self> {
        match raw.trim() {
            "pending" => Some(Self::Pending),
            "ready" => Some(Self::Ready),
            "pending_user" => Some(Self::PendingUser),
            "capability_missing" => Some(Self::CapabilityMissing),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::PendingUser => "pending_user",
            Self::CapabilityMissing => "capability_missing",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalItem {
    pub item_id: String,
    pub label_zh: String,
    pub status: GoalItemStatus,
    pub batch_id: Option<String>,
    pub task_id: Option<String>,
    pub note_zh: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalManifest {
    pub title_zh: String,
    pub items: Vec<GoalItem>,
}

impl GoalManifest {
    pub fn all_ready(&self) -> bool {
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|item| item.status == GoalItemStatus::Ready)
    }

    pub fn summary_json(&self) -> Value {
        let mut ready_count = 0usize;
        let mut unresolved_pending_count = 0usize;
        let mut pending_user_count = 0usize;
        let mut capability_missing_count = 0usize;
        for item in &self.items {
            match item.status {
                GoalItemStatus::Ready => ready_count += 1,
                GoalItemStatus::Pending => unresolved_pending_count += 1,
                GoalItemStatus::PendingUser => pending_user_count += 1,
                GoalItemStatus::CapabilityMissing => capability_missing_count += 1,
            }
        }
        let items_json: Vec<Value> = self
            .items
            .iter()
            .map(|item| {
                json!({
                    "item_id": item.item_id,
                    "label_zh": item.label_zh,
                    "status": item.status.as_str(),
                    "batch_id": item.batch_id,
                    "task_id": item.task_id,
                    "note_zh": item.note_zh,
                })
            })
            .collect();
        json!({
            "status": "ok",
            "title_zh": self.title_zh,
            "all_ready": self.all_ready(),
            "ready_count": ready_count,
            "unresolved_pending_count": unresolved_pending_count,
            "pending_user_count": pending_user_count,
            "capability_missing_count": capability_missing_count,
            "items": items_json,
        })
    }

    pub fn honest_disclosure_suffix(&self) -> Option<String> {
        if self.all_ready() {
            return None;
        }
        let mut lines = vec![format!("**{}（交付清单核对）**", self.title_zh)];
        for item in &self.items {
            let line = match item.status {
                GoalItemStatus::Ready => format!("- ✅ **{}**：已就绪", item.label_zh),
                GoalItemStatus::Pending => {
                    format!(
                        "- ⏳ **{}**：待补齐（尚未绑定真实检索批次或任务）",
                        item.label_zh
                    )
                }
                GoalItemStatus::PendingUser => {
                    let note = item
                        .note_zh
                        .as_deref()
                        .unwrap_or("等待用户确认或补充前置条件");
                    format!("- ⏳ **{}**：待确认（{}）", item.label_zh, note)
                }
                GoalItemStatus::CapabilityMissing => {
                    let note = item
                        .note_zh
                        .as_deref()
                        .unwrap_or("当前系统暂未接入该数据源或处理能力");
                    format!("- ⚠️ **{}**：当前能力未接入（{}）", item.label_zh, note)
                }
            };
            lines.push(line);
        }
        Some(lines.join("\n"))
    }

    pub fn needs_disclosure_in(&self, text: &str) -> bool {
        if self.all_ready() {
            return false;
        }
        let false_complete =
            text.contains("全部配齐") || text.contains("已全部就绪") || text.contains("全部完成");
        if false_complete {
            return true;
        }
        self.items
            .iter()
            .filter(|item| item.status != GoalItemStatus::Ready)
            .any(|item| !text.contains(&item.label_zh))
    }
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["action"],
        "properties": {
            "action": {
                "type": "string",
                "enum": ["define", "bind", "check"],
                "description": "目标清单操作：define 初始化必需交付物列表；bind 将某项绑定真实 batch_id 或 task_id 并更新状态；check 核验清单完成情况"
            },
            "title_zh": {
                "type": "string",
                "minLength": 1,
                "maxLength": 160,
                "description": "define 时的复合任务目标标题"
            },
            "items": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_GOAL_ITEMS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["item_id", "label_zh"],
                    "properties": {
                        "item_id": {"type": "string", "minLength": 1, "maxLength": 64},
                        "label_zh": {"type": "string", "minLength": 1, "maxLength": 120},
                        "status": {
                            "type": "string",
                            "enum": ["pending", "pending_user", "capability_missing"]
                        },
                        "note_zh": {"type": "string", "minLength": 1, "maxLength": 240}
                    }
                }
            },
            "item_id": {
                "type": "string",
                "minLength": 1,
                "maxLength": 64,
                "description": "bind 时指定的目标交付项 ID"
            },
            "status": {
                "type": "string",
                "enum": ["pending", "ready", "pending_user", "capability_missing"],
                "description": "bind 时更新的目标状态；设为 ready 时必须同时提供真实存在的 batch_id 或 task_id"
            },
            "batch_id": {
                "type": "string",
                "minLength": 1,
                "maxLength": 128,
                "description": "已冻结的 search_batch_id"
            },
            "task_id": {
                "type": "string",
                "minLength": 1,
                "maxLength": 128,
                "description": "已创建的下载或处理任务 task_id"
            },
            "note_zh": {
                "type": "string",
                "minLength": 1,
                "maxLength": 240,
                "description": "当状态为 capability_missing 或 pending_user 时填写的中文说明"
            }
        }
    })
}

fn validate_opt_str(
    map: &serde_json::Map<String, Value>,
    key: &str,
    max_len: usize,
) -> Result<Option<String>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.chars().count() > max_len {
                return Err(format!("{key} must be 1..={max_len} chars"));
            }
            Ok(Some(history::outbound_text(trimmed)))
        }
        Some(_) => Err(format!("{key} must be a string")),
    }
}

pub enum ApplyOutcome {
    Summary(Value),
    Refused(Value),
}

pub fn apply_with_store<F>(
    slot: &mut Option<GoalManifest>,
    arguments: &Value,
    verify_store: F,
) -> Result<ApplyOutcome, String>
where
    F: FnOnce(Option<&str>, Option<&str>) -> Result<bool, String>,
{
    let map = arguments
        .as_object()
        .ok_or_else(|| "goal_manifest arguments must be an object".to_owned())?;
    for key in map.keys() {
        if !matches!(
            key.as_str(),
            "action"
                | "title_zh"
                | "items"
                | "item_id"
                | "status"
                | "batch_id"
                | "task_id"
                | "note_zh"
        ) {
            return Err(format!("unknown field {key}"));
        }
    }
    let action = map
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| matches!(*s, "define" | "bind" | "check"))
        .ok_or_else(|| "action must be one of define, bind, check".to_owned())?;

    match action {
        "define" => {
            let title_zh = validate_opt_str(map, "title_zh", 160)?
                .unwrap_or_else(|| "复合科研配数清单".to_owned());
            let raw_items = map
                .get("items")
                .and_then(Value::as_array)
                .filter(|arr| !arr.is_empty() && arr.len() <= MAX_GOAL_ITEMS)
                .ok_or_else(|| format!("items must be an array of 1..={MAX_GOAL_ITEMS} items"))?;

            let mut seen = HashSet::new();
            let mut items = Vec::with_capacity(raw_items.len());
            for entry in raw_items {
                let obj = entry
                    .as_object()
                    .ok_or_else(|| "each goal item must be an object".to_owned())?;
                for key in obj.keys() {
                    if !matches!(key.as_str(), "item_id" | "label_zh" | "status" | "note_zh") {
                        return Err(format!("unknown goal item field {key}"));
                    }
                }
                let item_id = validate_opt_str(obj, "item_id", 64)?
                    .ok_or_else(|| "item_id is required".to_owned())?;
                if !seen.insert(item_id.clone()) {
                    return Err(format!("duplicate item_id {item_id}"));
                }
                let label_zh = validate_opt_str(obj, "label_zh", 120)?
                    .ok_or_else(|| "label_zh is required".to_owned())?;
                let status = match obj.get("status") {
                    None | Some(Value::Null) => GoalItemStatus::Pending,
                    Some(Value::String(raw_s)) => match GoalItemStatus::from_str(raw_s) {
                        Some(GoalItemStatus::Ready) => {
                            return Ok(ApplyOutcome::Refused(json!({
                                "status": "refused",
                                "code": "goal_evidence_not_found",
                                "item_id": item_id,
                                "detail": "初始化清单不能直接标记 ready，请通过 action=bind 绑定真实存在的 batch_id 或 task_id"
                            })));
                        }
                        Some(st) => st,
                        None => return Err(format!("invalid status {raw_s}")),
                    },
                    Some(_) => return Err("status must be a string".to_owned()),
                };
                let note_zh = validate_opt_str(obj, "note_zh", 240)?;
                if status == GoalItemStatus::CapabilityMissing && note_zh.is_none() {
                    return Err("capability_missing items require non-empty note_zh".to_owned());
                }
                items.push(GoalItem {
                    item_id,
                    label_zh,
                    status,
                    batch_id: None,
                    task_id: None,
                    note_zh,
                });
            }

            let manifest = GoalManifest { title_zh, items };
            let summary = manifest.summary_json();
            *slot = Some(manifest);
            Ok(ApplyOutcome::Summary(summary))
        }
        "bind" => {
            let Some(manifest) = slot.as_mut() else {
                return Ok(ApplyOutcome::Refused(json!({
                    "status": "refused",
                    "code": "goal_manifest_not_defined",
                    "detail": "请先调用 action=define 初始化交付清单"
                })));
            };
            let item_id = validate_opt_str(map, "item_id", 64)?
                .ok_or_else(|| "item_id is required for bind".to_owned())?;
            let status_raw = map
                .get("status")
                .and_then(Value::as_str)
                .ok_or_else(|| "status is required for bind".to_owned())?;
            let status = GoalItemStatus::from_str(status_raw)
                .ok_or_else(|| format!("invalid status {status_raw}"))?;
            let batch_id = validate_opt_str(map, "batch_id", 128)?;
            let task_id = validate_opt_str(map, "task_id", 128)?;
            let note_zh = validate_opt_str(map, "note_zh", 240)?;

            if !manifest.items.iter().any(|i| i.item_id == item_id) {
                return Ok(ApplyOutcome::Refused(json!({
                    "status": "refused",
                    "code": "goal_item_not_found",
                    "item_id": item_id,
                    "detail": "未在当前交付清单中找到该 item_id"
                })));
            }

            if status == GoalItemStatus::CapabilityMissing && note_zh.is_none() {
                return Err("capability_missing requires non-empty note_zh".to_owned());
            }

            if status == GoalItemStatus::Ready {
                if batch_id.is_none() && task_id.is_none() {
                    return Ok(ApplyOutcome::Refused(json!({
                        "status": "refused",
                        "code": "goal_evidence_not_found",
                        "item_id": item_id,
                        "detail": "标记为 ready 的配数项必须绑定真实存在的 search_batch_id 或 task_id"
                    })));
                }
                let exists = verify_store(batch_id.as_deref(), task_id.as_deref())?;
                if !exists {
                    return Ok(ApplyOutcome::Refused(json!({
                        "status": "refused",
                        "code": "goal_evidence_not_found",
                        "item_id": item_id,
                        "detail": "标记为 ready 的配数项必须绑定真实存在的 search_batch_id 或 task_id"
                    })));
                }
            }

            let target = manifest
                .items
                .iter_mut()
                .find(|i| i.item_id == item_id)
                .expect("item checked above");
            target.status = status;
            if batch_id.is_some() {
                target.batch_id = batch_id;
            }
            if task_id.is_some() {
                target.task_id = task_id;
            }
            if note_zh.is_some() {
                target.note_zh = note_zh;
            }
            Ok(ApplyOutcome::Summary(manifest.summary_json()))
        }
        "check" => {
            let Some(manifest) = slot.as_ref() else {
                return Ok(ApplyOutcome::Summary(json!({
                    "status": "ok",
                    "defined": false,
                    "all_ready": false,
                    "unresolved_pending_count": 0,
                    "ready_count": 0,
                    "pending_user_count": 0,
                    "capability_missing_count": 0,
                    "items": []
                })));
            };
            Ok(ApplyOutcome::Summary(manifest.summary_json()))
        }
        _ => unreachable!(),
    }
}

pub fn store_has_evidence(
    store: &TaskStore,
    batch_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<bool, String> {
    if let Some(bid) = batch_id {
        let ok = store
            .search_batch_exists(bid)
            .map_err(|err| format!("store search_batch check failed: {err}"))?;
        if !ok {
            return Ok(false);
        }
    }
    if let Some(tid) = task_id {
        let ok = store
            .load_task_record(tid)
            .map_err(|err| format!("store load_task_record failed: {err}"))?
            .is_some();
        if !ok {
            return Ok(false);
        }
    }
    Ok(batch_id.is_some() || task_id.is_some())
}
