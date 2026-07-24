pub mod api;
pub mod console;
mod error;
pub mod router;

use std::{collections::HashMap, sync::Arc};

use askama::Template;
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Form, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use error::WebError;

pub const SESSION_COOKIE: &str = "iotkit_edge_session";
pub const CSRF_COOKIE: &str = "iotkit_edge_csrf";
pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_HISTORY_PAGE: u16 = 1_000;
pub const MAX_HISTORY_EXPORT_ROWS: usize = 100_000;

#[derive(Clone)]
pub struct WebConfig {
    pub public_origin: String,
    pub secure_cookies: bool,
}

impl WebConfig {
    pub fn test() -> Self {
        Self {
            public_origin: "http://127.0.0.1:8080".into(),
            secure_cookies: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Principal {
    pub account_ref: String,
    pub login_id: String,
    pub display_name: String,
    pub role: String,
    pub state: String,
    pub must_change_password: bool,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub struct LoginSession {
    pub token: String,
    pub csrf: String,
    pub principal: Principal,
}

#[derive(Clone, Debug)]
pub struct RawHistoryRow {
    pub received_at: String,
    pub observed_at: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub pub_seq: i64,
    pub signal_ref: String,
    pub series_key: String,
    pub sensor_name: String,
    pub values: String,
    pub value_type: String,
    pub unit: String,
    pub decimal_places: i32,
    pub display_value_kind: String,
}

impl Serialize for RawHistoryRow {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let values =
            serde_json::from_str::<Value>(&self.values).unwrap_or_else(|_| json!(self.values));
        json!({
            "signal_ref": self.signal_ref,
            "series_key": self.series_key,
            "edge_node_id": self.edge_node_id,
            "ledger_epoch": self.ledger_epoch,
            "pub_seq": self.pub_seq,
            "received_at": self.received_at.parse::<i64>().unwrap_or_default(),
            "observed_at": self.observed_at.parse::<i64>().unwrap_or_default(),
            "values": values,
            "value_type": self.value_type,
            "unit": self.unit,
            "display_name": self.sensor_name,
            "decimal_places": self.decimal_places,
            "display_value_kind": self.display_value_kind,
        })
        .serialize(serializer)
    }
}

#[derive(Clone, Debug)]
pub struct HistoryPage {
    pub rows: Vec<RawHistoryRow>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct SemanticHistoryRow {
    pub observed_at: String,
    pub processed_at: String,
    pub edge_node_id: String,
    pub signal_ref: String,
    pub sensor_name: String,
    pub rule_name: String,
    pub kind: String,
    pub value: String,
    pub unit: String,
    pub series_id: String,
    pub sequence: i64,
    pub observation_id: String,
    pub rule_revision: i64,
    pub calibration_revision: i64,
    pub source_pub_seq: i64,
}

#[derive(Clone, Debug)]
pub struct SemanticHistoryPage {
    pub rows: Vec<SemanticHistoryRow>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct ConsoleRequest {
    pub path: String,
    pub query: HashMap<String, String>,
    pub principal: Principal,
}

#[derive(Clone, Debug, Default)]
pub struct ConsoleView {
    pub notice: String,
    pub page_error: String,
    pub edge_nodes: Vec<ConsoleEdgeNode>,
    pub signals: Vec<ConsoleSignal>,
    pub selected_edge_node: Option<ConsoleEdgeNode>,
    pub selected_signal: Option<ConsoleSignal>,
    pub history: Vec<RawHistoryRow>,
    pub history_chart_path: String,
    pub history_raw_export_url: String,
    pub history_processed_export_url: String,
    pub outputs: Vec<ConsoleOutput>,
    pub accounts: Vec<ConsoleAccount>,
    pub audit: Vec<ConsoleAudit>,
    pub storage: ConsoleStorage,
}

#[derive(Clone, Debug)]
pub struct ConsoleEdgeNode {
    pub edge_node_ref: String,
    pub edge_node_id: String,
    pub name: String,
    pub location: String,
    pub state_label: String,
    pub state_class: String,
    pub can_activate: bool,
}

#[derive(Clone, Debug)]
pub struct ConsoleSignal {
    pub signal_ref: String,
    pub device_ref: String,
    pub edge_node_id: String,
    pub name: String,
    pub sensor_type: String,
    pub value: String,
    pub unit: String,
    pub status_label: String,
    pub status_class: String,
    pub profile_complete: bool,
    pub rules: Vec<ConsoleRule>,
}

#[derive(Clone, Debug)]
pub struct ConsoleRule {
    pub rule_id: String,
    pub display_name: String,
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct ConsoleOutput {
    pub profile_id: String,
    pub adapter_id: String,
    pub display_name: String,
    pub description: String,
    pub active: bool,
    pub bindings: Vec<ConsoleBinding>,
}

#[derive(Clone, Debug)]
pub struct ConsoleBinding {
    pub binding_id: String,
    pub sensor_name: String,
    pub rule_name: String,
    pub state_label: String,
    pub prepared: bool,
}

#[derive(Clone, Debug)]
pub struct ConsoleAccount {
    pub account_ref: String,
    pub login_id: String,
    pub display_name: String,
    pub role: String,
    pub state: String,
    pub revision: i64,
}

#[derive(Clone, Debug)]
pub struct ConsoleAudit {
    pub occurred_at: String,
    pub actor: String,
    pub action: String,
    pub target: String,
}

#[derive(Clone, Debug, Default)]
pub struct ConsoleStorage {
    pub profile_label: String,
    pub raw_count: i64,
    pub pending_output_count: i64,
    pub used_percent: u8,
    pub host_capacity_available: bool,
    pub retention_note: String,
    pub diagnostic_messages: Vec<String>,
}

#[async_trait]
pub trait WebApplication: Send + Sync + 'static {
    async fn login(&self, username: &str, password: &str) -> Result<LoginSession, WebError>;
    async fn authenticate(&self, token: &str) -> Result<Principal, WebError>;
    async fn validate_csrf(&self, token: &str, csrf: &str) -> bool;
    async fn logout(&self, token: &str) -> Result<(), WebError>;
    async fn console(&self, request: ConsoleRequest) -> Result<ConsoleView, WebError>;
    async fn query(&self, operation: ApiQuery) -> Result<Value, WebError>;
    async fn mutate(
        &self,
        principal: &Principal,
        operation: ApiMutation,
        body: Value,
    ) -> Result<MutationOutput, WebError>;
    async fn raw_history(&self, query: HistoryQuery, export: bool)
    -> Result<HistoryPage, WebError>;
    async fn history_series(&self, query: HistoryQuery) -> Result<Value, WebError>;
    async fn semantic_history(&self, query: HistoryQuery) -> Result<SemanticHistoryPage, WebError>;
}

#[derive(Clone, Debug)]
pub enum ApiQuery {
    Session,
    Named {
        route: String,
        params: HashMap<String, String>,
    },
}

#[derive(Clone, Debug)]
pub enum ApiMutation {
    Named {
        method: Method,
        route: String,
        params: HashMap<String, String>,
        expected_revision: Option<i64>,
    },
}

#[derive(Clone, Debug)]
pub struct MutationOutput {
    pub status: StatusCode,
    pub body: Value,
}

impl MutationOutput {
    #[must_use]
    pub fn ok(body: Value) -> Self {
        Self {
            status: StatusCode::OK,
            body,
        }
    }

    #[must_use]
    pub fn created(body: Value) -> Self {
        Self {
            status: StatusCode::CREATED,
            body,
        }
    }

    #[must_use]
    pub fn accepted(body: Value) -> Self {
        Self {
            status: StatusCode::ACCEPTED,
            body,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
    pub signal_ref: Option<String>,
    pub edge_node_id: Option<String>,
    pub bucket_ms: Option<i64>,
}

#[derive(Clone)]
struct AppState {
    config: WebConfig,
    application: Arc<dyn WebApplication>,
}

pub fn router(config: WebConfig, application: Arc<dyn WebApplication>) -> Router {
    let state = AppState {
        config,
        application,
    };
    let mut app = Router::new()
        .route("/", get(root))
        .route("/login", get(login_page).post(login_form))
        .route("/logout", post(logout_form))
        .route("/password", get(password_page).post(password_form))
        .route("/static/edge.css", get(edge_css))
        .route("/static/console.js", get(console_js))
        .route("/static/pinkietech-mark.svg", get(mark_svg))
        .route(
            "/api/v1/session",
            post(api_login).get(api_session).delete(api_logout),
        )
        .route("/api/v1/history", get(history_json))
        .route("/api/v1/history/series", get(history_series))
        .route("/api/v1/history.csv", get(history_csv))
        .route("/api/v1/semantic-history.csv", get(semantic_history_csv))
        .route("/api/v1/mapping-previews", post(api_mutation))
        .route(
            "/api/v1/{*path}",
            get(api_query)
                .post(api_mutation)
                .put(api_mutation)
                .delete(api_mutation),
        )
        .route("/console/{*path}", post(console_mutation));

    for path in router::CONSOLE_GET_ROUTES {
        app = app.route(path, get(console_page));
    }
    app.with_state(state)
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn(security_headers))
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if headers
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value == "application/json")
    {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert("content-security-policy", HeaderValue::from_static(
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
    ));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    response
}

async fn root() -> Redirect {
    Redirect::to("/status")
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate<'a> {
    error: &'a str,
}

async fn login_page() -> Result<Html<String>, WebError> {
    let template = LoginTemplate { error: "" };
    Ok(Html(template.render().map_err(internal)?))
}

#[derive(Template)]
#[template(path = "console.html")]
struct ConsoleTemplate<'a> {
    title: &'a str,
    page: &'a str,
    sensor_view: &'a str,
    csrf: &'a str,
    display_name: &'a str,
    role: &'a str,
    is_owner: bool,
    is_admin: bool,
    view: ConsoleView,
}

#[derive(Template)]
#[template(path = "password.html")]
struct PasswordTemplate<'a> {
    csrf: &'a str,
}

async fn console_page(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, WebError> {
    let Some(token) = cookie(request.headers(), SESSION_COOKIE) else {
        return Ok(Redirect::to("/login").into_response());
    };
    let Ok(principal) = state.application.authenticate(&token).await else {
        return Ok(Redirect::to("/login").into_response());
    };
    if principal.must_change_password {
        return Ok(Redirect::to("/login").into_response());
    }
    let csrf = cookie(request.headers(), CSRF_COOKIE).unwrap_or_default();
    let path = request.uri().path();
    if path == "/monitor" || path == "/signals" {
        return Ok(Redirect::to("/sensors").into_response());
    }
    let page = navigation_page(path);
    let title = console_title(path);
    let sensor_view = if path == "/sensors" { "list" } else { "" };
    let query = request
        .uri()
        .query()
        .map(|value| serde_urlencoded::from_str(value).unwrap_or_default())
        .unwrap_or_default();
    let view = state
        .application
        .console(ConsoleRequest {
            path: path.to_owned(),
            query,
            principal: principal.clone(),
        })
        .await?;
    Ok(Html(
        ConsoleTemplate {
            title,
            page,
            sensor_view,
            csrf: &csrf,
            display_name: &principal.display_name,
            role: &principal.role,
            is_owner: principal.role == "system_admin",
            is_admin: principal.role == "admin" || principal.role == "system_admin",
            view,
        }
        .render()
        .map_err(internal)?,
    )
    .into_response())
}

fn navigation_page(path: &str) -> &str {
    if path.starts_with("/equipment/") {
        "equipment"
    } else {
        path.trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or("status")
    }
}

fn console_title(path: &str) -> &str {
    match navigation_page(path) {
        "status" => "システム概要",
        "sensors" => "センサー一覧",
        "logs" => "受信履歴",
        "equipment" => "機器管理",
        "output" => "外部出力",
        "audit" => "変更履歴",
        "accounts" => "アカウント",
        "system" => "システム",
        _ => "IoTKit Console",
    }
}

async fn password_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = cookie(&headers, SESSION_COOKIE) else {
        return Redirect::to("/login").into_response();
    };
    if state.application.authenticate(&token).await.is_err() {
        return Redirect::to("/login").into_response();
    }
    let csrf = cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    match (PasswordTemplate { csrf: &csrf }).render() {
        Ok(html) => Html(html).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn password_form(State(state): State<AppState>, request: Request) -> Response {
    let headers = request.headers().clone();
    let Some(token) = cookie(&headers, SESSION_COOKIE) else {
        return Redirect::to("/login").into_response();
    };
    let body = match axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let form: HashMap<String, String> = match serde_urlencoded::from_bytes(&body) {
        Ok(form) => form,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let principal = match require_mutation_form(
        &state,
        &headers,
        form.get("_csrf").map(String::as_str),
    )
    .await
    {
        Ok(principal) => principal,
        Err(error) => return error.into_response(),
    };
    if authorize_mutation(&principal, "/password").is_err()
        || state
            .application
            .mutate(
                &principal,
                ApiMutation::Named {
                    method: Method::POST,
                    route: "/password".into(),
                    params: HashMap::new(),
                    expected_revision: None,
                },
                serde_json::to_value(form).unwrap_or_else(|_| json!({})),
            )
            .await
            .is_err()
    {
        return (
            StatusCode::BAD_REQUEST,
            "パスワードを変更できませんでした。",
        )
            .into_response();
    }
    let _ = state.application.logout(&token).await;
    let mut response = Redirect::to("/login").into_response();
    let _ = clear_session_cookies(response.headers_mut(), &state.config);
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginInput {
    login_id: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginForm {
    login_id: String,
    password: String,
    #[serde(rename = "_csrf")]
    _csrf: Option<String>,
}

async fn api_login(State(state): State<AppState>, request: Request) -> Result<Response, WebError> {
    require_origin(&state.config, request.headers())?;
    let body = axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES)
        .await
        .map_err(|_| {
            WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "request body is too large",
            )
        })?;
    let input: LoginInput = serde_json::from_slice(&body).map_err(|_| {
        WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "request body is invalid",
        )
    })?;
    login_response(&state, &input.login_id, &input.password, false).await
}

async fn login_form(State(state): State<AppState>, request: Request) -> Response {
    if require_origin(&state.config, request.headers()).is_err() {
        return (StatusCode::FORBIDDEN, "この接続元からログインできません。").into_response();
    }
    let body = match axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::BAD_REQUEST, "入力内容を確認してください。").into_response(),
    };
    let input: LoginForm = match serde_urlencoded::from_bytes(&body) {
        Ok(input) => input,
        Err(_) => return (StatusCode::BAD_REQUEST, "入力内容を確認してください。").into_response(),
    };
    let _ = input._csrf;
    let session = match state
        .application
        .login(&input.login_id, &input.password)
        .await
    {
        Ok(session) => session,
        Err(error) if error.status == StatusCode::TOO_MANY_REQUESTS => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "ログインを続けて試行できません。しばらく待ってください。",
            )
                .into_response();
        }
        Err(_) => {
            let html = LoginTemplate {
                error: "ログインIDまたはパスワードが正しくありません。",
            }
            .render()
            .unwrap_or_else(|_| "画面を表示できません".to_owned());
            return (StatusCode::UNAUTHORIZED, Html(html)).into_response();
        }
    };
    let mut response = Redirect::to(if session.principal.must_change_password {
        "/password"
    } else {
        "/status"
    })
    .into_response();
    if append_session_cookies(response.headers_mut(), &session, &state.config).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    response
}

async fn login_response(
    state: &AppState,
    username: &str,
    password: &str,
    browser: bool,
) -> Result<Response, WebError> {
    let session = state.application.login(username, password).await?;
    let mut response = if browser {
        Redirect::to(if session.principal.must_change_password {
            "/password"
        } else {
            "/status"
        })
        .into_response()
    } else {
        (
            StatusCode::CREATED,
            Json(json!({"csrf_token":session.csrf,"account":session.principal})),
        )
            .into_response()
    };
    append_session_cookies(response.headers_mut(), &session, &state.config)?;
    Ok(response)
}

async fn api_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, WebError> {
    let principal = api_auth(&state, &headers).await?;
    Ok(Json(json!({"account":principal})))
}

async fn api_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, WebError> {
    require_mutation(&state, &headers).await?;
    if let Some(token) = cookie(&headers, SESSION_COOKIE) {
        state.application.logout(&token).await?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    clear_session_cookies(response.headers_mut(), &state.config)?;
    Ok(response)
}

async fn logout_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, WebError> {
    if cookie(&headers, SESSION_COOKIE).is_none() {
        let mut response = Redirect::to("/login").into_response();
        clear_session_cookies(response.headers_mut(), &state.config)?;
        return Ok(response);
    }
    require_mutation_form(&state, &headers, form.get("_csrf").map(String::as_str)).await?;
    if let Some(token) = cookie(&headers, SESSION_COOKIE) {
        state.application.logout(&token).await?;
    }
    let mut response = Redirect::to("/login").into_response();
    clear_session_cookies(response.headers_mut(), &state.config)?;
    Ok(response)
}

async fn api_query(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(mut params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, WebError> {
    let principal = api_auth(&state, &headers).await?;
    let route = format!("/api/v1/{path}");
    let path_params = match_known_route(&route, router::API_GET_ROUTES).ok_or_else(not_found)?;
    params.extend(path_params);
    authorize_query(&principal, &route)?;
    let body = state
        .application
        .query(ApiQuery::Named { route, params })
        .await?;
    let mut response = Json(body.clone()).into_response();
    if let Some(revision) = body.get("revision").and_then(Value::as_i64) {
        response.headers_mut().insert(
            header::ETAG,
            HeaderValue::from_str(&format!("\"{revision}\"")).map_err(internal)?,
        );
    }
    Ok(response)
}

async fn api_mutation(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, WebError> {
    let principal = require_mutation(&state, request.headers()).await?;
    let method = request.method().clone();
    let route = request.uri().path().to_owned();
    let clears_session = route == "/api/v1/session/password";
    let known = match *request.method() {
        Method::POST => router::API_POST_ROUTES,
        Method::PUT => router::API_PUT_ROUTES,
        Method::DELETE => router::API_DELETE_ROUTES,
        _ => &[],
    };
    let params = match_known_route(&route, known).ok_or_else(not_found)?;
    authorize_mutation(&principal, &route)?;
    let expected_revision = if requires_revision_precondition(&method, &route) {
        Some(parse_if_match(request.headers())?)
    } else {
        None
    };
    let body = axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES)
        .await
        .map_err(|_| {
            WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "invalid request body",
            )
        })?;
    let value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body).map_err(|_| {
            WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_json",
                "request body must be valid JSON",
            )
        })?
    };
    let output = state
        .application
        .mutate(
            &principal,
            ApiMutation::Named {
                method,
                route,
                params,
                expected_revision,
            },
            value,
        )
        .await?;
    let output_revision = output.body.get("revision").and_then(Value::as_i64);
    let mut response = (output.status, Json(output.body)).into_response();
    if let Some(revision) = output_revision {
        response.headers_mut().insert(
            header::ETAG,
            HeaderValue::from_str(&format!("\"{revision}\"")).map_err(internal)?,
        );
    }
    if clears_session {
        clear_session_cookies(response.headers_mut(), &state.config)?;
    }
    Ok(response)
}

async fn console_mutation(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, WebError> {
    let headers = request.headers().clone();
    let route = request.uri().path().to_owned();
    let params = match_known_route(&route, router::CONSOLE_POST_ROUTES).ok_or_else(not_found)?;
    let body = axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES)
        .await
        .map_err(|_| {
            WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "invalid form body",
            )
        })?;
    let form: HashMap<String, String> = serde_urlencoded::from_bytes(&body)
        .map_err(|_| WebError::new(StatusCode::BAD_REQUEST, "invalid_form", "invalid form body"))?;
    let principal =
        require_mutation_form(&state, &headers, form.get("_csrf").map(String::as_str)).await?;
    authorize_mutation(&principal, &route)?;
    let result = state
        .application
        .mutate(
            &principal,
            ApiMutation::Named {
                method: Method::POST,
                route,
                params,
                expected_revision: None,
            },
            serde_json::to_value(form).map_err(internal)?,
        )
        .await;
    let target = console_result_location(&headers, result.as_ref().err());
    if let Err(error) = result
        && error.status == StatusCode::PRECONDITION_FAILED
    {
        return Ok((
            StatusCode::PRECONDITION_FAILED,
            "画面を再読み込みして、もう一度操作してください。",
        )
            .into_response());
    }
    Ok(Redirect::to(&target).into_response())
}

fn console_result_location(headers: &HeaderMap, error: Option<&WebError>) -> String {
    let mut target = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| url::Url::parse(value).ok())
        .unwrap_or_else(|| {
            url::Url::parse("http://localhost/status").expect("static URL is valid")
        });
    target.set_fragment(None);
    {
        let mut query = target.query_pairs_mut();
        if let Some(error) = error {
            query.append_pair("error", error.code);
        } else {
            query.append_pair("saved", "1");
        }
    }
    match target.query() {
        Some(query) => format!("{}?{query}", target.path()),
        None => target.path().into(),
    }
}

async fn history_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, WebError> {
    api_auth(&state, &headers).await?;
    validate_history_query(&query)?;
    let page = state.application.raw_history(query, false).await?;
    let mut response = json!({"records":page.rows,"has_more":page.has_more});
    if let Some(cursor) = page.next_cursor {
        response["next_cursor"] = json!(cursor);
    }
    Ok(Json(response))
}

async fn history_series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, WebError> {
    api_auth(&state, &headers).await?;
    validate_history_query(&query)?;
    if query.signal_ref.as_deref().unwrap_or("").is_empty() {
        return Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "signal_ref is required",
        )
        .field("signal_ref"));
    }
    let bucket_ms = query.bucket_ms.unwrap_or_default();
    let from = parse_history_time(query.from.as_deref(), "from")?;
    let to = parse_history_time(query.to.as_deref(), "to")?;
    if bucket_ms <= 0 || (to - from + bucket_ms - 1) / bucket_ms > 1_000 {
        return Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "history series bucket count must be between 1 and 1000",
        )
        .field("bucket_ms"));
    }
    Ok(Json(state.application.history_series(query).await?))
}

async fn history_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Response, WebError> {
    api_auth(&state, &headers).await?;
    validate_history_export_query(&query)?;
    let page = state.application.raw_history(query, true).await?;
    if page.has_more || page.rows.len() > MAX_HISTORY_EXPORT_ROWS {
        return Err(WebError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "history_export_too_large",
            "history export exceeds 100000 rows",
        ));
    }
    Ok(csv_response(raw_csv(&page.rows), "iotkit-history.csv"))
}

async fn semantic_history_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Response, WebError> {
    api_auth(&state, &headers).await?;
    validate_history_export_query(&query)?;
    let page = state.application.semantic_history(query).await?;
    if page.has_more || page.rows.len() > MAX_HISTORY_EXPORT_ROWS {
        return Err(WebError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "semantic_history_export_too_large",
            "processed history export exceeds 100000 rows",
        ));
    }
    Ok(csv_response(
        semantic_csv(&page.rows),
        "iotkit-processed-history.csv",
    ))
}

fn validate_history_query(query: &HistoryQuery) -> Result<(), WebError> {
    validate_history_range(query)?;
    let limit = query.limit.unwrap_or(200);
    if limit == 0 || limit > MAX_HISTORY_PAGE {
        return Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "limit must be between 1 and 1000",
        )
        .field("limit"));
    }
    Ok(())
}

fn validate_history_export_query(query: &HistoryQuery) -> Result<(), WebError> {
    validate_history_range(query)?;
    if query.cursor.is_some() {
        return Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "cursor is not allowed for CSV export",
        )
        .field("cursor"));
    }
    Ok(())
}

fn validate_history_range(query: &HistoryQuery) -> Result<(), WebError> {
    let from = parse_history_time(query.from.as_deref(), "from")?;
    let to = parse_history_time(query.to.as_deref(), "to")?;
    if to <= from {
        return Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "to must be greater than from",
        )
        .field("to"));
    }
    const MAX_RANGE_MS: i64 = 31 * 24 * 60 * 60 * 1_000;
    if to - from > MAX_RANGE_MS {
        return Err(WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "history range must not exceed 31 days",
        )
        .field("to"));
    }
    Ok(())
}

fn parse_history_time(value: Option<&str>, field: &'static str) -> Result<i64, WebError> {
    let value = value.filter(|value| !value.is_empty()).ok_or_else(|| {
        WebError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            format!("{field} is required"),
        )
        .field(field)
    })?;
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value >= 0)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::BAD_REQUEST,
                "invalid_query",
                format!("{field} must be a non-negative Unix millisecond timestamp"),
            )
            .field(field)
        })
}

fn raw_csv(rows: &[RawHistoryRow]) -> Vec<u8> {
    let mut output = String::from(
        "\u{feff}received_at,observed_at,edge_node_id,signal_ref,series_key,sensor_name,values,unit\r\n",
    );
    for row in rows {
        let fields = [
            &row.received_at,
            &row.observed_at,
            &row.edge_node_id,
            &row.signal_ref,
            &row.series_key,
            &row.sensor_name,
            &row.values,
            &row.unit,
        ];
        output.push_str(
            &fields
                .into_iter()
                .map(|v| csv_field(v))
                .collect::<Vec<_>>()
                .join(","),
        );
        output.push_str("\r\n");
    }
    output.into_bytes()
}

fn semantic_csv(rows: &[SemanticHistoryRow]) -> Vec<u8> {
    let mut output = String::from(
        "\u{feff}observed_at,processed_at,edge_node_id,signal_ref,sensor_name,rule_name,kind,value,unit,series_id,sequence,observation_id,rule_revision,calibration_revision,source_pub_seq\r\n",
    );
    for row in rows {
        let fields = [
            csv_field(&row.observed_at),
            csv_field(&row.processed_at),
            csv_field(&row.edge_node_id),
            csv_field(&row.signal_ref),
            csv_field(&row.sensor_name),
            csv_field(&row.rule_name),
            csv_field(&row.kind),
            csv_field(&row.value),
            csv_field(&row.unit),
            csv_field(&row.series_id),
            row.sequence.to_string(),
            csv_field(&row.observation_id),
            row.rule_revision.to_string(),
            row.calibration_revision.to_string(),
            row.source_pub_seq.to_string(),
        ];
        output.push_str(&fields.join(","));
        output.push_str("\r\n");
    }
    output.into_bytes()
}

fn csv_field(value: &str) -> String {
    let trimmed = value.trim_start_matches([' ', '\t', '\r', '\n']);
    let safe = if matches!(trimmed.chars().next(), Some('=' | '+' | '-' | '@')) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    if safe.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

fn csv_response(body: Vec<u8>, filename: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response()
}

async fn api_auth(state: &AppState, headers: &HeaderMap) -> Result<Principal, WebError> {
    let token = cookie(headers, SESSION_COOKIE).ok_or_else(unauthenticated)?;
    let principal = state
        .application
        .authenticate(&token)
        .await
        .map_err(|_| unauthenticated())?;
    if principal.must_change_password {
        return Err(WebError::new(
            StatusCode::FORBIDDEN,
            "password_change_required",
            "password change is required",
        ));
    }
    Ok(principal)
}

async fn require_mutation(state: &AppState, headers: &HeaderMap) -> Result<Principal, WebError> {
    require_origin(&state.config, headers)?;
    let principal = api_auth(state, headers).await?;
    let header_csrf = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(csrf_error)?;
    let token = cookie(headers, SESSION_COOKIE).ok_or_else(unauthenticated)?;
    if !state.application.validate_csrf(&token, header_csrf).await {
        return Err(csrf_error());
    }
    Ok(principal)
}

async fn require_mutation_form(
    state: &AppState,
    headers: &HeaderMap,
    form_csrf: Option<&str>,
) -> Result<Principal, WebError> {
    require_origin(&state.config, headers)?;
    let token = cookie(headers, SESSION_COOKIE).ok_or_else(unauthenticated)?;
    let principal = state
        .application
        .authenticate(&token)
        .await
        .map_err(|_| unauthenticated())?;
    let supplied = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .or(form_csrf)
        .ok_or_else(csrf_error)?;
    if !state.application.validate_csrf(&token, supplied).await {
        return Err(csrf_error());
    }
    Ok(principal)
}

fn require_origin(config: &WebConfig, headers: &HeaderMap) -> Result<(), WebError> {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let referer = headers.get(header::REFERER).and_then(|v| v.to_str().ok());
    let matches = origin.map_or_else(
        || {
            referer
                .and_then(|value| url::Url::parse(value).ok())
                .is_some_and(|value| value.origin().ascii_serialization() == config.public_origin)
        },
        |value| value == config.public_origin,
    );
    if !matches {
        return Err(origin_error());
    }
    Ok(())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.to_owned()))
}

fn append_session_cookies(
    headers: &mut HeaderMap,
    session: &LoginSession,
    config: &WebConfig,
) -> Result<(), WebError> {
    append_cookie(
        headers,
        SESSION_COOKIE,
        &session.token,
        true,
        86_400,
        config,
    )?;
    append_cookie(headers, CSRF_COOKIE, &session.csrf, false, 86_400, config)
}

fn clear_session_cookies(headers: &mut HeaderMap, config: &WebConfig) -> Result<(), WebError> {
    append_cookie(headers, SESSION_COOKIE, "", true, -1, config)?;
    append_cookie(headers, CSRF_COOKIE, "", false, -1, config)
}

fn append_cookie(
    headers: &mut HeaderMap,
    name: &str,
    value: &str,
    http_only: bool,
    max_age: i64,
    config: &WebConfig,
) -> Result<(), WebError> {
    let mut cookie = format!("{name}={value}; Path=/; Max-Age={max_age}; SameSite=Strict");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if config.secure_cookies {
        cookie.push_str("; Secure");
    }
    headers.append(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(internal)?,
    );
    Ok(())
}

fn unauthenticated() -> WebError {
    WebError::new(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        "authentication is required",
    )
}

fn not_found() -> WebError {
    WebError::new(StatusCode::NOT_FOUND, "not_found", "route was not found")
}

fn match_known_route(actual: &str, patterns: &[&str]) -> Option<HashMap<String, String>> {
    patterns
        .iter()
        .find_map(|pattern| match_route(actual, pattern))
}

fn requires_revision_precondition(method: &Method, route: &str) -> bool {
    if !route.starts_with("/api/") {
        return false;
    }
    matches!(*method, Method::PUT | Method::DELETE)
        || (method == Method::POST
            && (route.ends_with("/semantic-rules")
                || route.ends_with("/counter-resets")
                || route.ends_with("/stop")
                || route.ends_with("/start")))
}

fn parse_if_match(headers: &HeaderMap) -> Result<i64, WebError> {
    let Some(raw) = headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
    else {
        return Err(WebError::new(
            StatusCode::PRECONDITION_REQUIRED,
            "precondition_required",
            "If-Match is required",
        ));
    };
    let revision = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::PRECONDITION_FAILED,
                "revision_mismatch",
                "resource revision does not match",
            )
        })?;
    Ok(revision)
}

fn match_route(actual: &str, pattern: &str) -> Option<HashMap<String, String>> {
    let actual: Vec<_> = actual.trim_matches('/').split('/').collect();
    let pattern: Vec<_> = pattern.trim_matches('/').split('/').collect();
    if actual.len() != pattern.len() {
        return None;
    }
    let mut params = HashMap::new();
    for (actual, pattern) in actual.into_iter().zip(pattern) {
        if let Some(name) = pattern
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            if actual.is_empty() {
                return None;
            }
            params.insert(name.into(), actual.into());
        } else if actual != pattern {
            return None;
        }
    }
    Some(params)
}
fn authorize_mutation(principal: &Principal, route: &str) -> Result<(), WebError> {
    let allowed = if route == "/password" || route == "/api/v1/session/password" {
        true
    } else if route.starts_with("/console/accounts/") || route.starts_with("/api/v1/accounts") {
        principal.role == "system_admin"
    } else {
        principal.role == "admin" || principal.role == "system_admin"
    };
    allowed.then_some(()).ok_or_else(|| {
        WebError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the account is not allowed to perform this operation",
        )
    })
}

fn authorize_query(principal: &Principal, route: &str) -> Result<(), WebError> {
    let allowed = if route == "/api/v1/accounts" {
        principal.role == "system_admin"
    } else if route == "/api/v1/setup/devices" {
        principal.role == "admin" || principal.role == "system_admin"
    } else {
        true
    };
    allowed.then_some(()).ok_or_else(|| {
        WebError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the account is not allowed to perform this operation",
        )
    })
}
fn csrf_error() -> WebError {
    WebError::new(
        StatusCode::FORBIDDEN,
        "csrf_forbidden",
        "CSRF token is missing or invalid",
    )
}
fn origin_error() -> WebError {
    WebError::new(
        StatusCode::FORBIDDEN,
        "origin_forbidden",
        "request origin is not allowed",
    )
}
fn internal<E: std::fmt::Display>(_: E) -> WebError {
    WebError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "internal server error",
    )
}

async fn edge_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_bytes!("../../frontend/static/edge.css").as_slice(),
    )
}
async fn console_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_bytes!("../../frontend/static/console.js").as_slice(),
    )
}
async fn mark_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_bytes!("../../frontend/static/pinkietech-mark.svg").as_slice(),
    )
}

pub mod test_support {
    use super::*;

    pub struct StubApplication {
        authenticated: bool,
        role: &'static str,
        rate_limited: bool,
    }
    impl Default for StubApplication {
        fn default() -> Self {
            Self {
                authenticated: false,
                role: "admin",
                rate_limited: false,
            }
        }
    }
    impl StubApplication {
        pub fn authenticated() -> Self {
            Self {
                authenticated: true,
                role: "admin",
                rate_limited: false,
            }
        }
        pub fn system_admin() -> Self {
            Self {
                authenticated: true,
                role: "system_admin",
                rate_limited: false,
            }
        }
        pub fn rate_limited() -> Self {
            Self {
                authenticated: false,
                role: "admin",
                rate_limited: true,
            }
        }
    }

    #[async_trait]
    impl WebApplication for StubApplication {
        async fn login(&self, username: &str, password: &str) -> Result<LoginSession, WebError> {
            if self.rate_limited {
                return Err(WebError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "login_rate_limited",
                    "login cannot be attempted again yet",
                ));
            }
            if username != "admin" || password != "correct" {
                return Err(WebError::new(
                    StatusCode::UNAUTHORIZED,
                    "invalid_credentials",
                    "invalid username or password",
                ));
            }
            Ok(LoginSession {
                token: "valid".into(),
                csrf: "csrf".into(),
                principal: Principal {
                    account_ref: "acct-admin".into(),
                    login_id: "admin".into(),
                    display_name: "Administrator".into(),
                    role: self.role.into(),
                    state: "active".into(),
                    must_change_password: false,
                    revision: 1,
                    created_at: 0,
                    updated_at: 0,
                },
            })
        }
        async fn authenticate(&self, token: &str) -> Result<Principal, WebError> {
            if token != "valid" && !self.authenticated {
                return Err(unauthenticated());
            }
            Ok(Principal {
                account_ref: "acct-admin".into(),
                login_id: "admin".into(),
                display_name: "Administrator".into(),
                role: self.role.into(),
                state: "active".into(),
                must_change_password: false,
                revision: 1,
                created_at: 0,
                updated_at: 0,
            })
        }
        async fn validate_csrf(&self, token: &str, csrf: &str) -> bool {
            token == "valid" && csrf == "csrf"
        }
        async fn logout(&self, _token: &str) -> Result<(), WebError> {
            Ok(())
        }
        async fn console(&self, request: ConsoleRequest) -> Result<ConsoleView, WebError> {
            let signal = ConsoleSignal {
                signal_ref: "signal-01".into(),
                device_ref: "device-01".into(),
                edge_node_id: "factory-edge-01".into(),
                name: "乾燥炉入口 温度".into(),
                sensor_type: "温度".into(),
                value: "28.5".into(),
                unit: "℃".into(),
                status_label: "受信中".into(),
                status_class: "receiving".into(),
                profile_complete: true,
                rules: vec![ConsoleRule {
                    rule_id: "rule-01".into(),
                    display_name: "現在温度".into(),
                    kind: "numeric".into(),
                }],
            };
            let selected_edge_node =
                request
                    .path
                    .contains("/edge-nodes/edge-node-02")
                    .then(|| ConsoleEdgeNode {
                        edge_node_ref: "edge-node-02".into(),
                        edge_node_id: "assembly-edge-02".into(),
                        name: "assembly-edge-02".into(),
                        location: "組立ライン".into(),
                        state_label: "未登録".into(),
                        state_class: "needs-setup".into(),
                        can_activate: true,
                    });
            let selected_signal = request
                .path
                .contains("/sensors/signal-01")
                .then(|| signal.clone());
            Ok(ConsoleView {
                edge_nodes: vec![
                    ConsoleEdgeNode {
                        edge_node_ref: "edge-node-01".into(),
                        edge_node_id: "factory-edge-01".into(),
                        name: "factory-edge-01".into(),
                        location: "乾燥炉".into(),
                        state_label: "登録済み".into(),
                        state_class: "configured".into(),
                        can_activate: false,
                    },
                    ConsoleEdgeNode {
                        edge_node_ref: "edge-node-02".into(),
                        edge_node_id: "assembly-edge-02".into(),
                        name: "assembly-edge-02".into(),
                        location: "組立ライン".into(),
                        state_label: "未登録".into(),
                        state_class: "needs-setup".into(),
                        can_activate: true,
                    },
                ],
                signals: vec![signal],
                selected_edge_node,
                selected_signal,
                history: vec![RawHistoryRow {
                    received_at: "1735689601000".into(),
                    observed_at: "1735689600000".into(),
                    edge_node_id: "factory-edge-01".into(),
                    ledger_epoch: "epoch-01".into(),
                    pub_seq: 1,
                    signal_ref: "signal-01".into(),
                    series_key: "temperature".into(),
                    sensor_name: "乾燥炉入口 温度".into(),
                    values: "[28.5]".into(),
                    value_type: "number".into(),
                    unit: "℃".into(),
                    decimal_places: 1,
                    display_value_kind: "numeric".into(),
                }],
                history_chart_path: "M0 90 L120 60 L240 70 L360 20".into(),
                history_raw_export_url:
                    "/api/v1/history.csv?from=0&to=2678400000&signal_ref=signal-01".into(),
                history_processed_export_url:
                    "/api/v1/semantic-history.csv?from=0&to=2678400000&signal_ref=signal-01".into(),
                outputs: vec![
                    ConsoleOutput {
                        profile_id: String::new(),
                        adapter_id: "iotkit.mqtt-json.v1".into(),
                        display_name: "汎用MQTT JSONで送る".into(),
                        description: "意味づけ済みの値をIoTKit共通形式で送ります。".into(),
                        active: false,
                        bindings: Vec::new(),
                    },
                    ConsoleOutput {
                        profile_id: String::new(),
                        adapter_id: "pinikiet.mqtt.v1".into(),
                        display_name: "Pinikietへ送る".into(),
                        description: "累積値・状態・アラームをPinikiet契約へ変換します。".into(),
                        active: false,
                        bindings: Vec::new(),
                    },
                ],
                accounts: vec![ConsoleAccount {
                    account_ref: "acct-owner".into(),
                    login_id: "owner".into(),
                    display_name: "システム管理者".into(),
                    role: "system_admin".into(),
                    state: "active".into(),
                    revision: 1,
                }],
                audit: vec![ConsoleAudit {
                    occurred_at: "2025-01-01T00:00:00Z".into(),
                    actor: "admin".into(),
                    action: "設定を変更".into(),
                    target: "signal-01".into(),
                }],
                storage: ConsoleStorage {
                    profile_label: "組み込みSQLite".into(),
                    raw_count: 1,
                    pending_output_count: 0,
                    used_percent: 12,
                    host_capacity_available: true,
                    retention_note: "rawの自動削除は無効".into(),
                    diagnostic_messages: vec!["確認が必要なことはありません".into()],
                },
                ..ConsoleView::default()
            })
        }
        async fn query(&self, operation: ApiQuery) -> Result<Value, WebError> {
            Ok(json!({"operation":format!("{operation:?}"), "revision": 1}))
        }
        async fn mutate(
            &self,
            _principal: &Principal,
            operation: ApiMutation,
            _body: Value,
        ) -> Result<MutationOutput, WebError> {
            if let ApiMutation::Named {
                expected_revision: Some(expected),
                ..
            } = &operation
                && *expected != 1
            {
                return Err(WebError::new(
                    StatusCode::PRECONDITION_FAILED,
                    "revision_mismatch",
                    "resource revision does not match",
                ));
            }
            let status = match &operation {
                ApiMutation::Named { route, .. }
                    if route.ends_with("/activation")
                        || route.ends_with("/counter-resets")
                        || route.ends_with("/stop") =>
                {
                    StatusCode::ACCEPTED
                }
                ApiMutation::Named { route, .. }
                    if route == "/api/v1/accounts"
                        || route == "/api/v1/export-profiles"
                        || route.ends_with("/semantic-rules") =>
                {
                    StatusCode::CREATED
                }
                _ => StatusCode::OK,
            };
            Ok(MutationOutput {
                status,
                body: json!({"operation":format!("{operation:?}"), "revision": 2}),
            })
        }
        async fn raw_history(
            &self,
            _query: HistoryQuery,
            _export: bool,
        ) -> Result<HistoryPage, WebError> {
            Ok(HistoryPage {
                rows: vec![RawHistoryRow {
                    received_at: "1735689600000".into(),
                    observed_at: "1735689600000".into(),
                    edge_node_id: "edge-1".into(),
                    ledger_epoch: "epoch-1".into(),
                    pub_seq: 1,
                    signal_ref: "signal-1".into(),
                    series_key: "temperature".into(),
                    sensor_name: "'=danger".into(),
                    values: "[1]".into(),
                    value_type: "number".into(),
                    unit: "C".into(),
                    decimal_places: 1,
                    display_value_kind: "numeric".into(),
                }],
                next_cursor: None,
                has_more: false,
            })
        }
        async fn history_series(&self, _query: HistoryQuery) -> Result<Value, WebError> {
            Ok(json!({"series":[]}))
        }
        async fn semantic_history(
            &self,
            _query: HistoryQuery,
        ) -> Result<SemanticHistoryPage, WebError> {
            Ok(SemanticHistoryPage {
                rows: vec![SemanticHistoryRow {
                    observed_at: "2025-01-01T00:00:00Z".into(),
                    processed_at: "2025-01-01T00:00:01Z".into(),
                    edge_node_id: "edge-1".into(),
                    signal_ref: "signal-1".into(),
                    sensor_name: "Temperature".into(),
                    rule_name: "=unsafe".into(),
                    kind: "number".into(),
                    value: "1".into(),
                    unit: "C".into(),
                    series_id: "series-1".into(),
                    sequence: 1,
                    observation_id: "observation-1".into(),
                    rule_revision: 1,
                    calibration_revision: 1,
                    source_pub_seq: 1,
                }],
                has_more: false,
            })
        }
    }
}
