package sitehttp

import (
	"net/http"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

const (
	previewLifetime = 5 * time.Minute
	maxPreviews     = 5
)

type semanticPreview struct {
	ID         string
	SessionRef string
	SignalRef  string
	Spec       semantics.DefinitionSpec
	Boundary   int64
	State      semantics.State
	Samples    []semantics.PreviewSample
	ExpiresAt  time.Time
}

func (server *Server) createMappingPreview(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAdminMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		SignalRef string                   `json:"signal_ref"`
		Spec      semantics.DefinitionSpec `json:"spec"`
	}
	if err := decodeJSON(response, request, &input); err != nil ||
		input.Spec.Validate() != nil {
		server.badRequest(response)
		return
	}
	boundary, err := server.store.SemanticPreviewBoundary(request.Context(), input.SignalRef)
	if err != nil {
		server.operationError(response, err)
		return
	}
	server.previewMu.Lock()
	defer server.previewMu.Unlock()
	server.expirePreviews()
	for _, existing := range server.previews {
		if existing.SessionRef == auth.principal.SessionRef {
			server.writeError(response, http.StatusConflict, "preview_exists",
				"この画面ではすでにプレビューを実行しています。", nil)
			return
		}
	}
	if len(server.previews) >= maxPreviews {
		server.writeError(response, http.StatusServiceUnavailable, "preview_busy",
			"プレビューが混み合っています。しばらく待ってください。", nil)
		return
	}
	preview := &semanticPreview{
		ID: newRequestID(), SessionRef: auth.principal.SessionRef,
		SignalRef: input.SignalRef, Spec: input.Spec, Boundary: boundary,
		ExpiresAt: server.now().Add(previewLifetime),
	}
	server.previews[preview.ID] = preview
	writeJSON(response, http.StatusCreated, previewResponse(preview))
}

func (server *Server) getMappingPreview(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireAPIAuth(response, request, false)
	if !ok {
		return
	}
	server.previewMu.Lock()
	defer server.previewMu.Unlock()
	server.expirePreviews()
	preview := server.previews[request.PathValue("preview_id")]
	if preview == nil || preview.SessionRef != auth.principal.SessionRef {
		server.writeError(response, http.StatusNotFound, "not_found",
			"プレビューが見つかりません。", nil)
		return
	}
	var inputs []store.SemanticPreviewInput
	var err error
	if len(preview.Samples) < 100 {
		inputs, err = server.store.ListSemanticPreviewInputs(
			request.Context(), preview.SignalRef, preview.Boundary, 100-len(preview.Samples),
		)
	}
	if err != nil {
		server.operationError(response, err)
		return
	}
	for _, input := range inputs {
		result, next, err := semantics.Evaluate(preview.Spec, preview.State, input.Value)
		if err != nil {
			server.operationError(response, err)
			return
		}
		preview.State = next
		preview.Boundary = input.RawRowID
		preview.Samples = append(preview.Samples, semantics.PreviewSample{
			SourcePubSeq: input.SourcePubSeq, ObservedAt: input.ObservedAt,
			Input: input.Value, Result: result,
		})
	}
	writeJSON(response, http.StatusOK, previewResponse(preview))
}

func (server *Server) deleteMappingPreview(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireMutation(response, request)
	if !ok {
		return
	}
	server.previewMu.Lock()
	defer server.previewMu.Unlock()
	preview := server.previews[request.PathValue("preview_id")]
	if preview == nil || preview.SessionRef != auth.principal.SessionRef {
		server.writeError(response, http.StatusNotFound, "not_found",
			"プレビューが見つかりません。", nil)
		return
	}
	delete(server.previews, preview.ID)
	response.WriteHeader(http.StatusNoContent)
}

func (server *Server) expirePreviews() {
	now := server.now()
	for id, preview := range server.previews {
		if !preview.ExpiresAt.After(now) {
			delete(server.previews, id)
		}
	}
}

func previewResponse(preview *semanticPreview) any {
	return struct {
		PreviewID string                    `json:"preview_id"`
		ExpiresAt int64                     `json:"expires_at"`
		Samples   []semantics.PreviewSample `json:"samples"`
	}{
		PreviewID: preview.ID, ExpiresAt: preview.ExpiresAt.UnixMilli(),
		Samples: preview.Samples,
	}
}
