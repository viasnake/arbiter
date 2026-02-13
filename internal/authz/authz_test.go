package authz

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/domain"
)

func TestExternalHTTPDeny(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"v":0,"decision":"deny","reason_code":"policy_deny","policy_version":"p1","ttl_ms":1000}`))
	}))
	defer srv.Close()

	p := NewProvider(config.AuthzConfig{Mode: "external_http", Endpoint: srv.URL, TimeoutMS: 200, FailMode: "deny"})
	d := p.Authorize(context.Background(), domain.Event{TenantID: "t1", EventID: "e1", RoomID: "r1", Actor: domain.Actor{Type: "human", ID: "u1"}})
	if d.Allow {
		t.Fatalf("must deny")
	}
}

func TestExternalHTTPFailModeAllow(t *testing.T) {
	p := NewProvider(config.AuthzConfig{Mode: "external_http", Endpoint: "http://127.0.0.1:1", TimeoutMS: 10, FailMode: "allow"})
	d := p.Authorize(context.Background(), domain.Event{})
	if !d.Allow {
		t.Fatalf("must allow on fail_mode=allow")
	}
}
