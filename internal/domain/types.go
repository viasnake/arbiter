package domain

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"time"
)

const ContractVersion = 0

const (
	ActionDoNothing         = "do_nothing"
	ActionRequestGeneration = "request_generation"
	ActionSendMessage       = "send_message"
	ActionSendReply         = "send_reply"
)

type Actor struct {
	Type   string                 `json:"type"`
	ID     string                 `json:"id"`
	Roles  []string               `json:"roles,omitempty"`
	Claims map[string]interface{} `json:"claims,omitempty"`
}

type EventContent struct {
	Type    string  `json:"type"`
	Text    string  `json:"text,omitempty"`
	ReplyTo *string `json:"reply_to"`
}

type Event struct {
	V          int                    `json:"v"`
	EventID    string                 `json:"event_id"`
	TenantID   string                 `json:"tenant_id"`
	Source     string                 `json:"source"`
	RoomID     string                 `json:"room_id"`
	Actor      Actor                  `json:"actor"`
	Content    EventContent           `json:"content"`
	TS         string                 `json:"ts"`
	Extensions map[string]interface{} `json:"extensions,omitempty"`
}

func (e Event) Validate() error {
	if e.V != ContractVersion {
		return fmt.Errorf("v must be %d", ContractVersion)
	}
	if strings.TrimSpace(e.EventID) == "" {
		return errors.New("event_id is required")
	}
	if strings.TrimSpace(e.TenantID) == "" {
		return errors.New("tenant_id is required")
	}
	if strings.TrimSpace(e.Source) == "" {
		return errors.New("source is required")
	}
	if strings.TrimSpace(e.RoomID) == "" {
		return errors.New("room_id is required")
	}
	if strings.TrimSpace(e.Actor.ID) == "" {
		return errors.New("actor.id is required")
	}
	switch e.Actor.Type {
	case "human", "service", "system":
	default:
		return errors.New("actor.type is invalid")
	}
	if e.Content.Type != "text" {
		return errors.New("content.type must be text")
	}
	if _, err := time.Parse(time.RFC3339, e.TS); err != nil {
		return fmt.Errorf("ts must be RFC3339: %w", err)
	}
	return nil
}

func (e Event) ParsedTS() time.Time {
	t, err := time.Parse(time.RFC3339, e.TS)
	if err != nil {
		return time.Time{}
	}
	return t
}

type Action struct {
	Type     string                 `json:"type"`
	ActionID string                 `json:"action_id"`
	Target   map[string]interface{} `json:"target,omitempty"`
	Payload  map[string]interface{} `json:"payload,omitempty"`
}

type ResponsePlan struct {
	V               int                    `json:"v"`
	PlanID          string                 `json:"plan_id"`
	TenantID        string                 `json:"tenant_id"`
	RoomID          string                 `json:"room_id"`
	Actions         []Action               `json:"actions"`
	PolicyDecisions []PolicyDecision       `json:"policy_decisions,omitempty"`
	Debug           map[string]interface{} `json:"debug,omitempty"`
}

type PolicyDecision struct {
	Stage      string `json:"stage"`
	Result     string `json:"result"`
	ReasonCode string `json:"reason_code,omitempty"`
}

type GenerationResult struct {
	V        int     `json:"v"`
	PlanID   string  `json:"plan_id"`
	ActionID string  `json:"action_id"`
	TenantID string  `json:"tenant_id"`
	Text     string  `json:"text"`
	TraceID  *string `json:"trace_id"`
}

func (g GenerationResult) Validate() error {
	if g.V != ContractVersion {
		return fmt.Errorf("v must be %d", ContractVersion)
	}
	if strings.TrimSpace(g.PlanID) == "" {
		return errors.New("plan_id is required")
	}
	if strings.TrimSpace(g.ActionID) == "" {
		return errors.New("action_id is required")
	}
	if strings.TrimSpace(g.TenantID) == "" {
		return errors.New("tenant_id is required")
	}
	return nil
}

func NewPlanID(tenantID, eventID string) string {
	sum := sha256.Sum256([]byte(tenantID + ":" + eventID))
	return "plan_" + hex.EncodeToString(sum[:8])
}

func NewActionID(planID, actionType string, index int) string {
	sum := sha256.Sum256([]byte(fmt.Sprintf("%s:%s:%d", planID, actionType, index)))
	return "act_" + hex.EncodeToString(sum[:8])
}

func DoNothingPlan(tenantID, roomID, eventID, reasonCode string) ResponsePlan {
	planID := NewPlanID(tenantID, eventID)
	action := Action{
		Type:     ActionDoNothing,
		ActionID: NewActionID(planID, ActionDoNothing, 0),
		Payload: map[string]interface{}{
			"reason_code": reasonCode,
		},
	}
	return ResponsePlan{
		V:        ContractVersion,
		PlanID:   planID,
		TenantID: tenantID,
		RoomID:   roomID,
		Actions:  []Action{action},
	}
}
