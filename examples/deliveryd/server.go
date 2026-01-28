package main

import (
	"context"
	"net"
	"net/http"
	"os"
)

// Server wraps the HTTP server and Unix socket listener.
type Server struct {
	httpServer *http.Server
	listener   net.Listener
	socketPath string
}

// NewServer creates a new server that listens on a Unix socket.
func NewServer(socketPath string, handler http.Handler) (*Server, error) {
	// Remove existing socket file if it exists
	if err := os.Remove(socketPath); err != nil && !os.IsNotExist(err) {
		return nil, err
	}

	listener, err := net.Listen("unix", socketPath)
	if err != nil {
		return nil, err
	}

	return &Server{
		httpServer: &http.Server{Handler: handler},
		listener:   listener,
		socketPath: socketPath,
	}, nil
}

// Serve starts serving HTTP requests.
func (s *Server) Serve() error {
	return s.httpServer.Serve(s.listener)
}

// Shutdown gracefully shuts down the server.
func (s *Server) Shutdown(ctx context.Context) error {
	err := s.httpServer.Shutdown(ctx)
	os.Remove(s.socketPath)
	return err
}
