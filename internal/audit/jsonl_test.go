package audit

import (
	"os"
	"strings"
	"testing"
)

func TestJSONLLoggerAppendOnly(t *testing.T) {
	f, err := os.CreateTemp(t.TempDir(), "audit-*.jsonl")
	if err != nil {
		t.Fatalf("create temp file: %v", err)
	}
	_ = f.Close()

	logger, err := NewJSONLLogger(f.Name())
	if err != nil {
		t.Fatalf("new logger: %v", err)
	}
	defer logger.Close()

	if err := logger.Append(Record{AuditID: "a1", TenantID: "t1", CorrelationID: "c1", Action: "x", Result: "ok", ReasonCode: "r"}); err != nil {
		t.Fatalf("append first: %v", err)
	}
	if err := logger.Append(Record{AuditID: "a2", TenantID: "t1", CorrelationID: "c2", Action: "y", Result: "ok", ReasonCode: "r"}); err != nil {
		t.Fatalf("append second: %v", err)
	}

	b, err := os.ReadFile(f.Name())
	if err != nil {
		t.Fatalf("read file: %v", err)
	}
	lines := strings.Split(strings.TrimSpace(string(b)), "\n")
	if len(lines) != 2 {
		t.Fatalf("expected 2 lines, got %d", len(lines))
	}
}
