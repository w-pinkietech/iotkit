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
            Request::get("/api/v1/history.csv?from=2025-01-01T00:00:00Z&to=2025-01-02T00:00:00Z")
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
            Request::get(
                "/api/v1/semantic-history.csv?from=2025-01-01T00:00:00Z&to=2025-01-02T00:00:00Z",
            )
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

fn authenticated(mut request: Request<Body>) -> Request<Body> {
    request.headers_mut().insert(
        header::COOKIE,
        "iotkit_edge_session=valid; iotkit_edge_csrf=csrf"
            .parse()
            .unwrap(),
    );
    request
}
