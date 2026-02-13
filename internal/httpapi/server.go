package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"time"

	"github.com/viasnake/arbiter/internal/app"
	"github.com/viasnake/arbiter/internal/domain"
)

type Server struct {
	svc *app.Service
}

func NewServer(svc *app.Service) *Server {
	return &Server{svc: svc}
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/v0/healthz", s.handleHealthz)
	mux.HandleFunc("/v0/contracts", s.handleContracts)
	mux.HandleFunc("/v0/events", s.handleEvents)
	mux.HandleFunc("/v0/generations", s.handleGenerations)
	mux.HandleFunc("/v0/action-results", s.handleActionResults)
	return mux
}

func (s *Server) handleHealthz(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeMethodNotAllowed(w)
		return
	}
	w.Header().Set("Content-Type", "text/plain")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte("ok"))
}

func (s *Server) handleContracts(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeMethodNotAllowed(w)
		return
	}
	out := map[string]interface{}{
		"version": "0.0.1",
		"actions": map[string]interface{}{
			"enabled":  []string{domain.ActionDoNothing, domain.ActionRequestGeneration, domain.ActionSendMessage, domain.ActionSendReply},
			"reserved": []string{"start_agent_job", "request_approval"},
		},
		"intents": []string{"IGNORE", "REPLY", "MESSAGE"},
	}
	writeJSON(w, http.StatusOK, out)
}

func (s *Server) handleEvents(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeMethodNotAllowed(w)
		return
	}

	var ev domain.Event
	if err := decodeStrictJSON(r, &ev); err != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_request", err.Error())
		return
	}

	plan, err := s.svc.ProcessEvent(r.Context(), ev)
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "validation_error", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, plan)
}

func (s *Server) handleGenerations(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeMethodNotAllowed(w)
		return
	}

	var in domain.GenerationResult
	if err := decodeStrictJSON(r, &in); err != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_request", err.Error())
		return
	}

	plan, err := s.svc.ProcessGeneration(r.Context(), in)
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "validation_error", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, plan)
}

func (s *Server) handleActionResults(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeMethodNotAllowed(w)
		return
	}

	var body map[string]interface{}
	if err := decodeStrictJSON(r, &body); err != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_request", err.Error())
		return
	}

	tenantID, _ := body["tenant_id"].(string)
	correlationID, _ := body["correlation_id"].(string)
	reason, _ := body["reason_code"].(string)
	if err := s.svc.RecordActionResult(tenantID, correlationID, reason); err != nil {
		writeJSONError(w, http.StatusBadRequest, "validation_error", err.Error())
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func decodeStrictJSON(r *http.Request, out interface{}) error {
	defer r.Body.Close()
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(out); err != nil {
		return err
	}
	if dec.More() {
		return errors.New("multiple JSON values are not allowed")
	}

	ctx, cancel := context.WithTimeout(r.Context(), 3*time.Second)
	defer cancel()
	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
	}
	return nil
}

func writeMethodNotAllowed(w http.ResponseWriter) {
	writeJSONError(w, http.StatusMethodNotAllowed, "method_not_allowed", "method not allowed")
}

func writeJSONError(w http.ResponseWriter, status int, code, message string) {
	writeJSON(w, status, map[string]interface{}{
		"error": map[string]interface{}{
			"code":    code,
			"message": message,
		},
	})
}

func writeJSON(w http.ResponseWriter, status int, v interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
