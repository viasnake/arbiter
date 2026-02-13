package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/viasnake/arbiter/internal/app"
	"github.com/viasnake/arbiter/internal/audit"
	"github.com/viasnake/arbiter/internal/authz"
	"github.com/viasnake/arbiter/internal/config"
	"github.com/viasnake/arbiter/internal/httpapi"
	"github.com/viasnake/arbiter/internal/planner"
	"github.com/viasnake/arbiter/internal/store"
)

func main() {
	if len(os.Args) < 2 {
		printUsage()
		os.Exit(2)
	}

	switch os.Args[1] {
	case "serve":
		if err := runServe(os.Args[2:]); err != nil {
			log.Fatalf("serve failed: %v", err)
		}
	default:
		printUsage()
		os.Exit(2)
	}
}

func runServe(args []string) error {
	fs := flag.NewFlagSet("serve", flag.ContinueOnError)
	cfgPath := fs.String("config", "./config/example-config.yaml", "path to config file")
	if err := fs.Parse(args); err != nil {
		return err
	}

	cfg, err := config.Load(*cfgPath)
	if err != nil {
		return fmt.Errorf("load config: %w", err)
	}

	st := store.NewMemoryStore()
	az := authz.NewProvider(cfg.Authz)
	pl := planner.New(cfg.Planner)

	auditor, err := audit.NewJSONLLogger(cfg.Audit.JSONLPath)
	if err != nil {
		return fmt.Errorf("init audit logger: %w", err)
	}
	defer auditor.Close()

	svc := app.NewService(cfg, st, az, pl, auditor)
	server := httpapi.NewServer(svc)

	httpServer := &http.Server{
		Addr:              cfg.Server.ListenAddr,
		Handler:           server.Handler(),
		ReadHeaderTimeout: 5 * time.Second,
	}

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)

	go func() {
		<-stop
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = httpServer.Shutdown(ctx)
	}()

	log.Printf("arbiter listening on %s", cfg.Server.ListenAddr)
	if err := httpServer.ListenAndServe(); err != nil && err != http.ErrServerClosed {
		return err
	}
	return nil
}

func printUsage() {
	fmt.Fprintln(os.Stderr, "Usage: arbiter serve --config ./config/example-config.yaml")
}
