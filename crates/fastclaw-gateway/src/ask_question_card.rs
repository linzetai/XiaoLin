//! Ask Question Interactive Card
//!
//! 构建飞书交互卡片用于 ask_question 功能

use serde_json::{json, Value};

/// 选项定义
#[derive(Debug, Clone)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
}

/// Ask Question 卡片构建器
pub struct AskQuestionCardBuilder {
    question: String,
    options: Vec<QuestionOption>,
    allow_multiple: bool,
    timeout_secs: Option<u32>,
    session_id: String,
    message_id: String,
}

impl AskQuestionCardBuilder {
    pub fn new(
        question: String,
        options: Vec<QuestionOption>,
        session_id: String,
        message_id: String,
    ) -> Self {
        Self {
            question,
            options,
            allow_multiple: false,
            timeout_secs: None,
            session_id,
            message_id,
        }
    }

    pub fn allow_multiple(mut self, allow: bool) -> Self {
        self.allow_multiple = allow;
        self
    }

    pub fn timeout_secs(mut self, secs: u32) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// 构建飞书卡片 JSON
    pub fn build(&self) -> Value {
        let config = json!({
            "width_mode": "adaptive",
            "enable_forward": false,
        });

        // 问题标题
        let mut elements = vec![json!({
            "tag": "div",
            "text": {
                "tag": "lark_md",
                "content": format!("**{}**", self.question),
            }
        })];

        // 超时提示
        if let Some(secs) = self.timeout_secs {
            elements.push(json!({
                "tag": "note",
                "elements": [{
                    "tag": "plain_text",
                    "content": format!("⏱️ 等待回答中...（超时: {}秒）", secs)
                }]
            }));
        }

        // 选项按钮
        let option_actions: Vec<Value> = self
            .options
            .iter()
            .map(|opt| {
                json!({
                    "tag": "button",
                    "text": {
                        "tag": "plain_text",
                        "content": &opt.label,
                    },
                    "type": "default",
                    "value": json!({
                        "session_id": &self.session_id,
                        "message_id": &self.message_id,
                        "option_id": &opt.id,
                        "action": if self.allow_multiple { "select_multi" } else { "select" },
                    }),
                })
            })
            .collect();

        // 根据是否多选，使用不同的布局
        if self.allow_multiple {
            // 多选：使用 checkbox 风格
            elements.push(json!({
                "tag": "action",
                "actions": option_actions,
                "layout": "bisect_spacing",
            }));

            // 添加确认按钮
            elements.push(json!({
                "tag": "action",
                "actions": [{
                    "tag": "button",
                    "text": {
                        "tag": "plain_text",
                        "content": "✓ 确认选择",
                    },
                    "type": "primary",
                    "value": json!({
                        "session_id": &self.session_id,
                        "message_id": &self.message_id,
                        "action": "confirm_multi",
                    }),
                }],
            }));
        } else {
            // 单选：按钮直接提交
            elements.push(json!({
                "tag": "action",
                "actions": option_actions,
                "layout": "bisect_spacing",
            }));
        }

        json!({
            "type": "template",
            "data": {
                "config": config,
                "elements": elements,
            }
        })
    }

    /// 构建已回答的卡片（用于更新）
    pub fn build_answered(&self, _selected_ids: &[String], selected_labels: &[String]) -> Value {
        let config = json!({
            "width_mode": "adaptive",
            "enable_forward": false,
        });

        let selected_text = selected_labels.join(", ");

        let elements = vec![
            json!({
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": format!("**{}**", self.question),
                }
            }),
            json!({
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": format!("✅ **已选择**: {}", selected_text),
                }
            }),
        ];

        json!({
            "type": "template",
            "data": {
                "config": config,
                "elements": elements,
            }
        })
    }

    /// 构建超时卡片
    pub fn build_timeout(&self) -> Value {
        let config = json!({
            "width_mode": "adaptive",
            "enable_forward": false,
        });

        let elements = vec![
            json!({
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": format!("**{}**", self.question),
                }
            }),
            json!({
                "tag": "div",
                "text": {
                    "tag": "lark_md",
                    "content": "⏰ **已超时** - 未收到回答",
                }
            }),
        ];

        json!({
            "type": "template",
            "data": {
                "config": config,
                "elements": elements,
            }
        })
    }
}

/// 从卡片回调 value 中解析选项
pub fn parse_card_callback(value: &Value) -> Option<CardCallback> {
    let session_id = value.get("session_id")?.as_str()?.to_string();
    let message_id = value.get("message_id")?.as_str()?.to_string();
    let action = value.get("action")?.as_str()?.to_string();
    let option_id = value.get("option_id").and_then(|v| v.as_str()).map(|s| s.to_string());

    Some(CardCallback {
        session_id,
        message_id,
        action,
        option_id,
    })
}

/// 卡片回调数据
#[derive(Debug, Clone)]
pub struct CardCallback {
    pub session_id: String,
    pub message_id: String,
    pub action: String,
    pub option_id: Option<String>,
}
