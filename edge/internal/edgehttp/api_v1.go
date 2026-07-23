package edgehttp

import (
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

func (server *Server) listEdgeNodes(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	edgeNodes, err := server.edge.ListEdgeNodes(request.Context())
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []edgeapp.EdgeNode `json:"items"`
	}{edgeNodes})
}

func (server *Server) activateEdgeNode(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	result, err := server.edge.Dispatch(
		request.Context(),
		server.actor(auth),
		edgeapp.ActivateEdgeNode{
			EdgeNodeRef:  request.PathValue("edge_node_ref"),
			Precondition: revisionPrecondition(request),
		},
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(result.EdgeNode.Revision))
	writeJSON(response, http.StatusAccepted, result.EdgeNode)
}

func (server *Server) putDeviceProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		DisplayName string `json:"display_name"`
		Location    string `json:"location"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	result, err := server.edge.Dispatch(request.Context(), server.actor(auth), edgeapp.UpdateDeviceProfile{
		DeviceRef: request.PathValue("device_ref"),
		Input: edgeapp.DeviceProfileInput{
			DisplayName: input.DisplayName,
			Location:    input.Location,
		},
		Precondition: revisionPrecondition(request),
	})
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(result.DeviceProfile.Revision))
	writeJSON(response, http.StatusOK, result.DeviceProfile)
}

func (server *Server) putSignalProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		DisplayName            string `json:"display_name"`
		DisplaySensorType      string `json:"display_sensor_type"`
		DisplaySensorTypeLabel string `json:"display_sensor_type_label"`
		DisplayValueKind       string `json:"display_value_kind"`
		DisplayUnitMode        string `json:"display_unit_mode"`
		DisplayUnit            string `json:"display_unit"`
		DecimalPlaces          int    `json:"decimal_places"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	result, err := server.edge.Dispatch(request.Context(), server.actor(auth), edgeapp.UpdateSignalProfile{
		SignalRef: request.PathValue("signal_ref"),
		Input: edgeapp.SignalProfileInput{
			DisplayName:            input.DisplayName,
			DisplaySensorType:      input.DisplaySensorType,
			DisplaySensorTypeLabel: input.DisplaySensorTypeLabel,
			DisplayValueKind:       input.DisplayValueKind,
			DisplayUnitMode:        input.DisplayUnitMode,
			DisplayUnit:            input.DisplayUnit,
			DecimalPlaces:          input.DecimalPlaces,
		},
		Precondition: revisionPrecondition(request),
	})
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(result.SignalProfile.Revision))
	writeJSON(response, http.StatusOK, result.SignalProfile)
}

func (server *Server) listSetupDevices(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAPIAuth(response, request, false)
	if !ok {
		return
	}
	if auth.account.Role != edgeapp.AccountRoleAdmin &&
		auth.account.Role != edgeapp.AccountRoleSystemAdmin {
		server.operationError(response, edgeapp.ErrForbidden)
		return
	}
	devices, err := server.edge.ListSetupDevices(
		request.Context(),
		server.actor(auth),
		100,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []edgeapp.SetupDevice `json:"items"`
	}{devices})
}

func (server *Server) listSemanticDefinitions(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	definitions, err := server.semantics.List(request.Context())
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []semantics.Definition `json:"items"`
	}{definitions})
}

func (server *Server) putSemanticDefinition(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var spec semantics.DefinitionSpec
	if err := decodeJSON(response, request, &spec); err != nil {
		server.badRequest(response)
		return
	}
	definition, err := server.semantics.Put(
		request.Context(),
		server.actor(auth),
		request.PathValue("signal_ref"),
		spec,
		revisionPrecondition(request),
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(definition.Revision))
	writeJSON(response, http.StatusOK, definition)
}

func (server *Server) deleteSemanticDefinition(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	definition, err := server.semantics.Deactivate(
		request.Context(), server.actor(auth), request.PathValue("signal_ref"),
		revisionPrecondition(request),
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, definition)
}

func (server *Server) resetSemanticCounter(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	definition, err := server.semantics.ResetCounter(
		request.Context(),
		server.actor(auth),
		request.PathValue("signal_ref"),
		revisionPrecondition(request),
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(definition.Revision))
	writeJSON(response, http.StatusOK, definition)
}

func (server *Server) getSemanticConfiguration(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAPIAuth(response, request, false)
	if !ok {
		return
	}
	configuration, err := server.semanticConfig.Get(
		request.Context(),
		server.actor(auth),
		request.PathValue("signal_ref"),
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(configuration.Revision))
	writeJSON(response, http.StatusOK, configuration)
}

func (server *Server) putSignalCalibration(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	precondition, ok := server.requireRevisionPrecondition(response, request)
	if !ok {
		return
	}
	var input struct {
		Scale  float64 `json:"scale"`
		Offset float64 `json:"offset"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	configuration, err := server.semanticConfig.UpdateCalibration(
		request.Context(),
		server.actor(auth),
		request.PathValue("signal_ref"),
		input.Scale,
		input.Offset,
		precondition,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(configuration.Revision))
	writeJSON(response, http.StatusOK, configuration)
}

func (server *Server) createSemanticRule(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	precondition, ok := server.requireRevisionPrecondition(response, request)
	if !ok {
		return
	}
	var input struct {
		DisplayName string             `json:"display_name"`
		Spec        semantics.RuleSpec `json:"spec"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	rule, err := server.semanticConfig.CreateRule(
		request.Context(),
		server.actor(auth),
		request.PathValue("signal_ref"),
		input.DisplayName,
		input.Spec,
		precondition,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	// Creating a rule advances the containing configuration revision. Return
	// that revision so clients can add another rule without an extra GET.
	response.Header().Set(
		"ETag",
		revisionETag(*precondition.Expected+1),
	)
	writeJSON(response, http.StatusCreated, rule)
}

func (server *Server) updateSemanticRule(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	precondition, ok := server.requireRevisionPrecondition(response, request)
	if !ok {
		return
	}
	var input struct {
		DisplayName string             `json:"display_name"`
		Spec        semantics.RuleSpec `json:"spec"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	rule, err := server.semanticConfig.UpdateRule(
		request.Context(),
		server.actor(auth),
		request.PathValue("rule_id"),
		input.DisplayName,
		input.Spec,
		precondition,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(rule.Revision))
	writeJSON(response, http.StatusOK, rule)
}

func (server *Server) retireSemanticRule(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	precondition, ok := server.requireRevisionPrecondition(response, request)
	if !ok {
		return
	}
	rule, err := server.semanticConfig.RetireRule(
		request.Context(),
		server.actor(auth),
		request.PathValue("rule_id"),
		precondition,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, rule)
}

func (server *Server) requestSemanticCounterReset(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	resetID := request.Header.Get("Idempotency-Key")
	reset, err := server.semanticConfig.RequestCounterReset(
		request.Context(),
		server.actor(auth),
		request.PathValue("rule_id"),
		resetID,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusAccepted, reset)
}

func (server *Server) deprecatedSemanticMutation(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireAdminMutation(response, request); !ok {
		return
	}
	server.writeError(
		response,
		http.StatusGone,
		"semantic_definition_retired",
		"この操作は複数ルール設定へ移行しました。semantic-configuration APIを使用してください。",
		nil,
	)
}

func (server *Server) listOutputAdapters(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	registry, err := outputadapter.BuiltInRegistry()
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []outputadapter.Descriptor `json:"items"`
	}{registry.Descriptors()})
}

func (server *Server) listExportProfiles(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	profiles, err := server.store.ListExportProfiles(request.Context())
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []edgeapp.ExportProfile `json:"items"`
	}{profiles})
}

func (server *Server) activateExportProfile(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		DisplayName         string `json:"display_name"`
		AdapterID           string `json:"adapter_id"`
		AutoBindFutureRules bool   `json:"auto_bind_future_rules"`
	}
	if err := decodeJSON(response, request, &input); err != nil ||
		input.DisplayName == "" ||
		input.AdapterID == "" ||
		!input.AutoBindFutureRules {
		server.badRequest(response)
		return
	}
	profile, err := server.store.ActivateExportProfile(
		request.Context(),
		server.actor(auth),
		input.DisplayName,
		input.AdapterID,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(profile.Revision))
	writeJSON(response, http.StatusCreated, profile)
}

func (server *Server) previewExportProfile(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	var input struct {
		AdapterID string `json:"adapter_id"`
	}
	if err := decodeJSON(response, request, &input); err != nil ||
		input.AdapterID == "" {
		server.badRequest(response)
		return
	}
	preview, err := server.store.PreviewExportProfileActivation(
		request.Context(), input.AdapterID,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, preview)
}

func (server *Server) configureExportBinding(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		Mode             string `json:"mode"`
		ExpectedRevision int64  `json:"expected_revision"`
	}
	if err := decodeJSON(response, request, &input); err != nil ||
		input.Mode == "" ||
		input.ExpectedRevision < 1 {
		server.badRequest(response)
		return
	}
	binding, err := server.store.ConfigurePinikietBooleanBinding(
		request.Context(),
		server.actor(auth),
		request.PathValue("binding_id"),
		input.Mode,
		input.ExpectedRevision,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(binding.Revision))
	writeJSON(response, http.StatusOK, binding)
}

func (server *Server) stopExportProfile(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		ExpectedRevision int64 `json:"expected_revision"`
	}
	if err := decodeJSON(response, request, &input); err != nil ||
		input.ExpectedRevision < 1 {
		server.badRequest(response)
		return
	}
	profile, err := server.store.RequestExportProfileStop(
		request.Context(),
		server.actor(auth),
		request.PathValue("profile_id"),
		input.ExpectedRevision,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(profile.Revision))
	writeJSON(response, http.StatusAccepted, profile)
}

func (server *Server) getOutputBindingPublication(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	preview, err := server.store.GetOutputBindingPublication(
		request.Context(),
		request.PathValue("binding_id"),
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, preview)
}

func (server *Server) startOutputBinding(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		ExpectedRevision             int64 `json:"expected_revision"`
		ExternalRegistrationComplete bool  `json:"external_registration_complete"`
	}
	if err := decodeJSON(response, request, &input); err != nil ||
		input.ExpectedRevision < 1 ||
		!input.ExternalRegistrationComplete {
		server.badRequest(response)
		return
	}
	binding, err := server.store.StartPreparedOutputBinding(
		request.Context(),
		server.actor(auth),
		request.PathValue("binding_id"),
		input.ExpectedRevision,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(binding.Revision))
	writeJSON(response, http.StatusOK, binding)
}

func (server *Server) listOutputRoutes(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	routes, err := server.store.ListOutputRoutes(request.Context())
	if err != nil {
		server.operationError(response, err)
		return
	}
	routes = profileOutputRoutes(routes)
	writeJSON(response, http.StatusOK, struct {
		Items []edgeapp.OutputRoute `json:"items"`
	}{routes})
}

func (server *Server) listAuditEvents(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	events, err := server.edge.ListAuditEvents(request.Context(), 100)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []edgeapp.AuditEvent `json:"items"`
	}{events})
}

func (server *Server) listAccounts(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAPIAuth(response, request, false)
	if !ok {
		return
	}
	accounts, err := server.accounts.ListAccounts(request.Context(), server.actor(auth))
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []edgeapp.Account `json:"items"`
	}{accounts})
}

func (server *Server) createAccount(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		LoginID           string              `json:"login_id"`
		DisplayName       string              `json:"display_name"`
		Role              edgeapp.AccountRole `json:"role"`
		TemporaryPassword string              `json:"temporary_password"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	result, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth),
		edgeapp.CreateAccount{
			LoginID: input.LoginID, DisplayName: input.DisplayName,
			Role: input.Role, TemporaryPassword: input.TemporaryPassword,
		},
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusCreated, result.Account)
}

func (server *Server) changeOwnPassword(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAPIAuth(response, request, true)
	if !ok || !server.authorizeMutation(response, request, auth.token) {
		return
	}
	var input struct {
		CurrentPassword string `json:"current_password"`
		NewPassword     string `json:"new_password"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	result, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth),
		edgeapp.ChangeOwnPassword{
			CurrentPassword: input.CurrentPassword,
			NewPassword:     input.NewPassword,
		},
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	if err := server.sessions.Logout(request.Context(), auth.token); err != nil {
		server.operationError(response, err)
		return
	}
	server.clearSessionCookies(response)
	writeJSON(response, http.StatusOK, result.Account)
}

func (server *Server) requireMutation(
	response http.ResponseWriter,
	request *http.Request,
) (requestAuth, bool) {
	auth, ok := server.requireAPIAuth(response, request, false)
	if !ok || !server.authorizeMutation(response, request, auth.token) {
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) requireAdminMutation(
	response http.ResponseWriter,
	request *http.Request,
) (requestAuth, bool) {
	auth, ok := server.requireMutation(response, request)
	if !ok {
		return requestAuth{}, false
	}
	if auth.account.Role != edgeapp.AccountRoleAdmin &&
		auth.account.Role != edgeapp.AccountRoleSystemAdmin {
		server.operationError(response, edgeapp.ErrForbidden)
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) actor(auth requestAuth) edgeapp.Actor {
	return edgeapp.AccountActor(auth.account.AccountRef, auth.account.Role)
}

func revisionPrecondition(request *http.Request) edgeapp.RevisionPrecondition {
	raw := strings.Trim(request.Header.Get("If-Match"), `"`)
	if raw == "" || raw == "*" {
		return edgeapp.RevisionPrecondition{}
	}
	value, err := strconv.ParseInt(raw, 10, 64)
	if err != nil || value < 1 {
		impossible := int64(-1)
		return edgeapp.RevisionPrecondition{Expected: &impossible}
	}
	return edgeapp.RevisionPrecondition{Expected: &value}
}

func (server *Server) requireRevisionPrecondition(
	response http.ResponseWriter,
	request *http.Request,
) (edgeapp.RevisionPrecondition, bool) {
	raw := strings.TrimSpace(request.Header.Get("If-Match"))
	if raw == "" || raw == "*" {
		server.writeError(
			response,
			http.StatusPreconditionRequired,
			"precondition_required",
			"最新の設定を読み直してから保存してください。",
			nil,
		)
		return edgeapp.RevisionPrecondition{}, false
	}
	return revisionPrecondition(request), true
}

func revisionETag(revision int64) string {
	return `"` + strconv.FormatInt(revision, 10) + `"`
}

func (server *Server) badRequest(response http.ResponseWriter) {
	server.writeError(response, http.StatusBadRequest, "invalid_request",
		"入力内容を確認してください。", nil)
}

func (server *Server) operationError(response http.ResponseWriter, err error) {
	status, code, message := http.StatusBadRequest, "invalid_request", "入力内容を確認してください。"
	switch {
	case errors.Is(err, edgeapp.ErrForbidden):
		status, code, message = http.StatusForbidden, "forbidden", "この操作を行う権限がありません。"
	case errors.Is(err, edgeapp.ErrNotFound):
		status, code, message = http.StatusNotFound, "not_found", "対象が見つかりません。"
	case errors.Is(err, edgeapp.ErrRevisionMismatch):
		status, code, message = http.StatusPreconditionFailed, "revision_mismatch", "ほかの変更が先に保存されています。"
	}
	server.writeError(response, status, code, message, nil)
}
