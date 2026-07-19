package sitehttp

import (
	"errors"
	"net/http"
	"strconv"
	"strings"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

func (server *Server) listEdges(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	edges, err := server.site.ListEdges(request.Context())
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []siteapp.Edge `json:"items"`
	}{edges})
}

func (server *Server) activateEdge(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	result, err := server.site.Dispatch(
		request.Context(),
		server.actor(auth),
		siteapp.ActivateEdge{
			EdgeRef:      request.PathValue("edge_ref"),
			Precondition: revisionPrecondition(request),
		},
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	response.Header().Set("ETag", revisionETag(result.Edge.Revision))
	writeJSON(response, http.StatusAccepted, result.Edge)
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
	result, err := server.site.Dispatch(request.Context(), server.actor(auth), siteapp.UpdateDeviceProfile{
		DeviceRef: request.PathValue("device_ref"),
		Input: siteapp.DeviceProfileInput{
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
	result, err := server.site.Dispatch(request.Context(), server.actor(auth), siteapp.UpdateSignalProfile{
		SignalRef: request.PathValue("signal_ref"),
		Input: siteapp.SignalProfileInput{
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
	if auth.account.Role != siteapp.AccountRoleAdmin &&
		auth.account.Role != siteapp.AccountRoleSystemAdmin {
		server.operationError(response, siteapp.ErrForbidden)
		return
	}
	devices, err := server.site.ListSetupDevices(
		request.Context(),
		server.actor(auth),
		100,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []siteapp.SetupDevice `json:"items"`
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

func (server *Server) listYokaKitOutputs(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	routes, err := server.store.ListYokaKitRoutes(request.Context())
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items any `json:"items"`
	}{routes})
}

func (server *Server) createYokaKitOutput(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		DefinitionID string                    `json:"definition_id"`
		SourceID     string                    `json:"source_id"`
		SignalID     string                    `json:"signal_id"`
		Kind         outputadapter.YokaKitKind `json:"kind"`
		Reason       string                    `json:"reason"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	route, err := server.store.ApplyYokaKitRoute(
		request.Context(), server.actor(auth), input.DefinitionID,
		outputadapter.YokaKit{
			SourceID: input.SourceID, SignalID: input.SignalID,
			Kind: input.Kind, Reason: input.Reason,
		},
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusCreated, route)
}

func (server *Server) listAuditEvents(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	events, err := server.site.ListAuditEvents(request.Context(), 100)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, struct {
		Items []siteapp.AuditEvent `json:"items"`
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
		Items []siteapp.Account `json:"items"`
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
		Role              siteapp.AccountRole `json:"role"`
		TemporaryPassword string              `json:"temporary_password"`
	}
	if err := decodeJSON(response, request, &input); err != nil {
		server.badRequest(response)
		return
	}
	result, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth),
		siteapp.CreateAccount{
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
		siteapp.ChangeOwnPassword{
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
	if auth.account.Role != siteapp.AccountRoleAdmin &&
		auth.account.Role != siteapp.AccountRoleSystemAdmin {
		server.operationError(response, siteapp.ErrForbidden)
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) actor(auth requestAuth) siteapp.Actor {
	return siteapp.AccountActor(auth.account.AccountRef, auth.account.Role)
}

func revisionPrecondition(request *http.Request) siteapp.RevisionPrecondition {
	raw := strings.Trim(request.Header.Get("If-Match"), `"`)
	if raw == "" || raw == "*" {
		return siteapp.RevisionPrecondition{}
	}
	value, err := strconv.ParseInt(raw, 10, 64)
	if err != nil || value < 1 {
		impossible := int64(-1)
		return siteapp.RevisionPrecondition{Expected: &impossible}
	}
	return siteapp.RevisionPrecondition{Expected: &value}
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
	case errors.Is(err, siteapp.ErrForbidden):
		status, code, message = http.StatusForbidden, "forbidden", "この操作を行う権限がありません。"
	case errors.Is(err, siteapp.ErrNotFound):
		status, code, message = http.StatusNotFound, "not_found", "対象が見つかりません。"
	case errors.Is(err, siteapp.ErrRevisionMismatch):
		status, code, message = http.StatusPreconditionFailed, "revision_mismatch", "ほかの変更が先に保存されています。"
	}
	server.writeError(response, status, code, message, nil)
}
