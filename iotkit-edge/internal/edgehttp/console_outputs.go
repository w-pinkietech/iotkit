package edgehttp

import (
	"errors"
	"net/http"
	"strconv"
)

func (server *Server) consoleActivateExportProfile(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	var err error
	if request.FormValue("auto_bind_future_rules") != "true" {
		err = errors.New("future rule authorization is required")
	} else {
		_, err = server.store.ActivateExportProfile(
			request.Context(),
			server.actor(auth),
			request.FormValue("display_name"),
			request.FormValue("adapter_id"),
		)
	}
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleConfigureOutputBinding(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, err := strconv.ParseInt(request.FormValue("revision"), 10, 64)
	if err == nil {
		_, err = server.store.ConfigureYokaKitBooleanBinding(
			request.Context(),
			server.actor(auth),
			request.PathValue("binding_id"),
			request.FormValue("mode"),
			revision,
		)
	}
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleStopExportProfile(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	revision, err := strconv.ParseInt(request.FormValue("revision"), 10, 64)
	if err == nil {
		_, err = server.store.RequestExportProfileStop(
			request.Context(),
			server.actor(auth),
			request.PathValue("profile_id"),
			revision,
		)
	}
	server.consoleMutationResult(response, request, "/output", err)
}

func (server *Server) consoleStartOutputBinding(
	response http.ResponseWriter,
	request *http.Request,
) {
	auth, ok := server.requireBrowserMutation(response, request, true)
	if !ok {
		return
	}
	var err error
	if request.FormValue("external_registration_complete") != "true" {
		err = errors.New("external topic registration confirmation is required")
	} else {
		revision, parseErr := strconv.ParseInt(
			request.FormValue("revision"), 10, 64,
		)
		if parseErr != nil {
			err = parseErr
		} else {
			_, err = server.store.StartPreparedOutputBinding(
				request.Context(),
				server.actor(auth),
				request.PathValue("binding_id"),
				revision,
			)
		}
	}
	server.consoleMutationResult(response, request, "/output", err)
}
