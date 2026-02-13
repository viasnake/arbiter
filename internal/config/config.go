package config

import (
	"errors"
	"fmt"
	"os"

	"gopkg.in/yaml.v3"
)

type Config struct {
	Server  ServerConfig  `yaml:"server"`
	Store   StoreConfig   `yaml:"store"`
	Authz   AuthzConfig   `yaml:"authz"`
	Gate    GateConfig    `yaml:"gate"`
	Planner PlannerConfig `yaml:"planner"`
	Audit   AuditConfig   `yaml:"audit"`
}

type ServerConfig struct {
	ListenAddr string `yaml:"listen_addr"`
}

type StoreConfig struct {
	Type       string `yaml:"type"`
	SQLitePath string `yaml:"sqlite_path"`
}

type AuthzCacheConfig struct {
	Enabled    bool `yaml:"enabled"`
	TTLMS      int  `yaml:"ttl_ms"`
	MaxEntries int  `yaml:"max_entries"`
}

type AuthzConfig struct {
	Mode      string           `yaml:"mode"`
	Endpoint  string           `yaml:"endpoint"`
	TimeoutMS int              `yaml:"timeout_ms"`
	FailMode  string           `yaml:"fail_mode"`
	Cache     AuthzCacheConfig `yaml:"cache"`
}

type GateConfig struct {
	CooldownMS            int `yaml:"cooldown_ms"`
	MaxQueue              int `yaml:"max_queue"`
	TenantRateLimitPerMin int `yaml:"tenant_rate_limit_per_min"`
}

type PlannerConfig struct {
	ReplyPolicy      string  `yaml:"reply_policy"`
	ReplyProbability float64 `yaml:"reply_probability"`
}

type AuditConfig struct {
	Sink                 string `yaml:"sink"`
	JSONLPath            string `yaml:"jsonl_path"`
	IncludeAuthzDecision bool   `yaml:"include_authz_decision"`
}

func Default() Config {
	return Config{
		Server: ServerConfig{ListenAddr: "0.0.0.0:8080"},
		Store:  StoreConfig{Type: "memory"},
		Authz: AuthzConfig{
			Mode:      "builtin",
			TimeoutMS: 300,
			FailMode:  "deny",
			Cache: AuthzCacheConfig{
				Enabled:    true,
				TTLMS:      30000,
				MaxEntries: 100000,
			},
		},
		Gate: GateConfig{
			CooldownMS:            3000,
			MaxQueue:              10,
			TenantRateLimitPerMin: 0,
		},
		Planner: PlannerConfig{
			ReplyPolicy:      "mention_first",
			ReplyProbability: 0,
		},
		Audit: AuditConfig{
			Sink:                 "jsonl",
			JSONLPath:            "./arbiter-audit.jsonl",
			IncludeAuthzDecision: true,
		},
	}
}

func Load(path string) (Config, error) {
	cfg := Default()
	if path == "" {
		return cfg, errors.New("config path is required")
	}

	b, err := os.ReadFile(path)
	if err != nil {
		return Config{}, fmt.Errorf("read config: %w", err)
	}
	if err := yaml.Unmarshal(b, &cfg); err != nil {
		return Config{}, fmt.Errorf("parse config: %w", err)
	}
	if err := cfg.Validate(); err != nil {
		return Config{}, err
	}
	return cfg, nil
}

func (c Config) Validate() error {
	if c.Server.ListenAddr == "" {
		return errors.New("server.listen_addr is required")
	}

	switch c.Store.Type {
	case "memory", "sqlite":
	default:
		return errors.New("store.type must be memory or sqlite")
	}
	if c.Store.Type == "sqlite" && c.Store.SQLitePath == "" {
		return errors.New("store.sqlite_path is required for sqlite")
	}

	switch c.Authz.Mode {
	case "builtin", "external_http":
	default:
		return errors.New("authz.mode must be builtin or external_http")
	}
	if c.Authz.Mode == "external_http" && c.Authz.Endpoint == "" {
		return errors.New("authz.endpoint is required for external_http")
	}
	switch c.Authz.FailMode {
	case "deny", "allow", "fallback_builtin":
	default:
		return errors.New("authz.fail_mode must be deny, allow, fallback_builtin")
	}
	if c.Authz.TimeoutMS <= 0 {
		return errors.New("authz.timeout_ms must be > 0")
	}

	if c.Gate.CooldownMS < 0 {
		return errors.New("gate.cooldown_ms must be >= 0")
	}
	if c.Gate.MaxQueue < 0 {
		return errors.New("gate.max_queue must be >= 0")
	}
	if c.Gate.TenantRateLimitPerMin < 0 {
		return errors.New("gate.tenant_rate_limit_per_min must be >= 0")
	}

	switch c.Planner.ReplyPolicy {
	case "all", "mention_first", "probabilistic", "reply_only":
	default:
		return errors.New("planner.reply_policy is invalid")
	}
	if c.Planner.ReplyProbability < 0 || c.Planner.ReplyProbability > 1 {
		return errors.New("planner.reply_probability must be between 0 and 1")
	}

	if c.Audit.Sink != "jsonl" {
		return errors.New("audit.sink must be jsonl")
	}
	if c.Audit.JSONLPath == "" {
		return errors.New("audit.jsonl_path is required")
	}
	return nil
}
