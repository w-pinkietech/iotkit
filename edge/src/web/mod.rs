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
    http::{HeaderMap, HeaderValue, StatusCode, header},
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

#[derive(Clone, Debug, Serialize)]
pub struct RawHistoryRow {
    pub received_at: String,
    pub observed_at: String,
    pub edge_node_id: String,
    pub signal_ref: String,
    pub series_key: String,
    pub sensor_name: String,
    pub values: String,
    pub unit: String,
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

#[async_trait]
pub trait WebApplication: Send + Sync + 'static {
    async fn login(&self, username: &str, password: &str) -> Result<LoginSession, WebError>;
    async fn authenticate(&self, token: &str) -> Result<Principal, WebError>;
    async fn validate_csrf(&self, token: &str, csrf: &str) -> bool;
    async fn logout(&self, token: &str) -> Result<(), WebError>;
    async fn query(&self, operation: ApiQuery) -> Result<Value, WebError>;
    async fn mutate(&self, operation: ApiMutation, body: Value) -> Result<Value, WebError>;
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
        route: String,
        params: HashMap<String, String>,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
    pub signal_ref: Option<String>,
    pub edge_node_id: Option<String>,
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
            get(api_query).post(api_mutation).delete(api_mutation),
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
    let page = navigation_page(path);
    let title = console_title(path);
    let sensor_view = if path == "/sensors" { "list" } else { "" };
    Ok(Html(
        ConsoleTemplate {
            title,
            page,
            sensor_view,
            csrf: &csrf,
            display_name: &principal.display_name,
            role: &principal.role,
            is_owner: principal.role == "system_admin",
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
                ApiMutation::Named {
                    route: "/password".into(),
                    params: HashMap::new(),
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
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<Value>, WebError> {
    api_auth(&state, &headers).await?;
    Ok(Json(
        state
            .application
            .query(ApiQuery::Named {
                route: format!("/api/v1/{path}"),
                params,
            })
            .await?,
    ))
}

async fn api_mutation(
    State(state): State<AppState>,
    request: Request,
) -> Result<Json<Value>, WebError> {
    let principal = require_mutation(&state, request.headers()).await?;
    let route = request.uri().path().to_owned();
    authorize_mutation(&principal, &route)?;
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
    Ok(Json(
        state
            .application
            .mutate(
                ApiMutation::Named {
                    route,
                    params: HashMap::new(),
                },
                value,
            )
            .await?,
    ))
}

async fn console_mutation(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, WebError> {
    let headers = request.headers().clone();
    let route = request.uri().path().to_owned();
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
    state
        .application
        .mutate(
            ApiMutation::Named {
                route,
                params: HashMap::new(),
            },
            serde_json::to_value(form).map_err(internal)?,
        )
        .await?;
    Ok(Redirect::to(
        headers
            .get(header::REFERER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("/status"),
    )
    .into_response())
}

async fn history_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, WebError> {
    api_auth(&state, &headers).await?;
    validate_history_query(&query)?;
    let page = state.application.raw_history(query, false).await?;
    Ok(Json(
        json!({"items":page.rows,"next_cursor":page.next_cursor}),
    ))
}

async fn history_series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, WebError> {
    api_auth(&state, &headers).await?;
    validate_history_query(&query)?;
    Ok(Json(state.application.history_series(query).await?))
}

async fn history_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Response, WebError> {
    api_auth(&state, &headers).await?;
    validate_history_query(&query)?;
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
    validate_history_query(&query)?;
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
    if query.from.as_deref().unwrap_or("").is_empty() {
        return Err(
            WebError::new(StatusCode::BAD_REQUEST, "invalid_query", "from is required")
                .field("from"),
        );
    }
    if query.to.as_deref().unwrap_or("").is_empty() {
        return Err(
            WebError::new(StatusCode::BAD_REQUEST, "invalid_query", "to is required").field("to"),
        );
    }
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
        include_bytes!("../../internal/edgehttp/static/edge.css").as_slice(),
    )
}
async fn console_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_bytes!("../../internal/edgehttp/static/console.js").as_slice(),
    )
}
async fn mark_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_bytes!("../../internal/edgehttp/static/pinkietech-mark.svg").as_slice(),
    )
}

pub mod test_support {
    use super::*;

    #[derive(Default)]
    pub struct StubApplication {
        authenticated: bool,
    }
    impl StubApplication {
        pub fn authenticated() -> Self {
            Self {
                authenticated: true,
            }
        }
    }

    #[async_trait]
    impl WebApplication for StubApplication {
        async fn login(&self, username: &str, password: &str) -> Result<LoginSession, WebError> {
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
                    role: "admin".into(),
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
                role: "admin".into(),
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
        async fn query(&self, operation: ApiQuery) -> Result<Value, WebError> {
            Ok(json!({"operation":format!("{operation:?}")}))
        }
        async fn mutate(&self, operation: ApiMutation, _body: Value) -> Result<Value, WebError> {
            Ok(json!({"operation":format!("{operation:?}")}))
        }
        async fn raw_history(
            &self,
            _query: HistoryQuery,
            _export: bool,
        ) -> Result<HistoryPage, WebError> {
            Ok(HistoryPage {
                rows: vec![RawHistoryRow {
                    received_at: "2025-01-01T00:00:00Z".into(),
                    observed_at: "2025-01-01T00:00:00Z".into(),
                    edge_node_id: "edge-1".into(),
                    signal_ref: "signal-1".into(),
                    series_key: "temperature".into(),
                    sensor_name: "'=danger".into(),
                    values: "{\"value\":1}".into(),
                    unit: "C".into(),
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
