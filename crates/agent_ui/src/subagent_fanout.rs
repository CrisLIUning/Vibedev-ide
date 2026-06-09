//! Pure-logic aggregation model for the subagent fan-out tree.
//!
//! This module deliberately has **no GPUI dependency**: it takes the raw
//! `SubagentProgress` entries surfaced on the thread and folds them into a
//! flat list of `SubagentNode`s keyed by `subagent_id`, plus a few cheap
//! queries (running count, lookup). The GPUI render shell in `thread_view.rs`
//! consumes this model; keeping the logic here makes it unit-testable without
//! a `TestAppContext`.

use acp_thread::{SubagentProgress, SubagentStatus, SubagentStep, SubagentToolCall};

/// One fanned-out subagent, aggregated from its latest `SubagentProgress`.
#[derive(Clone, Debug, PartialEq)]
pub struct SubagentNode {
    pub subagent_id: String,
    pub agent_type: String,
    pub title: String,
    pub status: SubagentStatus,
    pub tokens_used: Option<u64>,
    pub parent_id: Option<String>,
    pub step: Option<SubagentStep>,
    /// Accumulated structured tool-call trace for this subagent (the thread
    /// folds each progress's increment in via `push_subagent_progress`).
    /// Rendered as expandable cards in the Execution detail panel.
    pub tool_calls: Vec<SubagentToolCall>,
    /// The subagent's final reply markdown, present once it completes.
    pub reply: Option<String>,
}

impl SubagentNode {
    fn from_progress(progress: &SubagentProgress) -> Self {
        Self {
            subagent_id: progress.subagent_id.clone(),
            agent_type: progress.agent_type.clone(),
            title: progress.title.clone(),
            status: progress.status.clone(),
            tokens_used: progress.tokens_used,
            parent_id: progress.parent_id.clone(),
            step: progress.step.clone(),
            tool_calls: progress.tool_calls.clone(),
            reply: progress.reply.clone(),
        }
    }

    /// True when this node is actively running and currently streaming a step.
    pub fn has_live_stream(&self) -> bool {
        self.status == SubagentStatus::Running
            && self
                .step
                .as_ref()
                .and_then(|step| step.stream.as_ref())
                .is_some()
    }

    /// True when this node is a child of another subagent (renders indented).
    pub fn is_child(&self) -> bool {
        self.parent_id.is_some()
    }
}

/// Aggregated fan-out state for a contiguous run of `SubagentProgress` entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubagentFanoutModel {
    /// Nodes in first-seen order, one per distinct `subagent_id`.
    pub nodes: Vec<SubagentNode>,
}

impl SubagentFanoutModel {
    /// Fold a slice of progress entries into the aggregated model.
    ///
    /// Entries sharing a `subagent_id` collapse into a single node: the last
    /// occurrence wins (progress entries are emitted cumulatively, so the most
    /// recent carries the freshest status/step/tokens), while the node keeps
    /// its original position in `nodes`.
    pub fn from_entries(progresses: &[SubagentProgress]) -> Self {
        let mut nodes: Vec<SubagentNode> = Vec::new();
        for progress in progresses {
            if let Some(existing) = nodes
                .iter_mut()
                .find(|node| node.subagent_id == progress.subagent_id)
            {
                *existing = SubagentNode::from_progress(progress);
            } else {
                nodes.push(SubagentNode::from_progress(progress));
            }
        }
        Self { nodes }
    }

    /// Number of nodes currently in the `Running` state.
    pub fn running_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.status == SubagentStatus::Running)
            .count()
    }

    /// Total number of distinct subagents.
    pub fn total_count(&self) -> usize {
        self.nodes.len()
    }

    /// Build the model from the spawn *folds* (placeholders that exist the
    /// instant the bridge emits the `tool_call`, before any progress) and then
    /// overlay the real `progress` that arrives later in the turn.
    ///
    /// Each fold is `(title, agent_type)` taken from the spawn tool-call's
    /// description / `subagent_type`. A `Queued` placeholder node is created per
    /// fold so the very first frame shows a populated card instead of a bare
    /// fold. Real progress is then matched onto a placeholder by `title`
    /// (spawn folds never carry a `subagent_id` — the agent only assigns one
    /// once the subagent actually starts — so identity is the description
    /// string the agent emits identically on both sides). A matched placeholder
    /// is replaced in place, keeping its row position. Progress with no matching
    /// fold (shouldn't happen for a marked batch, but kept for safety) is
    /// appended. Multiple progress entries for one subagent collapse to the
    /// latest, mirroring [`from_entries`].
    ///
    /// The placeholder node's `subagent_id` is seeded with `"{title}#{index}"`
    /// (its fold position), **not** the bare title: two folds sharing a
    /// `description` would otherwise collide on `subagent_id`, fusing their
    /// per-row `ElementId` (hover/active/click state bleeds between rows) and
    /// their `SelectedSubagent` key (both rows highlight, drill-in is
    /// ambiguous). The index keeps each Queued row's id/selection key unique;
    /// progress still overlays by `title` (folds carry no real id), and once
    /// real progress lands the row adopts the agent's true `subagent_id`. The
    /// display `title` is the original — only the synthetic id carries `#index`.
    pub fn from_spawn_folds_and_progress(
        folds: &[(String, String)],
        progress: &[SubagentProgress],
    ) -> Self {
        let mut nodes: Vec<SubagentNode> = folds
            .iter()
            .enumerate()
            .map(|(index, (title, agent_type))| SubagentNode {
                subagent_id: format!("{title}#{index}"),
                agent_type: agent_type.clone(),
                title: title.clone(),
                status: SubagentStatus::Queued,
                tokens_used: None,
                parent_id: None,
                step: None,
                tool_calls: Vec::new(),
                reply: None,
            })
            .collect();

        for progress in progress {
            let real = SubagentNode::from_progress(progress);
            // Prefer matching a real node already overlaid (same `subagent_id`)
            // so cumulative updates for one subagent collapse to the latest.
            if let Some(existing) = nodes
                .iter_mut()
                .find(|node| node.status != SubagentStatus::Queued && node.subagent_id == real.subagent_id)
            {
                *existing = real;
                continue;
            }
            // Otherwise claim a still-queued placeholder whose title matches.
            if let Some(placeholder) = nodes.iter_mut().find(|node| {
                node.status == SubagentStatus::Queued && node.title.trim() == real.title.trim()
            }) {
                *placeholder = real;
                continue;
            }
            // No placeholder for this progress — append it (defensive).
            nodes.push(real);
        }

        Self { nodes }
    }

    /// Look up a node by its `subagent_id`.
    ///
    /// Exercised by the model tests and reserved for the T8 detail-panel
    /// lookup; the v1 render shell iterates `nodes` directly.
    #[allow(dead_code)]
    pub fn node(&self, subagent_id: &str) -> Option<&SubagentNode> {
        self.nodes
            .iter()
            .find(|node| node.subagent_id == subagent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acp_thread::{SubagentProgress, SubagentStatus, SubagentStep};

    #[test]
    fn fanout_groups_by_subagent_and_marks_running() {
        let progresses = vec![
            SubagentProgress {
                subagent_id: "a".into(),
                agent_type: "Explore".into(),
                title: "UI".into(),
                status: SubagentStatus::Done,
                tokens_used: Some(52000),
                parent_id: None,
                batch_id: None,
                step: None,
                tool_calls: Vec::new(),
                reply: None,
            },
            SubagentProgress {
                subagent_id: "b".into(),
                agent_type: "Explore".into(),
                title: "i18n".into(),
                status: SubagentStatus::Running,
                tokens_used: Some(44000),
                parent_id: None,
                batch_id: None,
                step: Some(SubagentStep {
                    label: "读 app_menus.rs".into(),
                    sub: None,
                    status: "running".into(),
                    stream: Some("发现 About Zed…".into()),
                }),
                tool_calls: Vec::new(),
                reply: None,
            },
        ];
        let model = SubagentFanoutModel::from_entries(&progresses);
        assert_eq!(model.nodes.len(), 2);
        assert_eq!(model.running_count(), 1);
        assert!(model.node("b").unwrap().has_live_stream());
    }

    #[test]
    fn same_subagent_id_collapses_to_latest() {
        let progresses = vec![
            SubagentProgress {
                subagent_id: "a".into(),
                agent_type: "Explore".into(),
                title: "UI".into(),
                status: SubagentStatus::Running,
                tokens_used: Some(1000),
                parent_id: None,
                batch_id: None,
                step: None,
                tool_calls: Vec::new(),
                reply: None,
            },
            SubagentProgress {
                subagent_id: "a".into(),
                agent_type: "Explore".into(),
                title: "UI".into(),
                status: SubagentStatus::Done,
                tokens_used: Some(2000),
                parent_id: None,
                batch_id: None,
                step: None,
                tool_calls: Vec::new(),
                reply: None,
            },
        ];
        let model = SubagentFanoutModel::from_entries(&progresses);
        assert_eq!(model.nodes.len(), 1);
        assert_eq!(model.running_count(), 0);
        assert_eq!(model.node("a").unwrap().status, SubagentStatus::Done);
        assert_eq!(model.node("a").unwrap().tokens_used, Some(2000));
    }

    #[test]
    fn spawn_folds_seed_queued_rows_then_progress_overlays_in_place() {
        let folds = vec![
            ("UI rebrand".to_string(), "Explore".to_string()),
            ("i18n sweep".to_string(), "Explore".to_string()),
        ];
        // Frame A: no progress yet — two Queued placeholder rows.
        let model_a = SubagentFanoutModel::from_spawn_folds_and_progress(&folds, &[]);
        assert_eq!(model_a.nodes.len(), 2);
        assert!(
            model_a
                .nodes
                .iter()
                .all(|node| node.status == SubagentStatus::Queued)
        );
        // Display title is the original; the synthetic id carries `#index` so
        // each Queued row's ElementId / selection key is unique.
        assert_eq!(model_a.nodes[0].title, "UI rebrand");
        assert_eq!(model_a.nodes[0].subagent_id, "UI rebrand#0");
        assert_eq!(model_a.nodes[1].subagent_id, "i18n sweep#1");

        // Frame B: real progress for the first fold (matched by title) overlays
        // its placeholder in place — still two rows, no extra card row, the
        // matched row now carries the real subagent_id and Running status.
        let progress = vec![SubagentProgress {
            subagent_id: "sub-1".into(),
            agent_type: "Explore".into(),
            title: "UI rebrand".into(),
            status: SubagentStatus::Running,
            tokens_used: Some(1200),
            parent_id: None,
            batch_id: Some("batch-a".into()),
            step: None,
            tool_calls: Vec::new(),
            reply: None,
        }];
        let model_b = SubagentFanoutModel::from_spawn_folds_and_progress(&folds, &progress);
        assert_eq!(model_b.nodes.len(), 2);
        assert_eq!(model_b.nodes[0].subagent_id, "sub-1");
        assert_eq!(model_b.nodes[0].status, SubagentStatus::Running);
        // The unmatched fold stays a Queued placeholder.
        assert_eq!(model_b.nodes[1].status, SubagentStatus::Queued);
        assert_eq!(model_b.running_count(), 1);
    }

    #[test]
    fn duplicate_description_folds_get_unique_placeholder_ids() {
        // Two folds sharing one description: the Queued placeholders must still
        // get distinct `subagent_id`s (so their per-row ElementId / selection
        // key don't collide) while sharing the display title.
        let folds = vec![
            ("Audit module".to_string(), "Explore".to_string()),
            ("Audit module".to_string(), "Explore".to_string()),
        ];
        let model_a = SubagentFanoutModel::from_spawn_folds_and_progress(&folds, &[]);
        assert_eq!(model_a.nodes.len(), 2);
        assert_eq!(model_a.nodes[0].title, "Audit module");
        assert_eq!(model_a.nodes[1].title, "Audit module");
        assert_ne!(
            model_a.nodes[0].subagent_id, model_a.nodes[1].subagent_id,
            "duplicate-description placeholders must not share a subagent_id"
        );
        assert_eq!(model_a.nodes[0].subagent_id, "Audit module#0");
        assert_eq!(model_a.nodes[1].subagent_id, "Audit module#1");

        // Progress for one such subagent still overlays a Queued placeholder by
        // title (the agent emits the description identically on both streams).
        // The first matching placeholder is claimed; the other stays Queued and
        // retains its unique synthetic id.
        let progress = vec![SubagentProgress {
            subagent_id: "real-1".into(),
            agent_type: "Explore".into(),
            title: "Audit module".into(),
            status: SubagentStatus::Running,
            tokens_used: Some(100),
            parent_id: None,
            batch_id: Some("b".into()),
            step: None,
            tool_calls: Vec::new(),
            reply: None,
        }];
        let model_b = SubagentFanoutModel::from_spawn_folds_and_progress(&folds, &progress);
        assert_eq!(model_b.nodes.len(), 2, "no extra appended row");
        assert_eq!(model_b.nodes[0].subagent_id, "real-1");
        assert_eq!(model_b.nodes[0].status, SubagentStatus::Running);
        // The unclaimed twin keeps its unique placeholder id and Queued status.
        assert_eq!(model_b.nodes[1].subagent_id, "Audit module#1");
        assert_eq!(model_b.nodes[1].status, SubagentStatus::Queued);
        assert_ne!(model_b.nodes[0].subagent_id, model_b.nodes[1].subagent_id);
    }

    #[test]
    fn child_nodes_report_parentage_and_no_live_stream_when_done() {
        let progresses = vec![SubagentProgress {
            subagent_id: "child".into(),
            agent_type: "Code".into(),
            title: "patch".into(),
            status: SubagentStatus::Done,
            tokens_used: None,
            parent_id: Some("root".into()),
            batch_id: None,
            step: Some(SubagentStep {
                label: "wrote file".into(),
                sub: None,
                status: "done".into(),
                stream: Some("…".into()),
            }),
            tool_calls: Vec::new(),
            reply: None,
        }];
        let model = SubagentFanoutModel::from_entries(&progresses);
        let node = model.node("child").unwrap();
        assert!(node.is_child());
        // Done node never reports a live stream even with a stream payload.
        assert!(!node.has_live_stream());
    }
}
