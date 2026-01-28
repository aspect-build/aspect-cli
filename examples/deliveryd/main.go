package main

import (
	"context"
	"flag"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
)

func main() {
	socketPath := flag.String("socket", "/tmp/deliveryd.sock", "Unix socket path")
	redisEndpoint := flag.String("redis-endpoint", "redis://localhost:6379", "Redis endpoint URL (redis:// or rediss:// for TLS)")
	flag.Parse()

	redisClient, err := NewRedisClientFromEndpoint(*redisEndpoint)
	if err != nil {
		log.Fatalf("Failed to parse Redis endpoint: %v", err)
	}
	defer redisClient.Close()

	handler := NewHandler(redisClient)

	mux := http.NewServeMux()
	mux.HandleFunc("/query", handler.HandleQuery)
	mux.HandleFunc("/health", handler.HandleHealth)
	mux.HandleFunc("/record", handler.HandleRecord)
	mux.HandleFunc("/deliver", handler.HandleDeliver)
	mux.HandleFunc("/artifact/delete", handler.HandleDeleteArtifact)

	server, err := NewServer(*socketPath, mux)
	if err != nil {
		log.Fatalf("Failed to create server: %v", err)
	}

	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	go func() {
		<-sigChan
		log.Println("Shutting down...")
		if err := server.Shutdown(context.Background()); err != nil {
			log.Printf("Shutdown error: %v", err)
		}
	}()

	log.Printf("Listening on %s", *socketPath)
	if err := server.Serve(); err != nil && err != http.ErrServerClosed {
		log.Fatalf("Server error: %v", err)
	}
}
