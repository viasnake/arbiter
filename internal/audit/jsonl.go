package audit

import (
	"encoding/json"
	"fmt"
	"os"
	"sync"
	"time"
)

type Record struct {
	AuditID       string `json:"audit_id"`
	TenantID      string `json:"tenant_id"`
	CorrelationID string `json:"correlation_id"`
	Action        string `json:"action"`
	Result        string `json:"result"`
	ReasonCode    string `json:"reason_code"`
	TS            string `json:"ts"`

	PlanID string `json:"plan_id,omitempty"`
}

type Logger interface {
	Append(record Record) error
	Close() error
}

type JSONLLogger struct {
	mu sync.Mutex
	f  *os.File
}

func NewJSONLLogger(path string) (*JSONLLogger, error) {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		return nil, fmt.Errorf("open audit file: %w", err)
	}
	return &JSONLLogger{f: f}, nil
}

func (l *JSONLLogger) Append(record Record) error {
	l.mu.Lock()
	defer l.mu.Unlock()

	if record.TS == "" {
		record.TS = time.Now().UTC().Format(time.RFC3339Nano)
	}
	b, err := json.Marshal(record)
	if err != nil {
		return fmt.Errorf("marshal audit record: %w", err)
	}
	if _, err := l.f.Write(append(b, '\n')); err != nil {
		return fmt.Errorf("write audit record: %w", err)
	}
	return nil
}

func (l *JSONLLogger) Close() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.f == nil {
		return nil
	}
	err := l.f.Close()
	l.f = nil
	return err
}
