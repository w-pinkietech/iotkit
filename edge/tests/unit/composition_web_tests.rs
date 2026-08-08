use super::*;

#[test]
fn range_bounds_are_bounded_by_the_selected_window() {
    assert_eq!(
        history_range_bounds("1h", 7_200_000),
        (3_600_000, 7_200_000)
    );
    assert_eq!(
        history_range_bounds("24h", 90_000_000),
        (3_600_000, 90_000_000)
    );
}

#[test]
fn numeric_history_builds_a_non_empty_bounded_chart_path() {
    let rows = [
        history_row("1735689602000", "[30.0]"),
        history_row("1735689601000", "[20.0]"),
    ];
    let chart = raw_history_chart(&rows);
    assert!(chart.path.starts_with('M'));
    assert!(chart.path.contains(" L"));
    assert!(!chart.path.contains("NaN"));
    assert_eq!(chart.start_at, "1735689601000");
    assert_eq!(chart.end_at, "1735689602000");
    assert_eq!(chart.minimum_label, "20.0");
    assert_eq!(chart.midpoint_label, "25.0");
    assert_eq!(chart.maximum_label, "30.0");
    assert_eq!(chart.unit, "℃");
    assert_eq!(chart.point_count, 2);
}

#[test]
fn display_raw_value_rounds_without_padding_fractional_zeroes() {
    assert_eq!(display_raw_value(&serde_json::json!(42.0), 1), "42");
    assert_eq!(display_raw_value(&serde_json::json!(42.45), 1), "42.5");
    assert_eq!(display_raw_value(&serde_json::json!(0), 0), "0");
}

fn history_row(received_at: &str, values: &str) -> RawHistoryRow {
    RawHistoryRow {
        received_at: received_at.into(),
        observed_at: received_at.into(),
        edge_node_id: "edge-node-01".into(),
        ledger_epoch: "epoch-01".into(),
        pub_seq: 1,
        signal_ref: "signal-01".into(),
        series_key: "temperature".into(),
        sensor_name: "温度".into(),
        values: values.into(),
        value_type: "number".into(),
        unit: "℃".into(),
        decimal_places: 1,
        display_value_kind: "numeric".into(),
    }
}
