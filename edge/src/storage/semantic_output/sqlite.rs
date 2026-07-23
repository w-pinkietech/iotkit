async fn create_rule_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    draft: SemanticRuleDraft,
    now: i64,
) -> Result<SemanticRule, StorageError> {
    let signal_ref = ensure_signal_sqlite(
        tx,
        &draft.edge_node_id,
        &draft.series_key,
        now,
    )
    .await?;
    let rule_id = Uuid::new_v4().to_string();
    let series_id = Uuid::new_v4().to_string();
    let spec_json = serde_json::to_vec(&draft.spec)
        .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
    sqlx::query(
        "INSERT INTO semantic_rules(rule_id,signal_ref,display_name,kind,series_id,\
         revision,spec_json,active,created_at) VALUES(?,?,?,?,?,1,?,1,?)",
    )
    .bind(&rule_id)
    .bind(&signal_ref)
    .bind(&draft.display_name)
    .bind(semantic_kind(draft.spec.kind))
    .bind(&series_id)
    .bind(&spec_json)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO semantic_rule_revisions(rule_id,revision,series_id,spec_json,created_at) \
         VALUES(?,1,?,?,?)",
    )
    .bind(&rule_id)
    .bind(&series_id)
    .bind(&spec_json)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO semantic_rule_starts(rule_id,revision,ledger_epoch,start_after_pub_seq) \
         SELECT ?,1,ledger_epoch,accepted_through FROM accepted_cursors WHERE edge_node_id=?",
    )
    .bind(&rule_id)
    .bind(&draft.edge_node_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("INSERT INTO semantic_rule_runtime(rule_id) VALUES(?)")
        .bind(&rule_id)
        .execute(&mut **tx)
        .await?;
    let rule = SemanticRule {
        rule_id,
        signal_ref,
        edge_node_id: draft.edge_node_id,
        series_key: draft.series_key,
        display_name: draft.display_name,
        kind: draft.spec.kind,
        series_id,
        revision: 1,
        active: true,
    };
    auto_bind_rule_sqlite(tx, &rule, now).await?;
    Ok(rule)
}

async fn ensure_signal_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    edge_node_id: &str,
    series_key: &str,
    now: i64,
) -> Result<String, StorageError> {
    if let Some(value) = sqlx::query_scalar(
        "SELECT signal_ref FROM semantic_signals WHERE edge_node_id=? AND series_key=?",
    )
    .bind(edge_node_id)
    .bind(series_key)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(value);
    }
    let signal_ref = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO semantic_signals(signal_ref,edge_node_id,series_key,created_at) \
         VALUES(?,?,?,?)",
    )
    .bind(&signal_ref)
    .bind(edge_node_id)
    .bind(series_key)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO semantic_calibration_revisions(signal_ref,revision,scale,calibration_offset,created_at) \
         VALUES(?,1,1,0,?)",
    )
    .bind(&signal_ref)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO semantic_calibration_starts(\
         signal_ref,revision,ledger_epoch,start_after_pub_seq) \
         SELECT ?,1,ledger_epoch,0 FROM accepted_cursors WHERE edge_node_id=?",
    )
    .bind(&signal_ref)
    .bind(edge_node_id)
    .execute(&mut **tx)
    .await?;
    Ok(signal_ref)
}

async fn revise_rule_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    rule_id: &str,
    display_name: &str,
    spec: RuleSpec,
    now: i64,
) -> Result<SemanticRule, StorageError> {
    let row = sqlx::query(
        "SELECT rule.signal_ref,signal.edge_node_id,signal.series_key,rule.kind,\
         rule.series_id,rule.revision,rule.spec_json FROM semantic_rules AS rule \
         JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
         WHERE rule.rule_id=? AND rule.active=1",
    )
    .bind(rule_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StorageError::SemanticNotFound)?;
    let existing_kind: String = row.try_get("kind")?;
    if parse_semantic_kind(&existing_kind)? != spec.kind {
        return Err(StorageError::InvalidSemantic(
            "semantic rule kind cannot change".into(),
        ));
    }
    let old_spec: Vec<u8> = row.try_get("spec_json")?;
    let spec_json = serde_json::to_vec(&spec)
        .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
    let revision: i64 = row.try_get::<i64, _>("revision")? + 1;
    let series_id = if old_spec == spec_json {
        row.try_get("series_id")?
    } else {
        Uuid::new_v4().to_string()
    };
    sqlx::query(
        "UPDATE semantic_rules SET display_name=?,series_id=?,revision=?,spec_json=? \
         WHERE rule_id=?",
    )
    .bind(display_name)
    .bind(&series_id)
    .bind(revision)
    .bind(&spec_json)
    .bind(rule_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO semantic_rule_revisions(rule_id,revision,series_id,spec_json,created_at) \
         VALUES(?,?,?,?,?)",
    )
    .bind(rule_id)
    .bind(revision)
    .bind(&series_id)
    .bind(&spec_json)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    let edge_node_id: String = row.try_get("edge_node_id")?;
    sqlx::query(
        "INSERT INTO semantic_rule_starts(rule_id,revision,ledger_epoch,start_after_pub_seq) \
         SELECT ?,?,ledger_epoch,accepted_through FROM accepted_cursors WHERE edge_node_id=?",
    )
    .bind(rule_id)
    .bind(revision)
    .bind(&edge_node_id)
    .execute(&mut **tx)
    .await?;
    Ok(SemanticRule {
        rule_id: rule_id.into(),
        signal_ref: row.try_get("signal_ref")?,
        edge_node_id,
        series_key: row.try_get("series_key")?,
        display_name: display_name.into(),
        kind: spec.kind,
        series_id,
        revision,
        active: true,
    })
}

async fn apply_reset_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    rule_id: &str,
    reset_id: &str,
    _observation_id: &str,
    now: i64,
) -> Result<(), StorageError> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM semantic_rules WHERE rule_id=? AND active=1")
            .bind(rule_id)
            .fetch_optional(&mut **tx)
            .await?;
    if kind.as_deref() != Some("cumulative_counter") {
        return Err(StorageError::InvalidSemantic(
            "only an active cumulative counter can be reset".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO semantic_counter_resets(reset_id,rule_id,requested_at) VALUES(?,?,?)",
    )
    .bind(reset_id)
    .bind(rule_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO semantic_counter_reset_boundaries(\
         reset_id,ledger_epoch,apply_after_pub_seq) \
         SELECT ?,cursor.ledger_epoch,cursor.accepted_through \
         FROM semantic_rules AS rule JOIN semantic_signals AS signal \
         ON signal.signal_ref=rule.signal_ref JOIN accepted_cursors AS cursor \
         ON cursor.edge_node_id=signal.edge_node_id WHERE rule.rule_id=?",
    )
    .bind(reset_id)
    .bind(rule_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn activate_profile_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    display_name: &str,
    registration: &'static OutputAdapterRegistration,
    values: Map<String, Value>,
    now: i64,
) -> Result<ExportProfile, StorageError> {
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| StorageError::InvalidOutput("Edge identity is not initialized".into()))?;
    let profile_id = prefixed_uuid("exp_");
    let descriptor = registration.adapter.descriptor();
    let setup = serde_json::to_vec(&values)
        .map_err(|error| StorageError::InvalidOutput(error.to_string()))?;
    let initial = if registration
        .profile_policy
        .setup()
        .requires_external_confirmation
    {
        "preparing"
    } else {
        "active"
    };
    sqlx::query(
        "INSERT INTO export_profiles(profile_id,display_name,adapter_id,\
         adapter_schema_version,setup_json,state,revision,created_at) \
         VALUES(?,?,?,?,?,?,1,?)",
    )
    .bind(&profile_id)
    .bind(display_name)
    .bind(descriptor.id)
    .bind(i64::from(descriptor.config_schema_version))
    .bind(setup)
    .bind(initial)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    let rows = sqlx::query(
        "SELECT rule.rule_id,rule.signal_ref,signal.edge_node_id,rule.kind \
         FROM semantic_rules AS rule JOIN semantic_signals AS signal \
         ON signal.signal_ref=rule.signal_ref WHERE rule.active=1 \
         ORDER BY rule.created_at,rule.rule_id",
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut bindings = Vec::new();
    for row in rows {
        let inventory = RuleInventory {
            rule_id: row.try_get("rule_id")?,
            signal_ref: row.try_get("signal_ref")?,
            edge_node_id: row.try_get("edge_node_id")?,
            kind: parse_semantic_kind(&row.try_get::<String, _>("kind")?)?,
        };
        bindings.push(
            create_binding_sqlite(
                tx,
                &profile_id,
                registration,
                &edge_id,
                &inventory,
                &values,
                now,
            )
            .await?,
        );
    }
    Ok(ExportProfile {
        profile_id,
        display_name: display_name.into(),
        adapter_id: descriptor.id.into(),
        state: ProfileState::parse(initial)?,
        revision: 1,
        bindings,
    })
}

async fn create_binding_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    registration: &'static OutputAdapterRegistration,
    edge_id: &str,
    rule: &RuleInventory,
    values: &Map<String, Value>,
    now: i64,
) -> Result<OutputBinding, StorageError> {
    let modes = compatible_modes(registration, rule.kind);
    let binding_id = prefixed_uuid("bind_");
    if modes.is_empty() {
        sqlx::query(
            "INSERT INTO output_bindings(binding_id,profile_id,rule_id,state,\
             ineligible_reason,revision,created_at) VALUES(?,?,?,'ineligible',?,1,?)",
        )
        .bind(&binding_id)
        .bind(profile_id)
        .bind(&rule.rule_id)
        .bind("adapter has no compatible mode")
        .bind(now)
        .execute(&mut **tx)
        .await?;
        return Ok(OutputBinding {
            binding_id,
            rule_id: rule.rule_id.clone(),
            external_id: String::new(),
            mode: None,
            active: false,
            needs_configuration: false,
            ineligible_reason: "adapter has no compatible mode".into(),
        });
    }
    if modes.len() > 1 {
        sqlx::query(
            "INSERT INTO output_bindings(binding_id,profile_id,rule_id,state,revision,created_at) \
             VALUES(?,?,?,'needs_configuration',1,?)",
        )
        .bind(&binding_id)
        .bind(profile_id)
        .bind(&rule.rule_id)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        return Ok(OutputBinding {
            binding_id,
            rule_id: rule.rule_id.clone(),
            external_id: String::new(),
            mode: None,
            active: false,
            needs_configuration: true,
            ineligible_reason: String::new(),
        });
    }
    create_configured_binding_sqlite(
        tx,
        profile_id,
        &binding_id,
        registration,
        edge_id,
        rule,
        modes[0],
        values,
        now,
    )
    .await
}

async fn identity_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    adapter_id: &str,
    scope_key: &str,
    prefix: &str,
    now: i64,
) -> Result<(String, String, bool), StorageError> {
    if let Some(row) = sqlx::query(
        "SELECT output_identity_id,external_id,confirmed_at IS NOT NULL AS confirmed \
         FROM output_identities WHERE adapter_id=? AND scope_key=?",
    )
    .bind(adapter_id)
    .bind(scope_key)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok((
            row.try_get("output_identity_id")?,
            row.try_get("external_id")?,
            row.try_get("confirmed")?,
        ));
    }
    let identity_id = prefixed_uuid("oid_");
    let external = external_id(prefix);
    sqlx::query(
        "INSERT INTO output_identities(output_identity_id,adapter_id,scope_key,\
         external_id,created_at) VALUES(?,?,?,?,?)",
    )
    .bind(&identity_id)
    .bind(adapter_id)
    .bind(scope_key)
    .bind(&external)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok((identity_id, external, false))
}

#[allow(clippy::too_many_arguments)]
async fn create_configured_binding_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    binding_id: &str,
    registration: &'static OutputAdapterRegistration,
    edge_id: &str,
    rule: &RuleInventory,
    mode: &str,
    values: &Map<String, Value>,
    now: i64,
) -> Result<OutputBinding, StorageError> {
    let descriptor = registration.adapter.descriptor();
    let policy = registration.profile_policy.identity_policy();
    let scope_key = identity_scope_key(registration, rule, mode);
    let (identity_id, external, confirmed) =
        identity_sqlite(tx, descriptor.id, &scope_key, policy.prefix, now).await?;
    let proposals = registration
        .profile_policy
        .propose(&ProfileRequest {
            edge_id,
            rule_id: &rule.rule_id,
            signal_ref: &rule.signal_ref,
            external_id: &external,
            observation_kind: adapter_kind(rule.kind),
            mode,
            values,
        })
        .map_err(|error| StorageError::InvalidOutput(error.to_string()))?;
    if proposals.len() != 1 {
        return Err(StorageError::InvalidOutput(
            "v1 profile policy must return exactly one route".into(),
        ));
    }
    let proposal = &proposals[0];
    registration
        .adapter
        .validate_config(&proposal.config, adapter_kind(rule.kind))
        .map_err(|error| StorageError::InvalidOutput(error.to_string()))?;
    let active = !proposal.requires_external_confirmation || confirmed;
    let state = if active { "active" } else { "prepared" };
    sqlx::query(
        "INSERT INTO output_bindings(binding_id,profile_id,rule_id,output_identity_id,\
         mode,state,revision,created_at,activated_at) VALUES(?,?,?,?,?,?,1,?,?)",
    )
    .bind(binding_id)
    .bind(profile_id)
    .bind(&rule.rule_id)
    .bind(&identity_id)
    .bind(mode)
    .bind(state)
    .bind(now)
    .bind(active.then_some(now))
    .execute(&mut **tx)
    .await?;
    let route_id = prefixed_uuid("out_");
    sqlx::query(
        "INSERT INTO output_routes(route_id,binding_id,rule_id,adapter_id,\
         config_schema_version,config_json,active,lifecycle_state,created_at) \
         VALUES(?,?,?,?,?,?,?,?,?)",
    )
    .bind(&route_id)
    .bind(binding_id)
    .bind(&rule.rule_id)
    .bind(descriptor.id)
    .bind(i64::from(descriptor.config_schema_version))
    .bind(proposal.config.get().as_bytes())
    .bind(active)
    .bind("active")
    .bind(now)
    .execute(&mut **tx)
    .await?;
    if active {
        insert_binding_starts_sqlite(tx, binding_id, &rule.edge_node_id).await?;
    }
    Ok(OutputBinding {
        binding_id: binding_id.into(),
        rule_id: rule.rule_id.clone(),
        external_id: external,
        mode: Some(mode.into()),
        active,
        needs_configuration: false,
        ineligible_reason: String::new(),
    })
}

async fn insert_binding_starts_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    binding_id: &str,
    edge_node_id: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT OR IGNORE INTO output_binding_starts(\
         binding_id,ledger_epoch,start_after_pub_seq) \
         SELECT ?,ledger_epoch,accepted_through FROM accepted_cursors WHERE edge_node_id=?",
    )
    .bind(binding_id)
    .bind(edge_node_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn auto_bind_rule_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    rule: &SemanticRule,
    now: i64,
) -> Result<(), StorageError> {
    let profiles = sqlx::query(
        "SELECT profile_id,adapter_id,setup_json FROM export_profiles \
         WHERE state IN ('preparing','active') ORDER BY created_at,profile_id",
    )
    .fetch_all(&mut **tx)
    .await?;
    let edge_id: Option<String> =
        sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
            .fetch_optional(&mut **tx)
            .await?;
    let Some(edge_id) = edge_id else {
        return Ok(());
    };
    for profile in profiles {
        let adapter_id: String = profile.try_get("adapter_id")?;
        let registration = registered_output_adapters()
            .iter()
            .find(|item| item.adapter.descriptor().id == adapter_id)
            .ok_or_else(|| StorageError::InvalidOutput("registered adapter disappeared".into()))?;
        let setup: Vec<u8> = profile.try_get("setup_json")?;
        let values: Map<String, Value> = serde_json::from_slice::<Value>(&setup)
            .map_err(|error| StorageError::InvalidOutput(error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| StorageError::InvalidOutput("profile setup is not an object".into()))?;
        create_binding_sqlite(
            tx,
            &profile.try_get::<String, _>("profile_id")?,
            registration,
            &edge_id,
            &RuleInventory {
                rule_id: rule.rule_id.clone(),
                signal_ref: rule.signal_ref.clone(),
                edge_node_id: rule.edge_node_id.clone(),
                kind: rule.kind,
            },
            &values,
            now,
        )
        .await?;
    }
    Ok(())
}

async fn configure_binding_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    binding_id: &str,
    mode: &str,
    values: Map<String, Value>,
    adapters: &'static [OutputAdapterRegistration],
    now: i64,
) -> Result<OutputBinding, StorageError> {
    let row = sqlx::query(
        "SELECT binding.profile_id,binding.rule_id,profile.adapter_id,profile.setup_json,\
         rule.signal_ref,signal.edge_node_id,rule.kind FROM output_bindings AS binding \
         JOIN export_profiles AS profile ON profile.profile_id=binding.profile_id \
         JOIN semantic_rules AS rule ON rule.rule_id=binding.rule_id \
         JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
         WHERE binding.binding_id=? AND binding.state='needs_configuration'",
    )
    .bind(binding_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StorageError::SemanticNotFound)?;
    let adapter_id: String = row.try_get("adapter_id")?;
    let registration = adapters
        .iter()
        .find(|item| item.adapter.descriptor().id == adapter_id)
        .ok_or_else(|| StorageError::InvalidOutput("registered adapter disappeared".into()))?;
    let mut merged: Map<String, Value> =
        serde_json::from_slice::<Value>(&row.try_get::<Vec<u8>, _>("setup_json")?)
            .map_err(|error| StorageError::InvalidOutput(error.to_string()))?
            .as_object()
            .cloned()
            .unwrap_or_default();
    merged.extend(values);
    let edge_id: String = sqlx::query_scalar("SELECT edge_id FROM edge_meta WHERE singleton=1")
        .fetch_one(&mut **tx)
        .await?;
    let inventory = RuleInventory {
        rule_id: row.try_get("rule_id")?,
        signal_ref: row.try_get("signal_ref")?,
        edge_node_id: row.try_get("edge_node_id")?,
        kind: parse_semantic_kind(&row.try_get::<String, _>("kind")?)?,
    };
    if !compatible_modes(registration, inventory.kind).contains(&mode) {
        return Err(StorageError::InvalidOutput(
            "mode is not compatible with the semantic rule".into(),
        ));
    }
    sqlx::query("DELETE FROM output_bindings WHERE binding_id=?")
        .bind(binding_id)
        .execute(&mut **tx)
        .await?;
    create_configured_binding_sqlite(
        tx,
        &row.try_get::<String, _>("profile_id")?,
        binding_id,
        registration,
        &edge_id,
        &inventory,
        mode,
        &merged,
        now,
    )
    .await
}

async fn confirm_binding_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    binding_id: &str,
    now: i64,
) -> Result<(), StorageError> {
    let row = sqlx::query(
        "SELECT profile_id,output_identity_id FROM output_bindings \
         WHERE binding_id=? AND state='prepared'",
    )
    .bind(binding_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StorageError::SemanticNotFound)?;
    let profile_id: String = row.try_get("profile_id")?;
    let identity_id: String = row.try_get("output_identity_id")?;
    sqlx::query(
        "UPDATE output_identities SET confirmed_at=COALESCE(confirmed_at,?) \
         WHERE output_identity_id=?",
    )
    .bind(now)
    .bind(&identity_id)
    .execute(&mut **tx)
    .await?;
    let rows = sqlx::query(
        "SELECT binding.binding_id,signal.edge_node_id FROM output_bindings AS binding \
         JOIN semantic_rules AS rule ON rule.rule_id=binding.rule_id \
         JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref \
         WHERE binding.profile_id=? AND binding.output_identity_id=? AND binding.state='prepared'",
    )
    .bind(&profile_id)
    .bind(&identity_id)
    .fetch_all(&mut **tx)
    .await?;
    for item in rows {
        let candidate: String = item.try_get("binding_id")?;
        insert_binding_starts_sqlite(
            tx,
            &candidate,
            &item.try_get::<String, _>("edge_node_id")?,
        )
        .await?;
    }
    sqlx::query(
        "UPDATE output_bindings SET state='active',revision=revision+1,activated_at=? \
         WHERE profile_id=? AND output_identity_id=? AND state='prepared'",
    )
    .bind(now)
    .bind(&profile_id)
    .bind(&identity_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE output_routes SET active=1,lifecycle_state='active' WHERE binding_id IN \
         (SELECT binding_id FROM output_bindings WHERE profile_id=? \
          AND output_identity_id=? AND state='active')",
    )
    .bind(&profile_id)
    .bind(&identity_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE export_profiles SET state='active',revision=revision+1 \
         WHERE profile_id=? AND state='preparing'",
    )
    .bind(&profile_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn stop_profile_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    profile_id: &str,
    now: i64,
) -> Result<(), StorageError> {
    let result = sqlx::query(
        "UPDATE export_profiles SET state='draining',revision=revision+1 \
         WHERE profile_id=? AND state IN ('preparing','active')",
    )
    .bind(profile_id)
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() != 1 {
        return Err(StorageError::SemanticNotFound);
    }
    sqlx::query(
        "INSERT OR IGNORE INTO output_binding_ends(binding_id,ledger_epoch,end_at_pub_seq) \
         SELECT binding.binding_id,cursor.ledger_epoch,cursor.accepted_through \
         FROM output_bindings AS binding JOIN semantic_rules AS rule \
         ON rule.rule_id=binding.rule_id JOIN semantic_signals AS signal \
         ON signal.signal_ref=rule.signal_ref JOIN accepted_cursors AS cursor \
         ON cursor.edge_node_id=signal.edge_node_id \
         WHERE binding.profile_id=? AND binding.state='active'",
    )
    .bind(profile_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE output_bindings SET state=CASE WHEN state='active' THEN 'draining' ELSE 'stopped' END,\
         revision=revision+1,stopped_at=CASE WHEN state='active' THEN NULL ELSE ? END \
         WHERE profile_id=? AND state IN ('active','prepared','needs_configuration','ineligible')",
    )
    .bind(now)
    .bind(profile_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE output_routes SET lifecycle_state='draining' WHERE binding_id IN \
         (SELECT binding_id FROM output_bindings WHERE profile_id=? AND state='draining')",
    )
    .bind(profile_id)
    .execute(&mut **tx)
    .await?;
    reconcile_profiles_sqlite(tx, now).await
}

async fn list_profiles_sqlite(
    pool: &sqlx::SqlitePool,
) -> Result<Vec<ExportProfile>, StorageError> {
    let profiles = sqlx::query(
        "SELECT profile_id,display_name,adapter_id,state,revision \
         FROM export_profiles ORDER BY created_at,profile_id",
    )
    .fetch_all(pool)
    .await?;
    let mut result = Vec::new();
    for profile in profiles {
        let profile_id: String = profile.try_get("profile_id")?;
        let rows = sqlx::query(
            "SELECT binding.binding_id,binding.rule_id,COALESCE(identity.external_id,'') external_id,\
             binding.mode,binding.state,binding.ineligible_reason \
             FROM output_bindings AS binding LEFT JOIN output_identities AS identity \
             ON identity.output_identity_id=binding.output_identity_id \
             WHERE binding.profile_id=? ORDER BY binding.created_at,binding.binding_id",
        )
        .bind(&profile_id)
        .fetch_all(pool)
        .await?;
        let bindings = rows
            .into_iter()
            .map(row_to_binding)
            .collect::<Result<Vec<_>, _>>()?;
        result.push(ExportProfile {
            profile_id,
            display_name: profile.try_get("display_name")?,
            adapter_id: profile.try_get("adapter_id")?,
            state: ProfileState::parse(&profile.try_get::<String, _>("state")?)?,
            revision: profile.try_get("revision")?,
            bindings,
        });
    }
    Ok(result)
}

async fn claim_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    token: &str,
    now: i64,
    lease_ms: i64,
) -> Result<Option<ClaimedOutput>, StorageError> {
    let row = sqlx::query(
        "WITH ranked AS (
            SELECT outbox.export_id,
              ROW_NUMBER() OVER (PARTITION BY outbox.route_id
                ORDER BY observation.observation_row_id,outbox.export_id) route_rank
            FROM output_outbox AS outbox
            JOIN semantic_observations AS observation
              ON observation.observation_id=outbox.observation_id
            WHERE outbox.published_at IS NULL
        )
        SELECT outbox.export_id,outbox.route_id,outbox.topic,outbox.qos,outbox.retain,
          outbox.payload_json,outbox.attempts
        FROM output_outbox AS outbox JOIN ranked ON ranked.export_id=outbox.export_id
        WHERE ranked.route_rank=1
          AND (outbox.claim_token IS NULL OR outbox.claim_until<=?)
        ORDER BY outbox.attempts,outbox.created_at,outbox.export_id
        LIMIT 1",
    )
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let export_id: String = row.try_get("export_id")?;
    let result = sqlx::query(
        "UPDATE output_outbox SET claim_token=?,claimed_at=?,claim_until=?,attempts=attempts+1 \
         WHERE export_id=? AND published_at IS NULL \
         AND (claim_token IS NULL OR claim_until<=?)",
    )
    .bind(token)
    .bind(now)
    .bind(now.saturating_add(lease_ms))
    .bind(&export_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() != 1 {
        return Ok(None);
    }
    Ok(Some(ClaimedOutput {
        export_id,
        route_id: row.try_get("route_id")?,
        topic: row.try_get("topic")?,
        qos: u8::try_from(row.try_get::<i64, _>("qos")?)
            .map_err(|_| StorageError::InvalidOutput("invalid stored QoS".into()))?,
        retain: row.try_get("retain")?,
        payload: row.try_get("payload_json")?,
        attempts: row.try_get::<i64, _>("attempts")? + 1,
    }))
}

async fn project_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    adapters: &'static [OutputAdapterRegistration],
) -> Result<Option<ProjectedOne>, StorageError> {
    let retry = retry_route_sqlite(tx, adapters).await?;
    if let Some(reset) = ready_reset_sqlite(tx).await? {
        let publications =
            retry.publications + enqueue_routes_sqlite(tx, &reset, adapters).await?;
        return Ok(Some(ProjectedOne {
            receipt: true,
            observation: true,
            publications,
        }));
    }
    let Some(candidate) = candidate_sqlite(tx).await? else {
        return Ok(retry.attempted.then_some(ProjectedOne {
            receipt: false,
            observation: false,
            publications: retry.publications,
        }));
    };
    let measurement = match decode_measurement(&candidate) {
        Ok(measurement) => measurement,
        Err(StorageError::InvalidSemantic(_)) => {
            return record_projection_failure_sqlite(tx, &candidate, retry.publications).await;
        }
        Err(error) => return Err(error),
    };
    let calibrated = crate::semantics::Calibration {
        scale: candidate.current_scale,
        offset: candidate.current_offset,
    }
    .apply(measurement.values[0]);
    let calibrated = match calibrated {
        Ok(calibrated) => calibrated,
        Err(_) => {
            return record_projection_failure_sqlite(tx, &candidate, retry.publications).await;
        }
    };
    let runtime = sqlx::query(
        "SELECT initialized,detector_active,counter,pending,pending_active,pending_since,\
         applied_revision,applied_calibration_revision,applied_ledger_epoch,\
         applied_series_id,next_sequence FROM semantic_rule_runtime WHERE rule_id=?",
    )
    .bind(&candidate.rule_id)
    .fetch_one(&mut **tx)
    .await?;
    let (mut state, mut next_sequence) = runtime_state(&runtime)?;
    let applied_revision: i64 = runtime.try_get("applied_revision")?;
    let applied_calibration: i64 = runtime.try_get("applied_calibration_revision")?;
    let applied_epoch: String = runtime.try_get("applied_ledger_epoch")?;
    let applied_series: String = runtime.try_get("applied_series_id")?;
    if applied_revision != candidate.current_revision
        || applied_calibration != candidate.current_calibration_revision
        || applied_epoch != candidate.ledger_epoch
        || applied_series != candidate.current_series_id
    {
        state.initialized = false;
        state.active = false;
        state.pending = false;
        state.pending_active = false;
        state.pending_since = 0;
        if applied_series != candidate.current_series_id {
            state.counter = 0;
            next_sequence = 1;
        }
    }
    let evaluation = evaluate_rule(
        candidate.current_spec,
        state,
        calibrated,
        measurement.received_at,
    );
    let (evaluation, next_state) = match evaluation {
        Ok(evaluation) => evaluation,
        Err(_) => {
            return record_projection_failure_sqlite(tx, &candidate, retry.publications).await;
        }
    };
    let produced = if evaluation.emitted {
        Some(produced_observation(
            &candidate,
            &measurement,
            &evaluation,
            next_sequence,
        )?)
    } else {
        None
    };
    let mut publications = retry.publications;
    if let Some(observation) = &produced {
        insert_observation_sqlite(tx, observation).await?;
        publications += enqueue_routes_sqlite(tx, observation, adapters).await?;
        next_sequence += 1;
    }
    sqlx::query(
        "INSERT INTO semantic_projection_receipts(rule_id,ledger_epoch,pub_seq,revision,\
         calibration_revision,observation_id) VALUES(?,?,?,?,?,?)",
    )
    .bind(&candidate.rule_id)
    .bind(&candidate.ledger_epoch)
    .bind(candidate.pub_seq)
    .bind(candidate.current_revision)
    .bind(candidate.current_calibration_revision)
    .bind(produced.as_ref().map(|value| value.observation_id.as_str()))
    .execute(&mut **tx)
    .await?;
    update_runtime_sqlite(
        tx,
        &candidate,
        next_state,
        next_sequence,
    )
    .await?;
    Ok(Some(ProjectedOne {
        receipt: true,
        observation: produced.is_some(),
        publications,
    }))
}

async fn record_projection_failure_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    candidate: &ProjectionCandidate,
    publications: usize,
) -> Result<Option<ProjectedOne>, StorageError> {
    sqlx::query(
        "INSERT INTO semantic_projection_failures(rule_id,ledger_epoch,pub_seq,error_code,\
         attempts,last_failed_at) VALUES(?,?,?,'invalid_observation',1,?) \
         ON CONFLICT(rule_id,ledger_epoch,pub_seq) DO UPDATE SET \
         attempts=attempts+1,last_failed_at=excluded.last_failed_at",
    )
    .bind(&candidate.rule_id)
    .bind(&candidate.ledger_epoch)
    .bind(candidate.pub_seq)
    .bind(candidate.received_at)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO semantic_projection_receipts(rule_id,ledger_epoch,pub_seq,revision,\
         calibration_revision,observation_id) VALUES(?,?,?,?,?,NULL)",
    )
    .bind(&candidate.rule_id)
    .bind(&candidate.ledger_epoch)
    .bind(candidate.pub_seq)
    .bind(candidate.current_revision)
    .bind(candidate.current_calibration_revision)
    .execute(&mut **tx)
    .await?;
    Ok(Some(ProjectedOne {
        receipt: true,
        observation: false,
        publications,
    }))
}

async fn candidate_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Option<ProjectionCandidate>, StorageError> {
    let row = sqlx::query(
        "SELECT rule.rule_id,rule.signal_ref,signal.edge_node_id,raw.ledger_epoch,raw.pub_seq,\
         raw.received_at,\
         raw.record_json,revision.revision,revision.series_id,revision.spec_json,\
         calibration.revision AS calibration_revision,calibration.scale,\
         calibration.calibration_offset AS calibration_offset \
         FROM semantic_rules AS rule JOIN semantic_signals AS signal \
         ON signal.signal_ref=rule.signal_ref JOIN raw_records AS raw \
         ON raw.edge_node_id=signal.edge_node_id \
         AND json_extract(raw.record_json,'$.series_key')=signal.series_key \
         JOIN semantic_rule_revisions AS revision ON revision.rule_id=rule.rule_id \
         AND revision.revision=CASE \
           WHEN EXISTS(SELECT 1 FROM semantic_rule_starts all_start \
             WHERE all_start.rule_id=rule.rule_id AND all_start.ledger_epoch=raw.ledger_epoch) \
           THEN COALESCE((SELECT MAX(start.revision) FROM semantic_rule_starts start \
             WHERE start.rule_id=rule.rule_id AND start.ledger_epoch=raw.ledger_epoch \
             AND raw.pub_seq>start.start_after_pub_seq),\
             (SELECT MIN(start.revision)-1 FROM semantic_rule_starts start \
               WHERE start.rule_id=rule.rule_id AND start.ledger_epoch=raw.ledger_epoch)) \
           ELSE rule.revision END \
         JOIN semantic_calibration_revisions AS calibration \
         ON calibration.signal_ref=signal.signal_ref AND calibration.revision=CASE \
           WHEN EXISTS(SELECT 1 FROM semantic_calibration_starts all_cal \
             WHERE all_cal.signal_ref=signal.signal_ref AND all_cal.ledger_epoch=raw.ledger_epoch) \
           THEN COALESCE((SELECT MAX(start.revision) FROM semantic_calibration_starts start \
             WHERE start.signal_ref=signal.signal_ref AND start.ledger_epoch=raw.ledger_epoch \
             AND raw.pub_seq>start.start_after_pub_seq),\
             (SELECT MIN(start.revision)-1 FROM semantic_calibration_starts start \
               WHERE start.signal_ref=signal.signal_ref \
                 AND start.ledger_epoch=raw.ledger_epoch)) \
           ELSE signal.calibration_revision END \
         WHERE json_extract(raw.record_json,'$.family')='measurement' \
         AND NOT EXISTS(SELECT 1 FROM semantic_projection_receipts receipt \
           WHERE receipt.rule_id=rule.rule_id AND receipt.ledger_epoch=raw.ledger_epoch \
           AND receipt.pub_seq=raw.pub_seq) \
         AND (NOT EXISTS(SELECT 1 FROM semantic_rule_ends finish \
           WHERE finish.rule_id=rule.rule_id AND finish.ledger_epoch=raw.ledger_epoch) \
           OR raw.pub_seq<=(SELECT finish.end_at_pub_seq FROM semantic_rule_ends finish \
             WHERE finish.rule_id=rule.rule_id AND finish.ledger_epoch=raw.ledger_epoch)) \
         ORDER BY raw.received_at,raw.edge_node_id,raw.ledger_epoch,raw.pub_seq,rule.created_at \
         LIMIT 1",
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        let spec: Vec<u8> = row.try_get("spec_json")?;
        Ok(ProjectionCandidate {
            rule_id: row.try_get("rule_id")?,
            signal_ref: row.try_get("signal_ref")?,
            edge_node_id: row.try_get("edge_node_id")?,
            ledger_epoch: row.try_get("ledger_epoch")?,
            pub_seq: row.try_get("pub_seq")?,
            received_at: row.try_get("received_at")?,
            record_json: row.try_get("record_json")?,
            current_revision: row.try_get("revision")?,
            current_series_id: row.try_get("series_id")?,
            current_spec: serde_json::from_slice(&spec)
                .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?,
            current_calibration_revision: row.try_get("calibration_revision")?,
            current_scale: row.try_get("scale")?,
            current_offset: row.try_get("calibration_offset")?,
        })
    })
    .transpose()
}

async fn insert_observation_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    value: &ProducedObservation,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO semantic_observations(observation_id,rule_id,revision,\
         calibration_revision,series_id,sequence,kind,value_json,reading,signal_ref,\
         edge_node_id,ledger_epoch,source_pub_seq,observed_at,created_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&value.observation_id)
    .bind(&value.rule_id)
    .bind(value.revision)
    .bind(value.calibration_revision)
    .bind(&value.series_id)
    .bind(value.sequence as i64)
    .bind(semantic_kind(value.kind))
    .bind(&value.value_json)
    .bind(value.reading)
    .bind(&value.signal_ref)
    .bind(&value.edge_node_id)
    .bind(&value.ledger_epoch)
    .bind(value.source_pub_seq)
    .bind(value.observed_at)
    .bind(value.created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn retry_route_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    adapters: &'static [OutputAdapterRegistration],
) -> Result<RouteRetry, StorageError> {
    let row = sqlx::query(
        "SELECT observation.observation_id,observation.rule_id,observation.revision,\
         observation.calibration_revision,observation.series_id,observation.sequence,\
         observation.kind,observation.value_json,observation.reading,observation.signal_ref,\
         observation.edge_node_id,observation.ledger_epoch,observation.source_pub_seq,\
         observation.observed_at,observation.created_at \
         FROM output_routes AS route JOIN output_bindings AS binding \
         ON binding.binding_id=route.binding_id JOIN semantic_observations AS observation \
         ON observation.rule_id=route.rule_id LEFT JOIN output_outbox AS outbox \
         ON outbox.route_id=route.route_id AND outbox.observation_id=observation.observation_id \
         WHERE outbox.export_id IS NULL AND route.active=1 \
         AND route.lifecycle_state IN ('active','draining') \
         AND binding.state IN ('active','draining') \
         AND (NOT EXISTS(SELECT 1 FROM output_binding_starts start \
           WHERE start.binding_id=binding.binding_id \
             AND start.ledger_epoch=observation.ledger_epoch) \
           OR EXISTS(SELECT 1 FROM output_binding_starts start \
             WHERE start.binding_id=binding.binding_id \
             AND start.ledger_epoch=observation.ledger_epoch \
             AND observation.source_pub_seq>start.start_after_pub_seq)) \
         AND (NOT EXISTS(SELECT 1 FROM output_binding_ends finish \
           WHERE finish.binding_id=binding.binding_id \
             AND finish.ledger_epoch=observation.ledger_epoch) \
           OR EXISTS(SELECT 1 FROM output_binding_ends finish \
             WHERE finish.binding_id=binding.binding_id \
             AND finish.ledger_epoch=observation.ledger_epoch \
             AND observation.source_pub_seq<=finish.end_at_pub_seq)) \
         ORDER BY COALESCE((SELECT MAX(attempts) FROM output_route_attempts attempt \
           WHERE attempt.route_id=route.route_id),0),\
         COALESCE((SELECT MAX(last_attempt_at) FROM output_route_attempts attempt \
           WHERE attempt.route_id=route.route_id),0),\
         route.created_at,observation.observation_row_id LIMIT 1",
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(RouteRetry::default());
    };
    let kind = parse_semantic_kind(&row.try_get::<String, _>("kind")?)?;
    let value_json: Vec<u8> = row.try_get("value_json")?;
    let decoded = decode_stored_observation(
        row.try_get("observation_id")?,
        row.try_get("rule_id")?,
        row.try_get("series_id")?,
        row.try_get("sequence")?,
        kind,
        &value_json,
        row.try_get("reading")?,
        row.try_get("observed_at")?,
    )?;
    let observation = ProducedObservation {
        observation_id: decoded.observation_id,
        rule_id: decoded.rule_id,
        revision: row.try_get("revision")?,
        calibration_revision: row.try_get("calibration_revision")?,
        series_id: decoded.series_id,
        sequence: decoded.sequence,
        kind,
        value_json,
        value: decoded.value,
        reading: row.try_get("reading")?,
        signal_ref: row.try_get("signal_ref")?,
        edge_node_id: row.try_get("edge_node_id")?,
        ledger_epoch: row.try_get("ledger_epoch")?,
        source_pub_seq: row.try_get("source_pub_seq")?,
        observed_at: decoded.observed_at,
        created_at: row.try_get("created_at")?,
    };
    Ok(RouteRetry {
        attempted: true,
        publications: enqueue_routes_sqlite(tx, &observation, adapters).await?,
    })
}

async fn enqueue_routes_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    observation: &ProducedObservation,
    adapters: &'static [OutputAdapterRegistration],
) -> Result<usize, StorageError> {
    let routes = sqlx::query(
        "SELECT route.route_id,route.adapter_id,route.config_schema_version,route.config_json \
         FROM output_routes AS route JOIN output_bindings AS binding \
         ON binding.binding_id=route.binding_id \
         WHERE route.rule_id=? AND route.active=1 \
         AND route.lifecycle_state IN ('active','draining') \
         AND binding.state IN ('active','draining') \
         AND NOT EXISTS(SELECT 1 FROM output_route_attempts attempt \
           WHERE attempt.route_id=route.route_id AND attempt.observation_id<>?) \
         AND (NOT EXISTS(SELECT 1 FROM output_binding_starts start \
           WHERE start.binding_id=binding.binding_id AND start.ledger_epoch=?) \
           OR EXISTS(SELECT 1 FROM output_binding_starts start \
             WHERE start.binding_id=binding.binding_id AND start.ledger_epoch=? \
             AND ?>start.start_after_pub_seq)) \
         AND (NOT EXISTS(SELECT 1 FROM output_binding_ends finish \
           WHERE finish.binding_id=binding.binding_id AND finish.ledger_epoch=?) \
           OR EXISTS(SELECT 1 FROM output_binding_ends finish \
             WHERE finish.binding_id=binding.binding_id AND finish.ledger_epoch=? \
             AND ?<=finish.end_at_pub_seq)) ORDER BY route.created_at,route.route_id",
    )
    .bind(&observation.rule_id)
    .bind(&observation.observation_id)
    .bind(&observation.ledger_epoch)
    .bind(&observation.ledger_epoch)
    .bind(observation.source_pub_seq)
    .bind(&observation.ledger_epoch)
    .bind(&observation.ledger_epoch)
    .bind(observation.source_pub_seq)
    .fetch_all(&mut **tx)
    .await?;
    let adapter_observation = adapter_observation(observation)?;
    let mut inserted = 0;
    for route in routes {
        let route_id: String = route.try_get("route_id")?;
        let adapter_id: String = route.try_get("adapter_id")?;
        let Some(registration) = adapters
            .iter()
            .find(|item| item.adapter.descriptor().id == adapter_id)
        else {
            record_route_error_sqlite(
                tx,
                &route_id,
                &observation.observation_id,
                "adapter_unavailable",
                observation.created_at,
            )
            .await?;
            continue;
        };
        let version: i64 = route.try_get("config_schema_version")?;
        if version != i64::from(registration.adapter.descriptor().config_schema_version) {
            record_route_error_sqlite(
                tx,
                &route_id,
                &observation.observation_id,
                "config_version_mismatch",
                observation.created_at,
            )
            .await?;
            continue;
        }
        let config: Vec<u8> = route.try_get("config_json")?;
        let config: Box<RawValue> = match serde_json::from_slice(&config) {
            Ok(value) => value,
            Err(_) => {
                record_route_error_sqlite(
                    tx,
                    &route_id,
                    &observation.observation_id,
                    "transform_failed",
                    observation.created_at,
                )
                .await?;
                continue;
            }
        };
        match registration.adapter.transform(&config, &adapter_observation) {
            Ok(publication) => {
                let result = sqlx::query(
                    "INSERT OR IGNORE INTO output_outbox(export_id,route_id,observation_id,\
                     topic,qos,retain,payload_json,created_at) VALUES(?,?,?,?,?,?,?,?)",
                )
                .bind(deterministic_export_id(
                    &route_id,
                    &observation.observation_id,
                ))
                .bind(&route_id)
                .bind(&observation.observation_id)
                .bind(publication.topic())
                .bind(i64::from(publication.qos()))
                .bind(publication.retain())
                .bind(publication.payload().get().as_bytes())
                .bind(observation.created_at)
                .execute(&mut **tx)
                .await?;
                inserted += result.rows_affected() as usize;
                sqlx::query(
                    "DELETE FROM output_route_attempts WHERE route_id=? AND observation_id=?",
                )
                .bind(&route_id)
                .bind(&observation.observation_id)
                .execute(&mut **tx)
                .await?;
                sqlx::query(
                    "UPDATE output_routes SET last_transform_error_code=NULL,\
                     last_transform_error_at=NULL,last_transform_success_at=? WHERE route_id=?",
                )
                .bind(observation.created_at)
                .bind(&route_id)
                .execute(&mut **tx)
                .await?;
            }
            Err(error) => {
                record_route_error_sqlite(
                    tx,
                    &route_id,
                    &observation.observation_id,
                    transform_error_code(error),
                    observation.created_at,
                )
                .await?;
            }
        }
    }
    Ok(inserted)
}

async fn record_route_error_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    route_id: &str,
    observation_id: &str,
    code: &str,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE output_routes SET last_transform_error_code=?,last_transform_error_at=? \
         WHERE route_id=?",
    )
    .bind(code)
    .bind(now)
    .bind(route_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO output_route_attempts(route_id,observation_id,attempts,last_attempt_at,error_code) \
         VALUES(?,?,1,?,?) ON CONFLICT(route_id,observation_id) DO UPDATE SET \
         attempts=attempts+1,last_attempt_at=excluded.last_attempt_at,error_code=excluded.error_code",
    )
    .bind(route_id)
    .bind(observation_id)
    .bind(now)
    .bind(code)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn update_runtime_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    candidate: &ProjectionCandidate,
    state: EvaluationState,
    next_sequence: u64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE semantic_rule_runtime SET initialized=?,detector_active=?,counter=?,pending=?,\
         pending_active=?,pending_since=?,applied_revision=?,applied_calibration_revision=?,\
         applied_ledger_epoch=?,applied_series_id=?,next_sequence=? WHERE rule_id=?",
    )
    .bind(state.initialized)
    .bind(state.active)
    .bind(state.counter)
    .bind(state.pending)
    .bind(state.pending_active)
    .bind(state.pending_since)
    .bind(candidate.current_revision)
    .bind(candidate.current_calibration_revision)
    .bind(&candidate.ledger_epoch)
    .bind(&candidate.current_series_id)
    .bind(next_sequence as i64)
    .bind(&candidate.rule_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn ready_reset_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Option<ProducedObservation>, StorageError> {
    let row = sqlx::query(
        "SELECT reset.reset_id,reset.rule_id,reset.requested_at,rule.signal_ref,\
         signal.edge_node_id,rule.series_id,rule.revision,signal.calibration_revision,\
         runtime.next_sequence,COALESCE(boundary.ledger_epoch,'' ) ledger_epoch,\
         COALESCE(boundary.apply_after_pub_seq,0) source_pub_seq \
         FROM semantic_counter_resets AS reset JOIN semantic_rules AS rule \
         ON rule.rule_id=reset.rule_id JOIN semantic_signals AS signal \
         ON signal.signal_ref=rule.signal_ref JOIN semantic_rule_runtime AS runtime \
         ON runtime.rule_id=rule.rule_id LEFT JOIN semantic_counter_reset_boundaries AS boundary \
         ON boundary.reset_id=reset.reset_id WHERE reset.applied_at IS NULL \
         AND NOT EXISTS(SELECT 1 FROM semantic_counter_reset_boundaries pending \
           JOIN raw_records raw ON raw.edge_node_id=signal.edge_node_id \
             AND raw.ledger_epoch=pending.ledger_epoch \
             AND raw.pub_seq<=pending.apply_after_pub_seq \
           WHERE pending.reset_id=reset.reset_id AND json_extract(raw.record_json,'$.series_key')=signal.series_key \
             AND NOT EXISTS(SELECT 1 FROM semantic_projection_receipts receipt \
               WHERE receipt.rule_id=reset.rule_id AND receipt.ledger_epoch=raw.ledger_epoch \
               AND receipt.pub_seq=raw.pub_seq)) \
         ORDER BY reset.requested_at,reset.reset_id,boundary.ledger_epoch LIMIT 1",
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let reset_id: String = row.try_get("reset_id")?;
    let observation_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("semantic-v3-reset:{reset_id}").as_bytes(),
    )
    .to_string();
    let sequence = u64::try_from(row.try_get::<i64, _>("next_sequence")?)
        .map_err(|_| StorageError::InvalidSemantic("invalid reset sequence".into()))?;
    let produced = ProducedObservation {
        observation_id: observation_id.clone(),
        rule_id: row.try_get("rule_id")?,
        revision: row.try_get("revision")?,
        calibration_revision: row.try_get("calibration_revision")?,
        series_id: row.try_get("series_id")?,
        sequence,
        kind: SemanticKind::CumulativeCounter,
        value_json: b"0".to_vec(),
        value: ObservationValue::CumulativeValue(0),
        reading: None,
        signal_ref: row.try_get("signal_ref")?,
        edge_node_id: row.try_get("edge_node_id")?,
        ledger_epoch: row.try_get("ledger_epoch")?,
        source_pub_seq: row.try_get("source_pub_seq")?,
        observed_at: row.try_get("requested_at")?,
        created_at: row.try_get("requested_at")?,
    };
    insert_observation_sqlite(tx, &produced).await?;
    sqlx::query(
        "UPDATE semantic_rule_runtime SET initialized=0,detector_active=0,counter=0,pending=0,\
         pending_active=0,pending_since=0,applied_revision=?,\
         applied_calibration_revision=?,applied_ledger_epoch=?,applied_series_id=?,\
         next_sequence=? WHERE rule_id=?",
    )
    .bind(produced.revision)
    .bind(produced.calibration_revision)
    .bind(&produced.ledger_epoch)
    .bind(&produced.series_id)
    .bind(sequence as i64 + 1)
    .bind(&produced.rule_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE semantic_counter_resets SET applied_at=?,zero_observation_id=? \
         WHERE reset_id=? AND applied_at IS NULL",
    )
    .bind(produced.created_at)
    .bind(&observation_id)
    .bind(&reset_id)
    .execute(&mut **tx)
    .await?;
    Ok(Some(produced))
}

async fn reconcile_profiles_sqlite(
    tx: &mut Transaction<'_, Sqlite>,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE output_bindings SET state='stopped',revision=revision+1,stopped_at=? \
         WHERE state='draining' AND NOT EXISTS(SELECT 1 FROM output_routes route \
           JOIN output_outbox outbox ON outbox.route_id=route.route_id \
           WHERE route.binding_id=output_bindings.binding_id AND outbox.published_at IS NULL)",
    )
    .bind(now)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE output_routes SET active=0,lifecycle_state='stopped' WHERE binding_id IN \
         (SELECT binding_id FROM output_bindings WHERE state='stopped')",
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE export_profiles SET state='stopped',stopped_at=?,revision=revision+1 \
         WHERE state='draining' AND NOT EXISTS(SELECT 1 FROM output_bindings binding \
           WHERE binding.profile_id=export_profiles.profile_id \
           AND binding.state NOT IN ('stopped','ineligible'))",
    )
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
