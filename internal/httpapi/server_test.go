package httpapi

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/viasnake/arbiter/internal/app"
	"github.com/viasnake/arbiter/internal/audit"
	"github.com/viasnake/arbiter/internal/authz"
	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/planner"
	"github.com/viasnake/arbiter/internal/store"
)

func testServer(t *testing.T) http.Handler {
	t.Helper()
	cfg := config.Default()
	cfg.Planner.ReplyPolicy = "all"

	st := store.NewMemoryStore()
	az := authz.BuiltinAllowAll{}

	af, err := audit.NewJSONLLogger(t.TempDir() + "/audit.jsonl")
	if err != nil {
		t.Fatalf("new audit logger: %v", err)
	}
	t.Cleanup(func() { _ = af.Close() })

	svc := app.NewService(cfg, st, az, planner.New(cfg.Planner), af)
	return NewServer(svc).Handler()
}

func TestHealthz(t *testing.T) {
	h := testServer(t)
	req := httptest.NewRequest(http.MethodGet, "/v0/healthz", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("status must be 200, got %d", rec.Code)
	}
}

func TestEventsEndpoint(t *testing.T) {
	h := testServer(t)
	body := map[string]interface{}{
		"v":         0,
		"event_id":  "e-http-1",
		"tenant_id": "t1",
		"source":    "slack",
		"room_id":   "r1",
		"actor": map[string]interface{}{
			"type": "human",
			"id":   "u1",
		},
		"content": map[string]interface{}{
			"type": "text",
			"text": "hello @arbiter",
		},
		"ts": time.Now().UTC().Format(time.RFC3339),
	}
	b, _ := json.Marshal(body)
	req := httptest.NewRequest(http.MethodPost, "/v0/events", bytes.NewReader(b))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("status must be 200, got %d", rec.Code)
	}

	var out map[string]interface{}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatalf("decode response: %v", err)
	}
	if _, ok := out["plan_id"]; !ok {
		t.Fatalf("response must have plan_id")
	}
}
