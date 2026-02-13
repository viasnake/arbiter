package store

import (
	"sync"
	"time"

	"github.com/viasnake/arbiter/internal/domain"
)

type PendingGeneration struct {
	TenantID string
	RoomID   string
	PlanID   string
	ActionID string
	Kind     string
	ReplyTo  *string
}

type RoomState struct {
	Generating       bool
	PendingQueueSize int
	LastSendAt       time.Time
}

type MemoryStore struct {
	mu sync.Mutex

	idempotency map[string]domain.ResponsePlan
	rooms       map[string]*RoomState
	pending     map[string]PendingGeneration
	tenantRate  map[string]map[int64]int
}

func NewMemoryStore() *MemoryStore {
	return &MemoryStore{
		idempotency: make(map[string]domain.ResponsePlan),
		rooms:       make(map[string]*RoomState),
		pending:     make(map[string]PendingGeneration),
		tenantRate:  make(map[string]map[int64]int),
	}
}

func eventKey(tenantID, eventID string) string {
	return tenantID + ":" + eventID
}

func roomKey(tenantID, roomID string) string {
	return tenantID + ":" + roomID
}

func pendingKey(tenantID, actionID string) string {
	return tenantID + ":" + actionID
}

func (s *MemoryStore) GetIdempotency(tenantID, eventID string) (domain.ResponsePlan, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	p, ok := s.idempotency[eventKey(tenantID, eventID)]
	return p, ok
}

func (s *MemoryStore) PutIdempotency(tenantID, eventID string, plan domain.ResponsePlan) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.idempotency[eventKey(tenantID, eventID)] = plan
}

func (s *MemoryStore) GetRoomState(tenantID, roomID string) RoomState {
	s.mu.Lock()
	defer s.mu.Unlock()
	key := roomKey(tenantID, roomID)
	room, ok := s.rooms[key]
	if !ok {
		room = &RoomState{}
		s.rooms[key] = room
	}
	return *room
}

func (s *MemoryStore) PutPendingGeneration(p PendingGeneration) {
	s.mu.Lock()
	defer s.mu.Unlock()

	rKey := roomKey(p.TenantID, p.RoomID)
	room, ok := s.rooms[rKey]
	if !ok {
		room = &RoomState{}
		s.rooms[rKey] = room
	}
	room.Generating = true
	room.PendingQueueSize++

	s.pending[pendingKey(p.TenantID, p.ActionID)] = p
}

func (s *MemoryStore) ConsumePendingGeneration(tenantID, actionID string, at time.Time) (PendingGeneration, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()

	key := pendingKey(tenantID, actionID)
	p, ok := s.pending[key]
	if !ok {
		return PendingGeneration{}, false
	}
	delete(s.pending, key)

	rKey := roomKey(tenantID, p.RoomID)
	room, ok := s.rooms[rKey]
	if !ok {
		room = &RoomState{}
		s.rooms[rKey] = room
	}
	if room.PendingQueueSize > 0 {
		room.PendingQueueSize--
	}
	if room.PendingQueueSize == 0 {
		room.Generating = false
	}
	room.LastSendAt = at

	return p, true
}

func (s *MemoryStore) TenantRateCount(tenantID string, minuteBucket int64) int {
	s.mu.Lock()
	defer s.mu.Unlock()

	byMin, ok := s.tenantRate[tenantID]
	if !ok {
		return 0
	}
	return byMin[minuteBucket]
}

func (s *MemoryStore) IncrementTenantRate(tenantID string, minuteBucket int64) {
	s.mu.Lock()
	defer s.mu.Unlock()

	byMin, ok := s.tenantRate[tenantID]
	if !ok {
		byMin = make(map[int64]int)
		s.tenantRate[tenantID] = byMin
	}
	byMin[minuteBucket]++

	// best-effort cleanup of old buckets
	for bucket := range byMin {
		if bucket < minuteBucket-5 {
			delete(byMin, bucket)
		}
	}
}
