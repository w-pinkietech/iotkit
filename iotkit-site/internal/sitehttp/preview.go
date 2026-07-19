package sitehttp

import (
	"math"
	"net/http"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

const (
	previewHistoryLimit = 20_000
	previewPlotLimit    = 300
	previewHistoryAge   = time.Hour
	previewCacheAge     = time.Second
)

type cachedPreviewWindow struct {
	window    store.SemanticPreviewWindow
	expiresAt time.Time
}

func (server *Server) createMappingPreview(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireMutation(response, request)
	if !ok {
		return
	}
	var input struct {
		SignalRef string                    `json:"signal_ref"`
		Spec      *semantics.DefinitionSpec `json:"spec"`
		TestValue *float64                  `json:"test_value"`
	}
	if err := decodeJSON(response, request, &input); err != nil ||
		input.SignalRef == "" {
		server.badRequest(response)
		return
	}
	if input.Spec != nil &&
		auth.account.Role != siteapp.AccountRoleAdmin &&
		auth.account.Role != siteapp.AccountRoleSystemAdmin {
		server.operationError(response, siteapp.ErrForbidden)
		return
	}
	if input.Spec != nil {
		if field, err := previewSpecValidationField(*input.Spec); err != nil {
			server.writeError(response, http.StatusBadRequest, "invalid_request",
				"入力内容を確認してください。", &field)
			return
		}
	}

	spec, err := server.previewSpec(request, input.SignalRef, input.Spec)
	if err != nil {
		server.operationError(response, err)
		return
	}
	window, err := server.previewWindow(request, input.SignalRef)
	if err != nil {
		server.operationError(response, err)
		return
	}
	preview, err := semantics.BuildPreview(
		spec,
		window.Inputs,
		previewPlotLimit,
		input.TestValue,
	)
	if err != nil {
		server.badRequest(response)
		return
	}
	var riseThreshold *float64
	var fallThreshold *float64
	if spec.Detector.Mode == semantics.DetectorHighActive ||
		spec.Detector.Mode == semantics.DetectorLowActive {
		rise := spec.Detector.RiseThreshold
		fall := spec.Detector.FallThreshold
		riseThreshold = &rise
		fallThreshold = &fall
	}
	writeJSON(response, http.StatusOK, struct {
		Kind semantics.Kind `json:"kind"`
		semantics.Preview
		WindowStart   int64    `json:"window_start,omitempty"`
		WindowEnd     int64    `json:"window_end,omitempty"`
		TruncatedBy   string   `json:"truncated_by,omitempty"`
		RiseThreshold *float64 `json:"rise_threshold,omitempty"`
		FallThreshold *float64 `json:"fall_threshold,omitempty"`
	}{
		Kind:          spec.Kind,
		Preview:       preview,
		WindowStart:   window.WindowStart,
		WindowEnd:     window.WindowEnd,
		TruncatedBy:   window.TruncatedBy,
		RiseThreshold: riseThreshold,
		FallThreshold: fallThreshold,
	})
}

func (server *Server) previewSpec(
	request *http.Request,
	signalRef string,
	draft *semantics.DefinitionSpec,
) (semantics.DefinitionSpec, error) {
	if draft != nil {
		if err := draft.Validate(); err != nil {
			return semantics.DefinitionSpec{}, err
		}
		return *draft, nil
	}
	definitions, err := server.semantics.List(request.Context())
	if err != nil {
		return semantics.DefinitionSpec{}, err
	}
	for _, definition := range definitions {
		if definition.SignalRef == signalRef && definition.Active {
			return definition.DefinitionSpec, nil
		}
	}
	return semantics.DefinitionSpec{}, siteapp.ErrNotFound
}

func previewSpecValidationField(spec semantics.DefinitionSpec) (string, error) {
	err := spec.Validate()
	if err == nil {
		return "", nil
	}
	if !previewFinite(spec.Scale) || spec.Scale == 0 {
		return "scale", err
	}
	if !previewFinite(spec.Offset) {
		return "offset", err
	}
	if !previewFinite(spec.Detector.RiseThreshold) {
		return "rise_threshold", err
	}
	if !previewFinite(spec.Detector.FallThreshold) {
		return "fall_threshold", err
	}
	if spec.Detector.RiseDebounceMS < 0 ||
		spec.Detector.RiseDebounceMS > 300_000 {
		return "rise_debounce_seconds", err
	}
	if spec.Detector.FallDebounceMS < 0 ||
		spec.Detector.FallDebounceMS > 300_000 {
		return "fall_debounce_seconds", err
	}
	switch spec.Kind {
	case semantics.KindNumeric:
		return "kind", err
	case semantics.KindBoolean:
		if !previewDetectorSupported(spec.Detector.Mode) {
			return "detector_mode", err
		}
		return "kind", err
	case semantics.KindCumulativeCounter:
		if !previewDetectorSupported(spec.Detector.Mode) {
			return "detector_mode", err
		}
		return "trigger", err
	case semantics.KindAlarm:
		if !previewDetectorSupported(spec.Detector.Mode) {
			return "detector_mode", err
		}
		return "kind", err
	default:
		return "kind", err
	}
}

func previewDetectorSupported(mode semantics.DetectorMode) bool {
	return mode == semantics.DetectorBooleanHighActive ||
		mode == semantics.DetectorBooleanLowActive ||
		mode == semantics.DetectorHighActive ||
		mode == semantics.DetectorLowActive
}

func previewFinite(value float64) bool {
	return !math.IsNaN(value) && !math.IsInf(value, 0)
}

func (server *Server) previewWindow(
	request *http.Request,
	signalRef string,
) (store.SemanticPreviewWindow, error) {
	now := server.now()
	server.previewMu.Lock()
	cached, ok := server.previewCache[signalRef]
	if ok && cached.expiresAt.After(now) {
		server.previewMu.Unlock()
		return cached.window, nil
	}
	server.previewMu.Unlock()

	window, err := server.store.ListSemanticPreviewWindow(
		request.Context(),
		signalRef,
		now.Add(-previewHistoryAge).UnixMilli(),
		previewHistoryLimit,
	)
	if err != nil {
		return store.SemanticPreviewWindow{}, err
	}
	server.previewMu.Lock()
	server.previewCache[signalRef] = cachedPreviewWindow{
		window:    window,
		expiresAt: now.Add(previewCacheAge),
	}
	server.previewMu.Unlock()
	return window, nil
}
