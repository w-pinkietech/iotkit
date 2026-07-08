use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::{
    NewOperatorToken, OpContext, OpDescriptor, OpError, Tier, TokenKind, issue_token, revoke_token,
};

use super::{required_str, required_string_array, target_string_array};

pub fn issue_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "operator_token.issue",
        tier: Tier::Construction,
        bulk_escalates: false,
        params_schema: issue_schema,
        targets: issue_targets,
        preconditions: issue_preconditions,
        dry_run: issue_dry_run,
        execute: issue_execute,
    }
}

pub fn revoke_descriptor() -> OpDescriptor {
    OpDescriptor {
        name: "operator_token.revoke",
        tier: Tier::Daily,
        bulk_escalates: true,
        params_schema: revoke_schema,
        targets: revoke_targets,
        preconditions: revoke_preconditions,
        dry_run: revoke_dry_run,
        execute: revoke_execute,
    }
}

fn issue_schema() -> Value {
    json!({ "required": ["name", "kind", "tier_ceiling"] })
}

fn revoke_schema() -> Value {
    json!({ "required": ["token_ids"] })
}

fn issue_targets(params: &Value) -> Vec<String> {
    params
        .get("name")
        .and_then(Value::as_str)
        .map(|name| vec![name.to_string()])
        .unwrap_or_default()
}

fn revoke_targets(params: &Value) -> Vec<String> {
    target_string_array(params, "token_ids")
}

fn issue_preconditions(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let token = parse_new_token(ctx.params)?;
    if token.kind == TokenKind::Ai && token.ceiling > Tier::Routine {
        return Err(OpError::Validation(
            "ai token tier ceiling cannot exceed routine".to_string(),
        ));
    }
    Ok(())
}

fn issue_dry_run(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let token = parse_new_token(ctx.params)?;
    Ok(json!({
        "would": "issue_token",
        "name": token.name,
        "kind": token.kind.as_str(),
        "tier_ceiling": token.ceiling.as_str(),
        "expires_at": token.expires_at,
    }))
}

fn issue_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let token = parse_new_token(ctx.params)?;
    let issued = issue_token(tx, &token, ctx.actor_id, ctx.source)?;
    Ok(json!({
        "token_id": issued.token_id,
        "plaintext": issued.plaintext.expose(),
    }))
}

fn revoke_preconditions(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<(), OpError> {
    let token_ids = required_string_array(ctx.params, "token_ids")?;
    if token_ids.is_empty() {
        return Err(OpError::Validation("empty targets".to_string()));
    }
    for token_id in token_ids {
        let alive = tx
            .query_row(
                "SELECT 1 FROM operator_tokens
                 WHERE token_id = ?1 AND revoked_at IS NULL",
                params![token_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !alive {
            return Err(OpError::NotFound);
        }
    }
    Ok(())
}

fn revoke_dry_run(_tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let token_ids = required_string_array(ctx.params, "token_ids")?;
    Ok(json!({
        "would": "revoke_token",
        "count": token_ids.len(),
        "token_ids": token_ids,
    }))
}

fn revoke_execute(tx: &Transaction<'_>, ctx: &OpContext<'_>) -> Result<Value, OpError> {
    let mut revoked = Vec::new();
    for token_id in required_string_array(ctx.params, "token_ids")? {
        revoke_token(tx, &token_id, ctx.actor_id)?;
        revoked.push(token_id);
    }
    Ok(json!({ "revoked": revoked }))
}

fn parse_new_token(params: &Value) -> Result<NewOperatorToken, OpError> {
    let kind = TokenKind::parse(required_str(params, "kind")?)?;
    let ceiling = Tier::parse(required_str(params, "tier_ceiling")?)?;
    let expires_at = match params.get("expires_at") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            value
                .as_i64()
                .ok_or_else(|| OpError::Validation("expires_at must be an integer".to_string()))?,
        ),
    };
    Ok(NewOperatorToken {
        name: required_str(params, "name")?.to_string(),
        kind,
        ceiling,
        is_session: false,
        expires_at,
    })
}
