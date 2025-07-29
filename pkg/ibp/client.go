package ibp

import (
	"fmt"
	"slices"

	"github.com/aspect-build/aspect-cli/util/socket"
)

type IncrementalClient interface {
	Connect() error
	Disconnect() error
	AwaitCycle() <-chan CycleSourcesMessage
}

type incClient struct {
	socketPath string
	socket     socket.Socket[interface{}, map[string]interface{}]
}

var _ IncrementalClient = (*incClient)(nil)

func NewClient(host string) IncrementalClient {
	return &incClient{
		socketPath: host,
	}
}
func (c *incClient) Connect() error {
	if c.socket != nil {
		return fmt.Errorf("client already connected")
	}

	socket, err := socket.ConnectJsonSocket[interface{}, map[string]interface{}](c.socketPath)
	if err != nil {
		return err
	}
	c.socket = socket

	if err := c.negotiate(); err != nil {
		return fmt.Errorf("failed to negotiate protocol version: %w", err)
	}
	return nil
}

func (c *incClient) negotiate() error {
	negReq, err := c.socket.Recv()
	if err != nil {
		return err
	}

	if negReq["kind"] != "NEGOTIATE" {
		return fmt.Errorf("Expected NEGOTIATE, got %v", negReq)
	}
	if negReq["versions"] == nil {
		return fmt.Errorf("Received NEGOTIATE without versions: %v", negReq)
	}
	if !slices.Contains(negReq["versions"].([]interface{}), (interface{})(float64(PROTOCOL_VERSION))) {
		return fmt.Errorf("Received NEGOTIATE with unsupported versions %v, expected %d", negReq["versions"], PROTOCOL_VERSION)
	}

	err = c.socket.Send(negotiateResponseMessage{
		Message: Message{
			Kind: "NEGOTIATE_RESPONSE",
		},
		Version: PROTOCOL_VERSION,
	})
	if err != nil {
		return fmt.Errorf("failed to negotiate protocol version: %w", err)
	}
	return nil
}

func (c *incClient) Disconnect() error {
	if c.socket == nil {
		return fmt.Errorf("client not connected")
	}

	err := c.socket.Close()
	if err != nil {
		return fmt.Errorf("failed to close socket: %w", err)
	}
	c.socket = nil
	return err
}

func (c *incClient) AwaitCycle() <-chan CycleSourcesMessage {
	ch := make(chan CycleSourcesMessage)

	go func() {
		defer close(ch)
		for {
			msg, err := c.socket.Recv()
			if err != nil {
				fmt.Printf("Error receiving message: %v\n", err)
				return
			}

			if msg["kind"] == "CYCLE" {
				cycleEvent, cycleErr := convertWireCycle(msg)
				if cycleErr != nil {
					fmt.Printf("Failed read cycle: %v\n", cycleErr)
					continue
				}

				c.socket.Send(CycleMessage{
					Message: Message{
						Kind: "CYCLE_STARTED",
					},
					CycleId: cycleEvent.CycleId,
				})

				ch <- cycleEvent

				c.socket.Send(CycleMessage{
					Message: Message{
						Kind: "CYCLE_COMPLETED",
					},
					CycleId: cycleEvent.CycleId,
				})
			} else {
				fmt.Printf("Expected CYCLE, received: %v\n", msg)
				continue
			}
		}
	}()

	return ch
}

func convertWireCycle(msg map[string]interface{}) (CycleSourcesMessage, error) {
	if msg["kind"] != "CYCLE" {
		return CycleSourcesMessage{}, fmt.Errorf("Expected CYCLE, got %v", msg["kind"])
	}

	cycleIdFloat, cycleIdIsFloat := msg["cycle_id"].(float64)
	if !cycleIdIsFloat {
		return CycleSourcesMessage{}, fmt.Errorf("Invalid cycle_id type: %T", msg["cycle_id"])
	}

	cycleId := int(cycleIdFloat)

	sources := make(SourceInfoMap, len(msg["sources"].(map[string]interface{})))
	for k, v := range msg["sources"].(map[string]interface{}) {
		sources[k] = &SourceInfo{
			IsSymlink: readOptionalBool(v.(map[string]interface{}), "is_symlink"),
			IsSource:  readOptionalBool(v.(map[string]interface{}), "is_source"),
		}
	}

	return CycleSourcesMessage{
		Message: Message{
			Kind: "CYCLE",
		},
		CycleId: cycleId,
		Sources: sources,
	}, nil
}

func readOptionalBool(m map[string]interface{}, key string) *bool {
	if val, ok := m[key]; ok {
		if boolVal, ok := val.(*bool); ok {
			return boolVal
		}
	}
	return nil
}
