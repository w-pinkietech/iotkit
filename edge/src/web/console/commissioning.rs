use crate::{
    storage::EdgeNodeState,
    web::{ConsoleDevice, ConsoleEdgeNode, ConsoleSignal},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommissioningView {
    pub stage: String,
    pub title: String,
    pub explanation: String,
    pub action_label: String,
    pub action_href: String,
    pub completed_steps: usize,
    pub total_steps: usize,
    pub pending_edge_nodes: usize,
    pub pending_devices: usize,
    pub pending_signals: usize,
}

#[must_use]
pub fn commissioning_view(
    edge_nodes: &[ConsoleEdgeNode],
    devices: &[ConsoleDevice],
    signals: &[ConsoleSignal],
) -> CommissioningView {
    let pending_edge_nodes = edge_nodes
        .iter()
        .filter(|node| node.state != EdgeNodeState::Active)
        .count();
    let pending_devices = devices
        .iter()
        .filter(|device| device.descriptor_current && device.revision == 0)
        .count();
    let pending_signals = signals
        .iter()
        .filter(|signal| {
            signal.descriptor_current && (!signal.profile_complete || signal.rules.is_empty())
        })
        .count();

    let stage = if edge_nodes.is_empty() {
        stage(
            "waiting-edge-node",
            "収集ノードの接続を待っています",
            "収集ノードからdescriptorを受信すると、ここに登録手順が表示されます。",
            "接続状況を確認",
            "/equipment".into(),
            0,
        )
    } else if let Some(node) = edge_nodes
        .iter()
        .find(|node| node.state == EdgeNodeState::RecoveryHold)
    {
        stage(
            "recovery",
            "収集ノードの復旧を確認",
            "復旧確認待ちの収集ノードがあります。",
            "収集ノードを確認",
            edge_node_href(node),
            0,
        )
    } else if let Some(node) = edge_nodes
        .iter()
        .find(|node| node.state == EdgeNodeState::Activating)
    {
        stage(
            "activation-in-progress",
            "収集ノードを登録中",
            "3秒ごとに登録状態を自動確認します。この画面を離れても登録処理は続きます。",
            "収集ノードを確認",
            edge_node_href(node),
            0,
        )
    } else if let Some(node) = edge_nodes
        .iter()
        .find(|node| node.state == EdgeNodeState::Discovered)
    {
        stage(
            "activate-edge-node",
            "収集ノードを登録",
            "検出した収集ノードを登録してください。",
            "収集ノードを確認",
            edge_node_href(node),
            0,
        )
    } else if let Some(device) = devices
        .iter()
        .find(|device| device.descriptor_current && device.revision == 0)
    {
        stage(
            "setup-device",
            "機器を確認",
            "名前と設置場所が未設定の機器があります。",
            "機器を設定",
            format!("/equipment/devices/{}", device.device_ref),
            1,
        )
    } else if let Some(signal) = signals
        .iter()
        .find(|signal| signal.descriptor_current && !signal.profile_complete)
    {
        stage(
            "setup-sensor",
            "センサーを設定",
            "表示方法が未設定のセンサーがあります。",
            "センサーを設定",
            signal_href(signal),
            2,
        )
    } else if let Some(signal) = signals
        .iter()
        .find(|signal| signal.descriptor_current && signal.rules.is_empty())
    {
        stage(
            "setup-rule",
            "計測ルールを設定",
            "計測を始めるためのルールが未設定です。",
            "計測ルールを設定",
            signal_href(signal),
            3,
        )
    } else {
        stage(
            "complete",
            "計測を開始できます",
            "収集ノード、機器、センサー、計測ルールの設定が完了しました。",
            "センサーを確認",
            "/sensors".into(),
            4,
        )
    };

    CommissioningView {
        total_steps: 4,
        pending_edge_nodes,
        pending_devices,
        pending_signals,
        ..stage
    }
}

fn stage(
    stage: &str,
    title: &str,
    explanation: &str,
    action_label: &str,
    action_href: String,
    completed_steps: usize,
) -> CommissioningView {
    CommissioningView {
        stage: stage.into(),
        title: title.into(),
        explanation: explanation.into(),
        action_label: action_label.into(),
        action_href,
        completed_steps,
        ..CommissioningView::default()
    }
}

fn edge_node_href(node: &ConsoleEdgeNode) -> String {
    format!("/equipment/edge-nodes/{}", node.edge_node_ref)
}

fn signal_href(signal: &ConsoleSignal) -> String {
    format!(
        "/equipment/devices/{}/sensors/{}",
        signal.device_ref, signal.signal_ref
    )
}
