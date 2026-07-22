package edgehttp

import (
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"net/http"
	"strconv"
	"strings"
)

func (server *Server) consoleDeviceProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	expected := formRevision(request)
	_, err := server.edge.Dispatch(request.Context(), server.actor(auth), edgeapp.UpdateDeviceProfile{
		DeviceRef: request.PathValue("device_ref"),
		Input: edgeapp.DeviceProfileInput{
			DisplayName: request.FormValue("display_name"),
			Location:    request.FormValue("location"),
		},
		Precondition: edgeapp.RevisionPrecondition{Expected: expected},
	})
	server.consoleMutationResult(response, request, consoleReturnTarget(request, "/devices"), err)
}

func (server *Server) consoleEdgeNodeActivation(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	_, err := server.edge.Dispatch(
		request.Context(),
		server.actor(auth),
		edgeapp.ActivateEdgeNode{
			EdgeNodeRef: request.PathValue("edge_node_ref"),
			Precondition: edgeapp.RevisionPrecondition{
				Expected: formRevision(request),
			},
		},
	)
	server.consoleMutationResult(
		response,
		request,
		consoleReturnTarget(request, "/edge-nodes"),
		err,
	)
}

func (server *Server) consoleSignalProfile(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	displayUnitMode := request.FormValue("display_unit_mode")
	displayUnit := request.FormValue("display_unit")
	if displayUnitMode == "dimensionless" {
		displayUnit = ""
	}
	_, err := server.edge.Dispatch(request.Context(), server.actor(auth), edgeapp.UpdateSignalProfile{
		SignalRef: request.PathValue("signal_ref"),
		Input: edgeapp.SignalProfileInput{
			DisplayName:            request.FormValue("display_name"),
			DisplaySensorType:      request.FormValue("display_sensor_type"),
			DisplaySensorTypeLabel: request.FormValue("display_sensor_type_label"),
			DisplayValueKind:       request.FormValue("display_value_kind"),
			DisplayUnitMode:        displayUnitMode,
			DisplayUnit:            displayUnit,
			DecimalPlaces:          formInt(request, "decimal_places"),
		},
		Precondition: edgeapp.RevisionPrecondition{Expected: formRevision(request)},
	})
	server.consoleMutationResult(response, request, consoleReturnTarget(request, "/signals"), err)
}

func consoleReturnTarget(request *http.Request, fallback string) string {
	target := request.FormValue("return_to")
	if safeSensorSettingsReturnTarget(target) {
		return target
	}
	if safeEquipmentReturnTarget(target) {
		return target
	}
	if safeSensorReturnTarget(target) {
		return target
	}
	switch target {
	case "/equipment":
		return "/equipment"
	case "/setup":
		return "/setup"
	case "/signals":
		return "/signals"
	case "/devices":
		return "/devices"
	default:
		return fallback
	}
}

func safeSensorSettingsReturnTarget(target string) bool {
	if !strings.HasPrefix(target, "/") ||
		strings.ContainsAny(target, "?#\\") ||
		strings.HasPrefix(target, "//") {
		return false
	}
	parts := strings.Split(strings.TrimPrefix(target, "/"), "/")
	return len(parts) == 5 &&
		parts[0] == "equipment" &&
		parts[1] == "devices" &&
		validConsoleResourceRef(parts[2], "dev_") &&
		parts[3] == "sensors" &&
		validConsoleResourceRef(parts[4], "sig_")
}

func safeSensorReturnTarget(target string) bool {
	if !strings.HasPrefix(target, "/") ||
		strings.ContainsAny(target, "?#\\") ||
		strings.HasPrefix(target, "//") {
		return false
	}
	parts := strings.Split(strings.TrimPrefix(target, "/"), "/")
	return len(parts) == 2 &&
		parts[0] == "sensors" &&
		validConsoleResourceRef(parts[1], "sig_")
}

func safeEquipmentReturnTarget(target string) bool {
	if target == "/equipment" {
		return true
	}
	if !strings.HasPrefix(target, "/") ||
		strings.ContainsAny(target, "?#\\") ||
		strings.HasPrefix(target, "//") {
		return false
	}
	parts := strings.Split(strings.TrimPrefix(target, "/"), "/")
	if len(parts) != 3 || parts[0] != "equipment" || parts[2] == "" {
		return false
	}
	switch parts[1] {
	case "edge-nodes":
		return validConsoleResourceRef(parts[2], "edge_node_")
	case "devices":
		return validConsoleResourceRef(parts[2], "dev_")
	default:
		return false
	}
}

func validConsoleResourceRef(value, prefix string) bool {
	if len(value) != len(prefix)+32 || !strings.HasPrefix(value, prefix) {
		return false
	}
	for _, character := range value[len(prefix):] {
		if (character < '0' || character > '9') &&
			(character < 'a' || character > 'f') {
			return false
		}
	}
	return true
}

func formInt(request *http.Request, name string) int {
	value, _ := strconv.Atoi(request.FormValue(name))
	return value
}
