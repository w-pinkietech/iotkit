use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use iotkit_edge::web::{WebConfig, router, test_support::StubApplication};
use tower::ServiceExt;

#[tokio::test]
async fn history_rejects_unbounded_and_oversized_pages() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    for path in ["/api/v1/history", "/api/v1/history?from=a&to=b&limit=1001"] {
        let response = app
            .clone()
            .oneshot(authenticated(
                Request::get(path).body(Body::empty()).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn csv_has_bom_exact_header_and_formula_injection_defense() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(authenticated(
            Request::get("/api/v1/history.csv?from=1735689600000&to=1735776000000&limit=0")
                .body(Body::empty())
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        r#"attachment; filename="iotkit-history.csv""#
    );
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(body.starts_with(&[0xef, 0xbb, 0xbf]));
    let csv = String::from_utf8(body[3..].to_vec()).unwrap();
    assert!(csv.starts_with(
        "received_at,observed_at,edge_node_id,signal_ref,series_key,sensor_name,values,unit\r\n"
    ));
    assert!(csv.contains("'=danger"));
}

#[tokio::test]
async fn semantic_csv_preserves_distinct_schema_and_escaping() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(authenticated(
            Request::get("/api/v1/semantic-history.csv?from=1735689600000&to=1735776000000")
                .body(Body::empty())
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        r#"attachment; filename="iotkit-processed-history.csv""#
    );
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let csv = String::from_utf8(body[3..].to_vec()).unwrap();
    assert!(csv.starts_with("observed_at,processed_at,edge_node_id,signal_ref,sensor_name,rule_name,kind,value,unit,series_id,sequence,observation_id,rule_revision,calibration_revision,source_pub_seq\r\n"));
    assert!(csv.contains("'=unsafe"));
}

#[tokio::test]
async fn history_rejects_invalid_timestamps_ranges_and_missing_series_bucket() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    for path in [
        "/api/v1/history?from=not-a-number&to=1000",
        "/api/v1/history?from=1000&to=1000",
        "/api/v1/history?from=0&to=2678400001",
        "/api/v1/history/series?from=0&to=1000&signal_ref=signal-1",
        "/api/v1/history/series?from=0&to=1000&signal_ref=signal-1&bucket_ms=0",
    ] {
        let response = app
            .clone()
            .oneshot(authenticated(
                Request::get(path).body(Body::empty()).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
    }
}

#[tokio::test]
async fn history_series_includes_exact_latest_value_for_live_cards() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(authenticated(
            Request::get(
                "/api/v1/history/series?from=1735689590000&to=1735689610000&signal_ref=signal-1&bucket_ms=1000",
            )
            .body(Body::empty())
            .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(body["signal_ref"], "signal-1");
    assert_eq!(body["latest_received_at"], 1_735_689_600_000_i64);
    assert_eq!(body["latest_value"], 1.0);
    assert_eq!(body["points"][0]["average"], 1.0);
}

#[tokio::test]
async fn history_json_preserves_go_page_and_record_schema() {
    let app = router(
        WebConfig::test(),
        Arc::new(StubApplication::authenticated()),
    );
    let response = app
        .oneshot(authenticated(
            Request::get("/api/v1/history?from=0&to=1000&limit=20")
                .body(Body::empty())
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert!(body["records"].is_array());
    assert_eq!(body["has_more"], false);
    assert!(body.get("items").is_none());
    let record = &body["records"][0];
    assert!(record["received_at"].is_i64());
    assert!(record["observed_at"].is_i64());
    assert!(record["values"].is_array());
    for field in [
        "signal_ref",
        "series_key",
        "edge_node_id",
        "ledger_epoch",
        "pub_seq",
        "value_type",
        "unit",
        "display_name",
        "decimal_places",
        "display_value_kind",
    ] {
        assert!(!record[field].is_null(), "missing {field}");
    }
}

fn authenticated(mut request: Request<Body>) -> Request<Body> {
    request.headers_mut().insert(
        header::COOKIE,
        "iotkit_edge_session=valid; iotkit_edge_csrf=csrf"
            .parse()
            .unwrap(),
    );
    request
}
