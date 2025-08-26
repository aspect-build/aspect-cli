package ibp

import (
	"context"
	"fmt"
	"os"
	"path"
	"sync/atomic"

	"github.com/aspect-build/aspect-cli/gazelle/common/socket"
	"github.com/fatih/color"
)

const PROTOCOL_VERSION = 0
const PROTOCOL_SOCKET_ENV = "ABAZEL_WATCH_SOCKET_FILE"

type IncrementalBazel interface {
	// Messaging to the client
	Init(sources SourceInfoMap) error
	Cycle(changes SourceInfoMap) error
	Exit(err error) error

	// Server + Connection to client
	Serve(ctx context.Context) error
	Close() error
	HasConnection() bool

	WaitForConnection() <-chan int

	// The path/address a client can connect to.
	Address() string

	// Env variables to provide to clients and potential clients.
	Env() []string
}

type Message struct {
	Kind string `json:"kind"`
}

type negotiateMessage struct {
	Message
	Versions []int `json:"versions"`
}
type negotiateResponseMessage struct {
	Message
	Version int `json:"version"`
}

type capMessage struct {
	Message
	Caps map[string]bool `json:"caps"`
}

type exitMessage struct {
	Message
	Description string `json:"description"`
}

type SourceInfo struct {
	IsSymlink *bool `json:"is_symlink,omitempty"`
	IsSource  *bool `json:"is_source,omitempty"`

	// TODO: is_directory? mtime? generated?
}
type SourceInfoMap = map[string]*SourceInfo

type CycleMessage struct {
	Message
	CycleId int `json:"cycle_id"`
}

type CycleSourcesMessage struct {
	Message
	CycleId int           `json:"cycle_id"`
	Sources SourceInfoMap `json:"sources"`
}

// The versions supported by this host implementation of the protocol.
var abazelSupportedProtocolVersions = []int{PROTOCOL_VERSION}

type aspectBazelSocket = socket.Server[interface{}, map[string]any]

type aspectBazelProtocol struct {
	socket     aspectBazelSocket
	socketPath string

	connectedCh chan int

	// cycle_id is used to track the current cycle number.
	cycle_id atomic.Int32
}

var _ IncrementalBazel = (*aspectBazelProtocol)(nil)

func NewServer() IncrementalBazel {
	socketPath := path.Join(os.TempDir(), fmt.Sprintf("aspect-watch-%v-socket", os.Getpid()))
	return &aspectBazelProtocol{
		socketPath: socketPath,
		socket:     socket.NewJsonServer[interface{}, map[string]interface{}](),

		connectedCh: make(chan int, 1),
	}
}

func (p *aspectBazelProtocol) WaitForConnection() <-chan int {
	return p.connectedCh
}

func (p *aspectBazelProtocol) Env() []string {
	return []string{
		PROTOCOL_SOCKET_ENV + "=" + p.socketPath,
	}
}

func (p *aspectBazelProtocol) Address() string {
	return p.socketPath
}

func (p *aspectBazelProtocol) Serve(ctx context.Context) error {
	if err := p.socket.Serve(p.socketPath); err != nil {
		return err
	}

	go func() {
		// TODO: cancel the if the context is done or if Close() invoked
		if err := p.acceptNegotiation(); err != nil {
			select {
			case <-ctx.Done():
				// Ignore errors if the context is done, as this is likely due to shutdown.
			default:
				// Ignore errors due to no connection being accepted.
				if err == socket.ErrNotAccepted {
					return
				}

				fmt.Printf("%s Failed to connect to aspect bazel protocol at %s: %v\n", color.RedString("ERROR:"), p.socketPath, err)
			}
		}
	}()

	return nil
}

func (p *aspectBazelProtocol) HasConnection() bool {
	return p.socket != nil && p.socket.HasConnection()
}

func (p *aspectBazelProtocol) acceptNegotiation() error {
	// Wait for a connection
	if err := p.socket.Accept(); err != nil {
		return err
	}

	// Negotiate the protocol version
	m := negotiateMessage{
		Message:  Message{Kind: "NEGOTIATE"},
		Versions: abazelSupportedProtocolVersions,
	}
	if err := p.socket.Send(m); err != nil {
		return fmt.Errorf("Failed to send NEGOTIATE: %v", err)
	}

	negResp, err := p.socket.Recv()
	if err != nil {
		return fmt.Errorf("Error receiving NEGOTIATE response: %v", err)
	}

	if negResp["kind"] != "NEGOTIATE_RESPONSE" {
		return fmt.Errorf("Expected NEGOTIATE_RESPONSE, got %v", negResp)
	}
	if negResp["version"] == nil {
		return fmt.Errorf("Received NEGOTIATE_RESPONSE without version: %v", negResp)
	}
	if negResp["version"].(float64) != PROTOCOL_VERSION {
		return fmt.Errorf("Received NEGOTIATE_RESPONSE with unsupported version %v, expected %d", negResp["version"], PROTOCOL_VERSION)
	}

	p.connectedCh <- int(negResp["version"].(float64))

	return nil
}

func (p *aspectBazelProtocol) Close() error {
	if p.socket == nil {
		return nil
	}
	if err := p.socket.Close(); err != nil {
		return err
	}
	p.socket = nil
	return nil
}

func (p *aspectBazelProtocol) Init(sources SourceInfoMap) error {
	return p.Cycle(sources)
}

func (p *aspectBazelProtocol) Cycle(changes SourceInfoMap) error {
	cycle_id := int(p.cycle_id.Add(1))

	fmt.Printf("%s Sending cycle #%v (%v changes) to %s\n", color.GreenString("INFO:"), cycle_id, len(changes), p.socketPath)

	c := CycleSourcesMessage{
		Message: Message{Kind: "CYCLE"},
		CycleId: cycle_id,
		Sources: changes,
	}
	if err := p.socket.Send(c); err != nil {
		return err
	}

	for {
		resp, err := p.socket.Recv()
		if err != nil {
			return err
		}

		if resp["cycle_id"] == nil {
			return fmt.Errorf("Received unexpected response without cycle_id: %v", resp)
		}

		receivedCicleId := resp["cycle_id"]

		if receivedCicleId != float64(cycle_id) {
			return fmt.Errorf("Received unexpected cycle response to cycle_id=%d: %v", cycle_id, resp)
		}

		switch resp["kind"] {
		// Still pending events
		case "CYCLE_STARTED":
			continue

		// End events
		case "CYCLE_ABORTED":
			fallthrough
		case "CYCLE_FAILED":
			fmt.Printf("%s received %v event: %v\n", color.RedString("ERROR:"), resp["kind"], resp)
			return nil

		case "CYCLE_COMPLETED":
			return nil

		default:
			return fmt.Errorf("Received unexpected response kind %v for cycle_id=%d: %v", resp["kind"], cycle_id, resp)
		}
	}
}

func (p *aspectBazelProtocol) Exit(err error) error {
	d := ""
	if err != nil {
		d = err.Error()
	}

	c := exitMessage{
		Message:     Message{Kind: "EXIT"},
		Description: d,
	}
	return p.socket.Send(c)
}
