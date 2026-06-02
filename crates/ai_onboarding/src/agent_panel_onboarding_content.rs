use std::sync::Arc;

use client::{Client, UserStore};
use gpui::{Entity, IntoElement};
use language_model::{LanguageModelRegistry, ZED_CLOUD_PROVIDER_ID};
use ui::prelude::*;

pub struct AgentPanelOnboarding {
    user_store: Entity<UserStore>,
    client: Arc<Client>,
    has_configured_providers: bool,
    continue_with_zed_ai: Arc<dyn Fn(&mut Window, &mut App)>,
}

impl AgentPanelOnboarding {
    pub fn new(
        user_store: Entity<UserStore>,
        client: Arc<Client>,
        continue_with_zed_ai: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(
            &LanguageModelRegistry::global(cx),
            |this: &mut Self, _registry, event: &language_model::Event, cx| match event {
                language_model::Event::ProviderStateChanged(_)
                | language_model::Event::AddedProvider(_)
                | language_model::Event::RemovedProvider(_)
                | language_model::Event::ProvidersChanged => {
                    this.has_configured_providers = Self::has_configured_providers(cx)
                }
                _ => {}
            },
        )
        .detach();

        Self {
            user_store,
            client,
            has_configured_providers: Self::has_configured_providers(cx),
            continue_with_zed_ai: Arc::new(continue_with_zed_ai),
        }
    }

    fn has_configured_providers(cx: &App) -> bool {
        LanguageModelRegistry::read_global(cx)
            .visible_providers()
            .iter()
            .any(|provider| provider.is_authenticated(cx) && provider.id() != ZED_CLOUD_PROVIDER_ID)
    }
}

impl Render for AgentPanelOnboarding {
    // VIBEDEV: hide the upstream Zed AI / Zed Pro welcome card on every state —
    // VibeDev ships its own account UI (`crates/vibedev_ui/src/account_panel.rs`)
    // and does not surface Zed's cloud/Pro upsell or plan stamps. Render an empty
    // div so the agent panel still has a valid child but no visible onboarding.
    // The struct fields stay live (held by `cx.new(...)` in agent_panel.rs and
    // updated by the LanguageModelRegistry subscription) but are not painted.
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.user_store;
        let _ = &self.client;
        let _ = self.has_configured_providers;
        let _ = &self.continue_with_zed_ai;
        gpui::div()
    }
}
