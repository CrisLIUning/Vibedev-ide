use gpui::{IntoElement, ParentElement};
use ui::{List, ListBulletItem, prelude::*};

/// Centralized definitions for Zed AI plans
pub struct PlanDefinitions;

impl PlanDefinitions {
    pub fn free_plan(&self) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("2,000 次接受的编辑预测"))
            .child(ListBulletItem::new(
                "使用您的 AI API 密钥无限制发送提示词",
            ))
            .child(ListBulletItem::new("无限制使用外部 Agent"))
    }

    pub fn sign_in_upsell(&self) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("无限制编辑预测"))
            .child(ListBulletItem::new("VibeDev Agent 中价值 $20 的 Token"))
            .child(ListBulletItem::new("无需信用卡"))
    }

    pub fn pro_trial(&self, period: bool) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("VibeDev Agent 中价值 $20 的 Token"))
            .child(ListBulletItem::new("无限制编辑预测"))
            .when(period, |this| {
                this.child(ListBulletItem::new(
                    "试用 14 天,无需信用卡",
                ))
            })
    }

    pub fn pro_plan(&self) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("VibeDev Agent 中价值 $5 的 Token"))
            .child(ListBulletItem::new("超过 $5 后按用量计费"))
            .child(ListBulletItem::new("无限制编辑预测"))
    }

    pub fn business_plan(&self) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("无限制编辑预测"))
            .child(ListBulletItem::new("基于用量的计费"))
    }

    pub fn student_plan(&self) -> impl IntoElement {
        List::new()
            .child(ListBulletItem::new("无限制编辑预测"))
            .child(ListBulletItem::new("VibeDev Agent 中价值 $10 的 Token"))
            .child(ListBulletItem::new(
                "可选额度包用于额外使用",
            ))
    }
}
