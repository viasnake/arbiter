package authz

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"time"

	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/domain"
)

type Decision struct {
	Allow       bool
	ReasonCode  string
	PolicyVer   string
	DecisionSrc string
}

type Provider interface {
	Authorize(context.Context, domain.Event) Decision
}

type BuiltinAllowAll struct{}

func (b BuiltinAllowAll) Authorize(_ context.Context, _ domain.Event) Decision {
	return Decision{
		Allow:       true,
		ReasonCode:  "builtin_allow_all",
		PolicyVer:   "builtin-v0",
		DecisionSrc: "builtin",
	}
}

type ExternalHTTP struct {
	endpoint string
	timeout  time.Duration
	failMode string
	fallback Provider
	client   *http.Client
}

type requestPayload struct {
	V             int                    `json:"v"`
	TenantID      string                 `json:"tenant_id"`
	CorrelationID string                 `json:"correlation_id"`
	Actor         domain.Actor           `json:"actor"`
	Request       map[string]interface{} `json:"request"`
}

type responsePayload struct {
	V             int    `json:"v"`
	Decision      string `json:"decision"`
	ReasonCode    string `json:"reason_code"`
	PolicyVersion string `json:"policy_version"`
	TTLMS         int    `json:"ttl_ms"`
}

func NewProvider(cfg config.AuthzConfig) Provider {
	builtin := BuiltinAllowAll{}
	if cfg.Mode == "builtin" {
		return builtin
	}
	return &ExternalHTTP{
		endpoint: cfg.Endpoint,
		timeout:  time.Duration(cfg.TimeoutMS) * time.Millisecond,
		failMode: cfg.FailMode,
		fallback: builtin,
		client:   &http.Client{Timeout: time.Duration(cfg.TimeoutMS) * time.Millisecond},
	}
}

func (e *ExternalHTTP) Authorize(ctx context.Context, ev domain.Event) Decision {
	reqBody := requestPayload{
		V:             domain.ContractVersion,
		TenantID:      ev.TenantID,
		CorrelationID: ev.EventID,
		Actor:         ev.Actor,
		Request: map[string]interface{}{
			"action": "process_event",
			"resource": map[string]interface{}{
				"type": "room",
				"id":   ev.RoomID,
				"attributes": map[string]interface{}{
					"source": ev.Source,
				},
			},
			"context": map[string]interface{}{
				"event_id": ev.EventID,
			},
		},
	}

	b, err := json.Marshal(reqBody)
	if err != nil {
		return e.applyFailureMode(ctx)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, e.endpoint, bytes.NewReader(b))
	if err != nil {
		return e.applyFailureMode(ctx)
	}
	req.Header.Set("Content-Type", "application/json")

	resp, err := e.client.Do(req)
	if err != nil {
		return e.applyFailureMode(ctx)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return e.applyFailureMode(ctx)
	}

	var out responsePayload
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return e.applyFailureMode(ctx)
	}

	allow := out.Decision == "allow"
	reason := out.ReasonCode
	if reason == "" {
		if allow {
			reason = "authz_allow"
		} else {
			reason = "authz_deny"
		}
	}
	return Decision{
		Allow:       allow,
		ReasonCode:  reason,
		PolicyVer:   out.PolicyVersion,
		DecisionSrc: "external_http",
	}
}

func (e *ExternalHTTP) applyFailureMode(ctx context.Context) Decision {
	switch e.failMode {
	case "allow":
		return Decision{Allow: true, ReasonCode: "authz_error_allow", PolicyVer: "external-error", DecisionSrc: "external_http"}
	case "fallback_builtin":
		d := e.fallback.Authorize(ctx, domain.Event{})
		d.ReasonCode = "authz_error_fallback_builtin"
		return d
	case "deny":
		fallthrough
	default:
		return Decision{Allow: false, ReasonCode: "authz_error_deny", PolicyVer: "external-error", DecisionSrc: "external_http"}
	}
}

func (e *ExternalHTTP) String() string {
	return fmt.Sprintf("ExternalHTTP(endpoint=%s, timeout=%s)", e.endpoint, e.timeout)
}
