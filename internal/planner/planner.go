package planner

import (
	"hash/fnv"
	"strings"

	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/domain"
)

type Intent string

const (
	IntentIgnore  Intent = "IGNORE"
	IntentReply   Intent = "REPLY"
	IntentMessage Intent = "MESSAGE"
)

type Engine struct {
	cfg config.PlannerConfig
}

func New(cfg config.PlannerConfig) *Engine {
	return &Engine{cfg: cfg}
}

func (e *Engine) Decide(ev domain.Event) Intent {
	if ev.Content.ReplyTo != nil && strings.TrimSpace(*ev.Content.ReplyTo) != "" {
		return IntentReply
	}

	mentioned := isMentioned(ev.Content.Text)
	switch e.cfg.ReplyPolicy {
	case "all":
		return IntentMessage
	case "reply_only":
		if mentioned {
			return IntentReply
		}
		return IntentIgnore
	case "mention_first":
		if mentioned {
			return IntentReply
		}
		if seededProbability(ev.EventID) < e.cfg.ReplyProbability {
			return IntentMessage
		}
		return IntentIgnore
	case "probabilistic":
		if seededProbability(ev.EventID) < e.cfg.ReplyProbability {
			return IntentMessage
		}
		return IntentIgnore
	default:
		return IntentIgnore
	}
}

func seededProbability(eventID string) float64 {
	h := fnv.New64a()
	_, _ = h.Write([]byte(eventID))
	v := h.Sum64()
	return float64(v%10000) / 10000.0
}

func isMentioned(text string) bool {
	lower := strings.ToLower(text)
	return strings.Contains(lower, "@arbiter")
}
