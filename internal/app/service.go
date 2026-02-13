package app

import (
	"context"
	"errors"
	"time"

	"github.com/viasnake/arbiter/internal/audit"
	"github.com/viasnake/arbiter/internal/authz"
	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/domain"
	"github.com/viasnake/arbiter/internal/gate"
	"github.com/viasnake/arbiter/internal/planner"
	"github.com/viasnake/arbiter/internal/store"
)

type Service struct {
	cfg     config.Config
	store   *store.MemoryStore
	gate    *gate.Evaluator
	authz   authz.Provider
	planner *planner.Engine
	audit   audit.Logger
	nowFn   func() time.Time
}

func NewService(cfg config.Config, st *store.MemoryStore, az authz.Provider, pl *planner.Engine, al audit.Logger) *Service {
	return &Service{
		cfg:     cfg,
		store:   st,
		gate:    gate.NewEvaluator(cfg.Gate),
		authz:   az,
		planner: pl,
		audit:   al,
		nowFn:   func() time.Time { return time.Now().UTC() },
	}
}

func (s *Service) ProcessEvent(ctx context.Context, ev domain.Event) (domain.ResponsePlan, error) {
	if err := ev.Validate(); err != nil {
		return domain.ResponsePlan{}, err
	}

	if p, ok := s.store.GetIdempotency(ev.TenantID, ev.EventID); ok {
		_ = s.audit.Append(audit.Record{
			AuditID:       domain.NewActionID(p.PlanID, "audit", 0),
			TenantID:      ev.TenantID,
			CorrelationID: ev.EventID,
			Action:        "process_event",
			Result:        "idempotency_hit",
			ReasonCode:    "idempotency_hit",
			TS:            s.nowFn().Format(time.RFC3339Nano),
			PlanID:        p.PlanID,
		})
		return p, nil
	}

	eventTime := ev.ParsedTS()
	if eventTime.IsZero() {
		eventTime = s.nowFn()
	}

	room := s.store.GetRoomState(ev.TenantID, ev.RoomID)

	minuteBucket := eventTime.Unix() / 60
	tenantCount := s.store.TenantRateCount(ev.TenantID, minuteBucket)
	gateResult := s.gate.Evaluate(room, eventTime, tenantCount)
	if !gateResult.Allowed {
		plan := domain.DoNothingPlan(ev.TenantID, ev.RoomID, ev.EventID, gateResult.ReasonCode)
		s.store.PutIdempotency(ev.TenantID, ev.EventID, plan)
		_ = s.audit.Append(audit.Record{
			AuditID:       domain.NewActionID(plan.PlanID, "audit", 0),
			TenantID:      ev.TenantID,
			CorrelationID: ev.EventID,
			Action:        "gate",
			Result:        "deny",
			ReasonCode:    gateResult.ReasonCode,
			TS:            s.nowFn().Format(time.RFC3339Nano),
			PlanID:        plan.PlanID,
		})
		return plan, nil
	}

	authzDecision := s.authz.Authorize(ctx, ev)
	if !authzDecision.Allow {
		plan := domain.DoNothingPlan(ev.TenantID, ev.RoomID, ev.EventID, authzDecision.ReasonCode)
		s.store.PutIdempotency(ev.TenantID, ev.EventID, plan)
		_ = s.audit.Append(audit.Record{
			AuditID:       domain.NewActionID(plan.PlanID, "audit", 0),
			TenantID:      ev.TenantID,
			CorrelationID: ev.EventID,
			Action:        "authz",
			Result:        "deny",
			ReasonCode:    authzDecision.ReasonCode,
			TS:            s.nowFn().Format(time.RFC3339Nano),
			PlanID:        plan.PlanID,
		})
		return plan, nil
	}

	intent := s.planner.Decide(ev)
	var plan domain.ResponsePlan
	switch intent {
	case planner.IntentIgnore:
		plan = domain.DoNothingPlan(ev.TenantID, ev.RoomID, ev.EventID, "planner_ignore")
	case planner.IntentReply, planner.IntentMessage:
		plan = domain.ResponsePlan{
			V:        domain.ContractVersion,
			PlanID:   domain.NewPlanID(ev.TenantID, ev.EventID),
			TenantID: ev.TenantID,
			RoomID:   ev.RoomID,
			Actions: []domain.Action{
				{
					Type:     domain.ActionRequestGeneration,
					ActionID: domain.NewActionID(domain.NewPlanID(ev.TenantID, ev.EventID), domain.ActionRequestGeneration, 0),
					Target: map[string]interface{}{
						"room_id": ev.RoomID,
					},
					Payload: map[string]interface{}{
						"intent":   string(intent),
						"event_id": ev.EventID,
						"text":     ev.Content.Text,
					},
				},
			},
			PolicyDecisions: []domain.PolicyDecision{
				{Stage: "gate", Result: "allow"},
				{Stage: "authz", Result: "allow", ReasonCode: authzDecision.ReasonCode},
				{Stage: "planner", Result: "allow", ReasonCode: string(intent)},
			},
		}
		action := plan.Actions[0]
		s.store.PutPendingGeneration(store.PendingGeneration{
			TenantID: ev.TenantID,
			RoomID:   ev.RoomID,
			PlanID:   plan.PlanID,
			ActionID: action.ActionID,
			Kind:     string(intent),
			ReplyTo:  ev.Content.ReplyTo,
		})
	default:
		plan = domain.DoNothingPlan(ev.TenantID, ev.RoomID, ev.EventID, "planner_unknown")
	}

	s.store.IncrementTenantRate(ev.TenantID, minuteBucket)
	s.store.PutIdempotency(ev.TenantID, ev.EventID, plan)
	_ = s.audit.Append(audit.Record{
		AuditID:       domain.NewActionID(plan.PlanID, "audit", 0),
		TenantID:      ev.TenantID,
		CorrelationID: ev.EventID,
		Action:        "process_event",
		Result:        "ok",
		ReasonCode:    plan.Actions[0].Type,
		TS:            s.nowFn().Format(time.RFC3339Nano),
		PlanID:        plan.PlanID,
	})
	return plan, nil
}

func (s *Service) ProcessGeneration(_ context.Context, result domain.GenerationResult) (domain.ResponsePlan, error) {
	if err := result.Validate(); err != nil {
		return domain.ResponsePlan{}, err
	}

	pending, ok := s.store.ConsumePendingGeneration(result.TenantID, result.ActionID, s.nowFn())
	if !ok {
		plan := domain.DoNothingPlan(result.TenantID, "", result.ActionID, "generation_unknown_action")
		_ = s.audit.Append(audit.Record{
			AuditID:       domain.NewActionID(plan.PlanID, "audit", 0),
			TenantID:      result.TenantID,
			CorrelationID: result.ActionID,
			Action:        "generation_result",
			Result:        "no_pending_action",
			ReasonCode:    "generation_unknown_action",
			TS:            s.nowFn().Format(time.RFC3339Nano),
			PlanID:        plan.PlanID,
		})
		return plan, nil
	}

	actionType := domain.ActionSendMessage
	target := map[string]interface{}{"room_id": pending.RoomID}
	if pending.Kind == string(planner.IntentReply) || (pending.ReplyTo != nil && *pending.ReplyTo != "") {
		actionType = domain.ActionSendReply
		if pending.ReplyTo != nil {
			target["reply_to"] = *pending.ReplyTo
		}
	}

	planID := domain.NewPlanID(result.TenantID, result.ActionID)
	plan := domain.ResponsePlan{
		V:        domain.ContractVersion,
		PlanID:   planID,
		TenantID: result.TenantID,
		RoomID:   pending.RoomID,
		Actions: []domain.Action{
			{
				Type:     actionType,
				ActionID: domain.NewActionID(planID, actionType, 0),
				Target:   target,
				Payload: map[string]interface{}{
					"text":    result.Text,
					"plan_id": result.PlanID,
				},
			},
		},
	}

	_ = s.audit.Append(audit.Record{
		AuditID:       domain.NewActionID(plan.PlanID, "audit", 0),
		TenantID:      result.TenantID,
		CorrelationID: result.ActionID,
		Action:        "generation_result",
		Result:        "ok",
		ReasonCode:    actionType,
		TS:            s.nowFn().Format(time.RFC3339Nano),
		PlanID:        plan.PlanID,
	})
	return plan, nil
}

func (s *Service) RecordActionResult(tenantID, correlationID, reason string) error {
	if tenantID == "" || correlationID == "" {
		return errors.New("tenant_id and correlation_id are required")
	}
	return s.audit.Append(audit.Record{
		AuditID:       domain.NewActionID(correlationID, "audit", 0),
		TenantID:      tenantID,
		CorrelationID: correlationID,
		Action:        "action_result",
		Result:        "recorded",
		ReasonCode:    reason,
		TS:            s.nowFn().Format(time.RFC3339Nano),
	})
}
