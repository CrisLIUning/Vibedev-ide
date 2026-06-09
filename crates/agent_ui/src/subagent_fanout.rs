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
    fn child_nodes_report_parentage_and_no_live_stream_when_done() {
        let progresses = vec![SubagentProgress {
            subagent_id: "child".into(),
            agent_type: "Code".into(),
            title: "patch".into(),
            status: SubagentStatus::Done,
            tokens_used: None,
            parent_id: Some("root".into()),
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
