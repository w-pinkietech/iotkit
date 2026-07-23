package edgehttp

import "net/http"

func (server *Server) getStorageStatus(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	status, err := server.store.GetStorageStatus(
		request.Context(), server.storageWarningPercent,
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, status)
}

func (server *Server) getDiagnostics(response http.ResponseWriter, request *http.Request) {
	if _, ok := server.requireAPIAuth(response, request, false); !ok {
		return
	}
	report, err := server.store.GetDiagnostics(
		request.Context(), server.storageWarningPercent, server.now(),
	)
	if err != nil {
		server.operationError(response, err)
		return
	}
	writeJSON(response, http.StatusOK, report)
}
