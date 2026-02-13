package planner

import (
	"testing"

	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/domain"
)

func TestDeterministicByEventID(t *testing.T) {
	eng := New(config.PlannerConfig{ReplyPolicy: "probabilistic", ReplyProbability: 0.5})

	ev := domain.Event{EventID: "same-id", Content: domain.EventContent{Type: "text"}}
	a := eng.Decide(ev)
	b := eng.Decide(ev)
	if a != b {
		t.Fatalf("planner decision must be deterministic for same event_id")
	}
}

func TestMentionFirstPrefersReply(t *testing.T) {
	eng := New(config.PlannerConfig{ReplyPolicy: "mention_first", ReplyProbability: 0.0})

	ev := domain.Event{EventID: "e1", Content: domain.EventContent{Type: "text", Text: "hello @arbiter"}}
	if got := eng.Decide(ev); got != IntentReply {
		t.Fatalf("expected REPLY, got %s", got)
	}
}
