package edgehttp

import (
	"errors"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"net/http"
	"net/url"
	"strconv"
)

func (server *Server) consoleAccount(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), edgeapp.CreateAccount{
			LoginID:           request.FormValue("login_id"),
			DisplayName:       request.FormValue("display_name"),
			Role:              edgeapp.AccountRole(request.FormValue("role")),
			TemporaryPassword: request.FormValue("temporary_password"),
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) consoleAccountUpdate(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	revision := formRevision(request)
	if revision == nil {
		http.Error(response, "画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed)
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), edgeapp.UpdateAccount{
			AccountRef:       request.PathValue("account_ref"),
			DisplayName:      request.FormValue("display_name"),
			Role:             edgeapp.AccountRole(request.FormValue("role")),
			ExpectedRevision: *revision,
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) consoleAccountDisable(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	revision := formRevision(request)
	if revision == nil {
		http.Error(response, "画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed)
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), edgeapp.DisableAccount{
			AccountRef:       request.PathValue("account_ref"),
			ExpectedRevision: *revision,
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) consoleAccountPassword(response http.ResponseWriter, request *http.Request) {
	auth, ok := server.requireBrowserOwnerMutation(response, request)
	if !ok {
		return
	}
	revision := formRevision(request)
	if revision == nil {
		http.Error(response, "画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed)
		return
	}
	_, err := server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), edgeapp.ResetAccountPassword{
			AccountRef:        request.PathValue("account_ref"),
			TemporaryPassword: request.FormValue("temporary_password"),
			ExpectedRevision:  *revision,
		},
	)
	server.consoleMutationResult(response, request, "/accounts", err)
}

func (server *Server) requireBrowserOwnerMutation(
	response http.ResponseWriter,
	request *http.Request,
) (requestAuth, bool) {
	auth, ok := server.requireBrowserAuth(response, request)
	if !ok {
		return requestAuth{}, false
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return requestAuth{}, false
	}
	if auth.account.Role != edgeapp.AccountRoleSystemAdmin {
		http.Error(response, "この操作を行う権限がありません。", http.StatusForbidden)
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) requireBrowserMutation(
	response http.ResponseWriter,
	request *http.Request,
	adminOnly bool,
) (requestAuth, bool) {
	auth, ok := server.requireBrowserAuth(response, request)
	if !ok {
		return requestAuth{}, false
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return requestAuth{}, false
	}
	if adminOnly && auth.account.Role == edgeapp.AccountRoleViewer {
		http.Error(response, "この操作を行う権限がありません。", http.StatusForbidden)
		return requestAuth{}, false
	}
	return auth, true
}

func (server *Server) consoleMutationResult(
	response http.ResponseWriter,
	request *http.Request,
	target string,
	err error,
) {
	parsed, parseErr := url.Parse(target)
	if parseErr != nil {
		parsed = &url.URL{Path: "/status"}
	}
	query := parsed.Query()
	anchor := request.FormValue("return_anchor")
	if safeConsoleAnchor(anchor) {
		query.Set("focus", anchor)
		parsed.Fragment = anchor
	}
	if err != nil {
		query.Set("error", consoleErrorCode(err))
		parsed.RawQuery = query.Encode()
		http.Redirect(response, request, parsed.String(), http.StatusSeeOther)
		return
	}
	query.Set("saved", "1")
	parsed.RawQuery = query.Encode()
	http.Redirect(response, request, parsed.String(), http.StatusSeeOther)
}

func safeConsoleAnchor(anchor string) bool {
	if anchor == "" || len(anchor) > 128 {
		return false
	}
	for _, character := range anchor {
		if (character < 'a' || character > 'z') &&
			(character < 'A' || character > 'Z') &&
			(character < '0' || character > '9') &&
			character != '-' && character != '_' {
			return false
		}
	}
	return true
}

func consoleErrorCode(err error) string {
	if errors.Is(err, edgeapp.ErrRevisionMismatch) {
		return "revision_mismatch"
	}
	switch err.Error() {
	case "semantic falling threshold cannot exceed rising threshold":
		return "threshold_order"
	case "invalid semantic rule number",
		"semantic detector thresholds must be finite":
		return "rule_number"
	case "semantic detector debounce must be between 0 and 300000 milliseconds":
		return "debounce_range"
	case "semantic calibration scale must be a finite non-zero number":
		return "calibration_scale"
	case "semantic calibration offset must be finite":
		return "calibration_offset"
	case "semantic rule display name must be 1 to 128 characters without surrounding whitespace":
		return "rule_name"
	default:
		return "save"
	}
}

func consoleErrorMessage(code string) string {
	switch code {
	case "revision_mismatch":
		return "別の担当者が先に設定を変更しました。最新の設定を確認して、もう一度変更してください。"
	case "threshold_order":
		return "保存できませんでした。立ち下がりしきい値は、立ち上がりしきい値以下にしてください。"
	case "rule_number":
		return "保存できませんでした。しきい値と確定待ち時間には数値を入力してください。"
	case "debounce_range":
		return "保存できませんでした。確定待ち時間は0秒から300秒の範囲で入力してください。"
	case "calibration_scale":
		return "保存できませんでした。補正倍率には0以外の数値を入力してください。"
	case "calibration_offset":
		return "保存できませんでした。補正加算には数値を入力してください。"
	case "rule_name":
		return "保存できませんでした。ルール名は前後の空白を除き、1文字から128文字で入力してください。"
	default:
		return "保存できませんでした。入力内容を確認し、もう一度お試しください。"
	}
}

func formRevision(request *http.Request) *int64 {
	raw := request.FormValue("revision")
	if raw == "" {
		return nil
	}
	value, err := strconv.ParseInt(raw, 10, 64)
	if err != nil {
		value = -1
	}
	return &value
}

func requireConsoleRevision(
	response http.ResponseWriter,
	request *http.Request,
) (*int64, bool) {
	revision := formRevision(request)
	if revision == nil {
		http.Error(
			response,
			"画面を再読み込みして、もう一度操作してください。",
			http.StatusPreconditionFailed,
		)
		return nil, false
	}
	return revision, true
}

func (server *Server) passwordPage(response http.ResponseWriter, request *http.Request) {
	_, err := server.authenticate(request)
	if err != nil {
		http.Redirect(response, request, "/login", http.StatusSeeOther)
		return
	}
	csrf := ""
	if cookie, err := request.Cookie(csrfCookieName); err == nil {
		csrf = cookie.Value
	}
	response.Header().Set("Content-Type", "text/html; charset=utf-8")
	_ = server.templates.ExecuteTemplate(response, "password.html", struct {
		CSRF  string
		Error string
	}{CSRF: csrf, Error: ""})
}

func (server *Server) passwordForm(response http.ResponseWriter, request *http.Request) {
	auth, err := server.authenticate(request)
	if err != nil {
		http.Redirect(response, request, "/login", http.StatusSeeOther)
		return
	}
	if !server.authorizeMutation(response, request, auth.token) {
		return
	}
	_, err = server.accounts.DispatchAccount(
		request.Context(), server.actor(auth), edgeapp.ChangeOwnPassword{
			CurrentPassword: request.FormValue("current_password"),
			NewPassword:     request.FormValue("new_password"),
		},
	)
	if err != nil {
		http.Error(response, "パスワードを変更できませんでした。", http.StatusBadRequest)
		return
	}
	_ = server.sessions.Logout(request.Context(), auth.token)
	server.clearSessionCookies(response)
	http.Redirect(response, request, "/login", http.StatusSeeOther)
}
