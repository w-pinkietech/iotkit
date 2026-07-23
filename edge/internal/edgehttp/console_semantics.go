package edgehttp

import (
	"errors"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"net/http"
	"strconv"
)

func (server *Server) consoleSemantic(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	scale, _ := strconv.ParseFloat(request.FormValue("scale"), 64)
	offset, _ := strconv.ParseFloat(request.FormValue("offset"), 64)
	riseThreshold, _ := strconv.ParseFloat(request.FormValue("rise_threshold"), 64)
	fallThreshold, _ := strconv.ParseFloat(request.FormValue("fall_threshold"), 64)
	riseDebounceSeconds, _ := strconv.ParseFloat(
		request.FormValue("rise_debounce_seconds"), 64,
	)
	fallDebounceSeconds, _ := strconv.ParseFloat(
		request.FormValue("fall_debounce_seconds"), 64,
	)
	spec := semantics.DefinitionSpec{
		Kind: semantics.Kind(request.FormValue("kind")), Scale: scale, Offset: offset,
		Detector: semantics.Detector{
			Mode:          semantics.DetectorMode(request.FormValue("detector_mode")),
			RiseThreshold: riseThreshold, FallThreshold: fallThreshold,
			RiseDebounceMS: int64(riseDebounceSeconds * 1000),
			FallDebounceMS: int64(fallDebounceSeconds * 1000),
		},
		Trigger: semantics.TriggerMode(request.FormValue("trigger")),
	}
	_, err := server.semantics.Put(
		request.Context(), server.actor(auth), request.PathValue("signal_ref"),
		spec, edgeapp.RevisionPrecondition{Expected: formRevision(request)},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) consoleSemanticCounterReset(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.semantics.ResetCounter(
		request.Context(),
		server.actor(auth),
		request.PathValue("signal_ref"),
		edgeapp.RevisionPrecondition{Expected: formRevision(request)},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) deprecatedConsoleSemanticMutation(
	response http.ResponseWriter,
	request *http.Request,
) {
	if _, ok := server.requireBrowserMutation(response, request, true); !ok {
		return
	}
	http.Error(
		response,
		"画面を再読み込みして、ルールごとの設定を使用してください。",
		http.StatusGone,
	)
}

func (server *Server) consoleSignalCalibration(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	scale, scaleErr := strconv.ParseFloat(request.FormValue("scale"), 64)
	offset, offsetErr := strconv.ParseFloat(request.FormValue("offset"), 64)
	var err error
	if scaleErr != nil || offsetErr != nil {
		err = semantics.Calibration{}.Validate()
	} else {
		_, err = server.semanticConfig.UpdateCalibration(
			request.Context(),
			server.actor(auth),
			request.PathValue("signal_ref"),
			scale,
			offset,
			edgeapp.RevisionPrecondition{Expected: revision},
		)
	}
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) consoleSemanticRuleCreate(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	spec, err := semanticRuleSpecFromForm(request)
	if err == nil {
		_, err = server.semanticConfig.CreateRule(
			request.Context(),
			server.actor(auth),
			request.PathValue("signal_ref"),
			request.FormValue("display_name"),
			spec,
			edgeapp.RevisionPrecondition{Expected: revision},
		)
	}
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(
			request,
			"/sensors/"+request.PathValue("signal_ref"),
		),
		err,
	)
}

func (server *Server) consoleSemanticRuleUpdate(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	spec, err := semanticRuleSpecFromForm(request)
	if err == nil {
		_, err = server.semanticConfig.UpdateRule(
			request.Context(),
			server.actor(auth),
			request.PathValue("rule_id"),
			request.FormValue("display_name"),
			spec,
			edgeapp.RevisionPrecondition{Expected: revision},
		)
	}
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/sensors"),
		err,
	)
}

func (server *Server) consoleSemanticRuleRetire(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, ok := requireConsoleRevision(response, request)
	if !ok {
		return
	}
	_, err := server.semanticConfig.RetireRule(
		request.Context(),
		server.actor(auth),
		request.PathValue("rule_id"),
		edgeapp.RevisionPrecondition{Expected: revision},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/sensors"),
		err,
	)
}

func (server *Server) consoleSemanticRuleCounterReset(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	resetID := request.FormValue("reset_id")
	if resetID == "" {
		resetID = "console_" + newRequestID()
	}
	_, err := server.semanticConfig.RequestCounterReset(
		request.Context(),
		server.actor(auth),
		request.PathValue("rule_id"),
		resetID,
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/sensors"),
		err,
	)
}

func semanticRuleSpecFromForm(request *http.Request) (semantics.RuleSpec, error) {
	riseThreshold, riseThresholdErr := strconv.ParseFloat(
		request.FormValue("rise_threshold"),
		64,
	)
	fallThreshold, fallThresholdErr := strconv.ParseFloat(
		request.FormValue("fall_threshold"),
		64,
	)
	riseDebounceSeconds, riseDebounceErr := strconv.ParseFloat(
		request.FormValue("rise_debounce_seconds"),
		64,
	)
	fallDebounceSeconds, fallDebounceErr := strconv.ParseFloat(
		request.FormValue("fall_debounce_seconds"),
		64,
	)
	if request.FormValue("rise_threshold") == "" {
		riseThreshold, riseThresholdErr = 0, nil
	}
	if request.FormValue("fall_threshold") == "" {
		fallThreshold, fallThresholdErr = 0, nil
	}
	if request.FormValue("rise_debounce_seconds") == "" {
		riseDebounceSeconds, riseDebounceErr = 0, nil
	}
	if request.FormValue("fall_debounce_seconds") == "" {
		fallDebounceSeconds, fallDebounceErr = 0, nil
	}
	if riseThresholdErr != nil || fallThresholdErr != nil ||
		riseDebounceErr != nil || fallDebounceErr != nil {
		return semantics.RuleSpec{}, errors.New("invalid semantic rule number")
	}
	spec := semantics.RuleSpec{
		Kind: semantics.Kind(request.FormValue("kind")),
		Detector: semantics.Detector{
			Mode:           semantics.DetectorMode(request.FormValue("detector_mode")),
			RiseThreshold:  riseThreshold,
			FallThreshold:  fallThreshold,
			RiseDebounceMS: int64(riseDebounceSeconds * 1000),
			FallDebounceMS: int64(fallDebounceSeconds * 1000),
		},
		Trigger: semantics.TriggerMode(request.FormValue("trigger")),
	}
	if err := spec.Validate(); err != nil {
		return semantics.RuleSpec{}, err
	}
	return spec, nil
}

func consoleObservationKind(kind semantics.Kind) outputadapter.ObservationKind {
	switch kind {
	case semantics.KindNumeric:
		return outputadapter.KindNumeric
	case semantics.KindBoolean:
		return outputadapter.KindBoolean
	case semantics.KindCumulativeCounter:
		return outputadapter.KindCumulativeValue
	case semantics.KindAlarm:
		return outputadapter.KindAlarm
	default:
		return ""
	}
}
