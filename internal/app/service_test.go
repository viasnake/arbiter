package app

import (
	"context"
	"reflect"
	"testing"
	"time"

	"github.com/viasnake/arbiter/internal/audit"
	"github.com/viasnake/arbiter/internal/authz"
	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/domain"
	"github.com/viasnake/arbiter/internal/planner"
	"github.com/viasnake/arbiter/internal/store"
)

type memoryAudit struct {
	records []audit.Record
}

func (m *memoryAudit) Append(r audit.Record) error {
	m.records = append(m.records, r)
	return nil
}

func (m *memoryAudit) Close() error { return nil }

type fixedProvider struct {
	d     authz.Decision
	calls int
}

func (f *fixedProvider) Authorize(_ context.Context, _ domain.Event) authz.Decision {
	f.calls++
	return f.d
}

func baseEvent(id string) domain.Event {
	return domain.Event{
		V:        0,
		EventID:  id,
		TenantID: "t1",
		Source:   "slack",
		RoomID:   "r1",
		Actor: domain.Actor{
			Type: "human",
			ID:   "u1",
		},
		Content: domain.EventContent{
			Type: "text",
			Text: "hello @arbiter",
		},
		TS: time.Now().UTC().Format(time.RFC3339),
	}
}

func newServiceForTest(t *testing.T, cfg config.Config, p authz.Provider) (*Service, *store.MemoryStore, *memoryAudit) {
	t.Helper()
	st := store.NewMemoryStore()
	al := &memoryAudit{}
	svc := NewService(cfg, st, p, planner.New(cfg.Planner), al)
	return svc, st, al
}

func TestProcessEventIdempotency(t *testing.T) {
	cfg := config.Default()
	cfg.Planner.ReplyPolicy = "all"

	svc, st, _ := newServiceForTest(t, cfg, &fixedProvider{d: authz.Decision{Allow: true, ReasonCode: "ok"}})
	ev := baseEvent("e1")

	p1, err := svc.ProcessEvent(context.Background(), ev)
	if err != nil {
		t.Fatalf("first process failed: %v", err)
	}
	p2, err := svc.ProcessEvent(context.Background(), ev)
	if err != nil {
		t.Fatalf("second process failed: %v", err)
	}
	if !reflect.DeepEqual(p1, p2) {
		t.Fatalf("plans must be equal for idempotency")
	}

	room := st.GetRoomState("t1", "r1")
	if room.PendingQueueSize != 1 {
		t.Fatalf("pending queue must remain 1, got %d", room.PendingQueueSize)
	}
}

func TestGateRunsBeforeAuthz(t *testing.T) {
	cfg := config.Default()
	cfg.Planner.ReplyPolicy = "all"

	provider := &fixedProvider{d: authz.Decision{Allow: true, ReasonCode: "ok"}}
	svc, _, _ := newServiceForTest(t, cfg, provider)

	_, err := svc.ProcessEvent(context.Background(), baseEvent("e1"))
	if err != nil {
		t.Fatalf("first event failed: %v", err)
	}

	p2, err := svc.ProcessEvent(context.Background(), baseEvent("e2"))
	if err != nil {
		t.Fatalf("second event failed: %v", err)
	}

	if got := p2.Actions[0].Type; got != domain.ActionDoNothing {
		t.Fatalf("second event should be blocked by gate, got %s", got)
	}
	if provider.calls != 1 {
		t.Fatalf("authz must not be called on gate rejection, calls=%d", provider.calls)
	}
}

func TestAuthzDenyReturnsDoNothing(t *testing.T) {
	cfg := config.Default()
	cfg.Planner.ReplyPolicy = "all"
	cfg.Gate.MaxQueue = 0

	svc, _, _ := newServiceForTest(t, cfg, &fixedProvider{d: authz.Decision{Allow: false, ReasonCode: "deny_by_policy"}})
	p, err := svc.ProcessEvent(context.Background(), baseEvent("e3"))
	if err != nil {
		t.Fatalf("process failed: %v", err)
	}
	if p.Actions[0].Type != domain.ActionDoNothing {
		t.Fatalf("expected do_nothing, got %s", p.Actions[0].Type)
	}
}

func TestGenerationResultProducesSendReply(t *testing.T) {
	cfg := config.Default()
	cfg.Planner.ReplyPolicy = "all"

	svc, _, _ := newServiceForTest(t, cfg, &fixedProvider{d: authz.Decision{Allow: true, ReasonCode: "ok"}})
	ev := baseEvent("e4")
	replyTo := "m-1"
	ev.Content.ReplyTo = &replyTo

	plan, err := svc.ProcessEvent(context.Background(), ev)
	if err != nil {
		t.Fatalf("process event failed: %v", err)
	}

	out, err := svc.ProcessGeneration(context.Background(), domain.GenerationResult{
		V:        0,
		PlanID:   plan.PlanID,
		ActionID: plan.Actions[0].ActionID,
		TenantID: "t1",
		Text:     "generated",
	})
	if err != nil {
		t.Fatalf("process generation failed: %v", err)
	}

	if out.Actions[0].Type != domain.ActionSendReply {
		t.Fatalf("expected send_reply, got %s", out.Actions[0].Type)
	}
}
