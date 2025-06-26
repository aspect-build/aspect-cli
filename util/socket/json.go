package socket

import (
	"encoding/json"
	"fmt"
	"net"
)

type jsonSocket[S, R interface{}] struct {
	conn net.Conn
	read *json.Decoder
}
type jsonClientSocket[S, R interface{}] struct {
	jsonSocket[S, R]
}

var _ Socket[any, any] = (*jsonSocket[any, any])(nil)

func ConnectJsonSocket[S, R interface{}](socketPath string) (Socket[S, R], error) {
	s := &jsonClientSocket[S, R]{}
	if err := s.connect(socketPath); err != nil {
		return nil, err
	}
	return s, nil
}

func (sock *jsonSocket[S, R]) Close() error {
	if sock.conn != nil {
		if err := sock.conn.Close(); err != nil {
			return err
		}
		sock.conn = nil
	}
	return nil
}

func (sock *jsonSocket[S, R]) Recv() (R, error) {
	var resp R

	if sock.conn == nil {
		return resp, fmt.Errorf("not connected to socket")
	}

	if err := sock.read.Decode(&resp); err != nil {
		return resp, fmt.Errorf("failed to parse response: %w", err)
	}

	return resp, nil
}

func (sock *jsonSocket[S, R]) Send(cmd S) error {
	if sock.conn == nil {
		return fmt.Errorf("not connected to socket")
	}

	data, err := json.Marshal(cmd)
	if err != nil {
		return fmt.Errorf("failed to marshal command to json: %w", err)
	}

	_, err = sock.conn.Write(data)
	if err != nil {
		return fmt.Errorf("failed to write to socket: %w", err)
	}
	_, err = sock.conn.Write([]byte("\n"))
	if err != nil {
		return fmt.Errorf("failed to write to socket: %w", err)
	}
	return nil
}

func (sock *jsonClientSocket[S, R]) connect(socketPath string) error {
	if sock.conn != nil {
		return fmt.Errorf("socket already connected")
	}

	conn, err := net.Dial("unix", socketPath)
	if err != nil {
		return err
	}
	sock.conn = conn
	sock.read = json.NewDecoder(sock.conn)
	return nil
}
