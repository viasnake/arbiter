package gate

import (
	"time"

	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/store"
)

type Result struct {
	Allowed    bool
	ReasonCode string
}

type Evaluator struct {
	cfg config.GateConfig
}

func NewEvaluator(cfg config.GateConfig) *Evaluator {
	return &Evaluator{cfg: cfg}
}

func (e *Evaluator) Evaluate(room store.RoomState, eventTS time.Time, tenantCount int) Result {
	if room.Generating {
		return Result{Allowed: false, ReasonCode: "gate_generating_lock"}
	}

	if e.cfg.CooldownMS > 0 && !room.LastSendAt.IsZero() {
		cooldownUntil := room.LastSendAt.Add(time.Duration(e.cfg.CooldownMS) * time.Millisecond)
		if eventTS.Before(cooldownUntil) {
			return Result{Allowed: false, ReasonCode: "gate_cooldown"}
		}
	}

	if e.cfg.MaxQueue > 0 && room.PendingQueueSize >= e.cfg.MaxQueue {
		return Result{Allowed: false, ReasonCode: "gate_backpressure"}
	}

	if e.cfg.TenantRateLimitPerMin > 0 && tenantCount >= e.cfg.TenantRateLimitPerMin {
		return Result{Allowed: false, ReasonCode: "gate_tenant_rate_limit"}
	}

	return Result{Allowed: true}
}
