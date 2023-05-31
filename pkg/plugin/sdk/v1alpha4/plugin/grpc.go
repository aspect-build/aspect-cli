/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

// grpc.go hides all the complexity of doing the gRPC calls between the aspect
// Core and a Plugin implementation by providing simple abstractions from the
// point of view of Plugin maintainers.
package plugin

import (
	"context"
	"fmt"
	"sync"

	goplugin "github.com/hashicorp/go-plugin"
	"github.com/manifoldco/promptui"
	"google.golang.org/grpc"

	buildeventstream "aspect.build/cli/bazel/buildeventstream"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha4/proto"
)

// GRPCPlugin represents a Plugin that communicates over gRPC.
type GRPCPlugin struct {
	goplugin.Plugin
	Impl Plugin
}

// GRPCServer registers an instance of the GRPCServer in the Plugin binary.
func (p *GRPCPlugin) GRPCServer(broker *goplugin.GRPCBroker, s *grpc.Server) error {
	proto.RegisterPluginServer(s, &GRPCServer{
		Impl:   p.Impl,
		broker: broker,
		commandManager: &PluginCommandManager{
			commands: make(map[string]CustomCommandFn),
		},
	})
	return nil
}

// GRPCClient returns a client to perform the RPC calls to the Plugin
// instance from the Core.
func (p *GRPCPlugin) GRPCClient(ctx context.Context, broker *goplugin.GRPCBroker, c *grpc.ClientConn) (interface{}, error) {
	return &GRPCClient{client: proto.NewPluginClient(c), broker: broker}, nil
}

// GRPCServer implements the gRPC server that runs on the Plugin instances.
type GRPCServer struct {
	Impl           Plugin
	broker         *goplugin.GRPCBroker
	commandManager CommandManager
}

// BEPEventCallback translates the gRPC call to the Plugin BEPEventCallback
// implementation.
func (m *GRPCServer) BEPEventCallback(
	ctx context.Context,
	req *proto.BEPEventCallbackReq,
) (*proto.BEPEventCallbackRes, error) {
	return &proto.BEPEventCallbackRes{}, m.Impl.BEPEventCallback(req.Event)
}

// Setup translates the gRPC call to the Plugin Setup implementation.
func (m *GRPCServer) Setup(
	ctx context.Context,
	req *proto.SetupReq,
) (*proto.SetupRes, error) {
	config := NewSetupConfig(req.Properties)
	return &proto.SetupRes{}, m.Impl.Setup(config)
}

// CustomCommands translates the gRPC call to the Plugin CustomCommands
// implementation. It returns a list of commands that the plugin implements.
func (m *GRPCServer) CustomCommands(
	ctx context.Context,
	req *proto.CustomCommandsReq,
) (*proto.CustomCommandsRes, error) {
	customCommands, err := m.Impl.CustomCommands()

	if err != nil {
		return nil, err
	}

	m.commandManager.Save(customCommands)

	pbCommands := make([]*proto.Command, 0, len(customCommands))
	for _, command := range customCommands {
		pbCommands = append(pbCommands, command.Command)
	}

	pb := &proto.CustomCommandsRes{
		Commands: pbCommands,
	}

	return pb, nil
}

// ExecuteCustomCommand translates the gRPC call to the sdk ExecuteCustomCommand
// implementation.
func (m *GRPCServer) ExecuteCustomCommand(
	_ context.Context,
	req *proto.ExecuteCustomCommandReq,
) (*proto.ExecuteCustomCommandRes, error) {
	ctx := context.Background()

	return &proto.ExecuteCustomCommandRes{},
		m.commandManager.Execute(req.CustomCommand, ctx, req.Args, req.BazelStartupArgs)
}

// PostBuildHook translates the gRPC call to the Plugin PostBuildHook
// implementation. It starts a prompt runner that is passed to the Plugin
// instance to be able to perform prompt actions to the CLI user.
func (m *GRPCServer) PostBuildHook(
	ctx context.Context,
	req *proto.PostBuildHookReq,
) (*proto.PostBuildHookRes, error) {
	conn, err := m.broker.Dial(req.BrokerId)
	if err != nil {
		return nil, err
	}
	defer conn.Close()

	client := proto.NewPrompterClient(conn)
	prompter := &PrompterGRPCClient{client: client}
	return &proto.PostBuildHookRes{},
		m.Impl.PostBuildHook(req.IsInteractiveMode, prompter)
}

// PostTestHook translates the gRPC call to the Plugin PostTestHook
// implementation. It starts a prompt runner that is passed to the Plugin
// instance to be able to perform prompt actions to the CLI user.
func (m *GRPCServer) PostTestHook(
	ctx context.Context,
	req *proto.PostTestHookReq,
) (*proto.PostTestHookRes, error) {
	conn, err := m.broker.Dial(req.BrokerId)
	if err != nil {
		return nil, err
	}
	defer conn.Close()

	client := proto.NewPrompterClient(conn)
	prompter := &PrompterGRPCClient{client: client}
	return &proto.PostTestHookRes{},
		m.Impl.PostTestHook(req.IsInteractiveMode, prompter)
}

// PostRunHook translates the gRPC call to the Plugin PostRunHook
// implementation. It starts a prompt runner that is passed to the Plugin
// instance to be able to perform prompt actions to the CLI user.
func (m *GRPCServer) PostRunHook(
	ctx context.Context,
	req *proto.PostRunHookReq,
) (*proto.PostRunHookRes, error) {
	conn, err := m.broker.Dial(req.BrokerId)
	if err != nil {
		return nil, err
	}
	defer conn.Close()

	client := proto.NewPrompterClient(conn)
	prompter := &PrompterGRPCClient{client: client}
	return &proto.PostRunHookRes{},
		m.Impl.PostRunHook(req.IsInteractiveMode, prompter)
}

// GRPCClient implements the gRPC client that is used by the Core to communicate
// with the Plugin instances.
type GRPCClient struct {
	client proto.PluginClient
	broker *goplugin.GRPCBroker
}

var _ Plugin = (*GRPCClient)(nil)

// BEPEventCallback is called from the Core to execute the Plugin
// BEPEventCallback.
func (m *GRPCClient) BEPEventCallback(event *buildeventstream.BuildEvent) error {
	_, err := m.client.BEPEventCallback(context.Background(), &proto.BEPEventCallbackReq{Event: event})
	return err
}

// Setup is called from the Core to execute the Plugin Setup.
func (m *GRPCClient) Setup(config *SetupConfig) error {
	file := &proto.File{Path: ""}
	if config.File != nil {
		file = &proto.File{
			Path: config.File.Path,
		}
	}
	req := &proto.SetupReq{
		Properties: config.Properties,
		File:       file,
	}
	_, err := m.client.Setup(context.Background(), req)
	return err
}

// CustomCommands is called from the Core to execute the Plugin CustomCommands.
// It returns a list of commands that the plugin implements.
func (m *GRPCClient) CustomCommands() ([]*Command, error) {
	req := &proto.CustomCommandsReq{}
	customCommandsPB, err := m.client.CustomCommands(context.Background(), req)

	customCommands := make([]*Command, 0, len(customCommandsPB.Commands))

	for _, pbCommand := range customCommandsPB.Commands {
		customCommands = append(customCommands, &Command{Command: pbCommand})
	}

	return customCommands, err
}

// ExecuteCustomCommand is called from the Core to execute the sdk ExecuteCustomCommand.
func (m *GRPCClient) ExecuteCustomCommand(customCommand string, ctx context.Context, args []string, bazelStartupArgs []string) error {
	pbContext := &proto.Context{}

	req := &proto.ExecuteCustomCommandReq{
		CustomCommand:    customCommand,
		Ctx:              pbContext,
		Args:             args,
		BazelStartupArgs: bazelStartupArgs,
	}
	_, err := m.client.ExecuteCustomCommand(context.Background(), req)
	return err
}

// PostBuildHook is called from the Core to execute the Plugin PostBuildHook. It
// starts the prompt runner server with the provided PromptRunner.
func (m *GRPCClient) PostBuildHook(isInteractiveMode bool, promptRunner ioutils.PromptRunner) error {
	return callClientHook(m.broker, m.client.PostBuildHook, isInteractiveMode, promptRunner)
}

// PostTestHook is called from the Core to execute the Plugin PostTestHook. It
// starts the prompt runner server with the provided PromptRunner.
func (m *GRPCClient) PostTestHook(isInteractiveMode bool, promptRunner ioutils.PromptRunner) error {
	return callClientHook(m.broker, m.client.PostTestHook, isInteractiveMode, promptRunner)
}

// PostRunHook is called from the Core to execute the Plugin PostRunHook. It
// starts the prompt runner server with the provided PromptRunner.
func (m *GRPCClient) PostRunHook(isInteractiveMode bool, promptRunner ioutils.PromptRunner) error {
	return callClientHook(m.broker, m.client.PostRunHook, isInteractiveMode, promptRunner)
}

func callClientHook[
	ReqT proto.PostBuildHookReq | proto.PostTestHookReq | proto.PostRunHookReq,
	ResT proto.PostBuildHookRes | proto.PostTestHookRes | proto.PostRunHookRes,
](
	broker *goplugin.GRPCBroker,
	callFn func(context.Context, *ReqT, ...grpc.CallOption) (*ResT, error),
	isInteractiveMode bool,
	promptRunner ioutils.PromptRunner,
) error {
	prompterServer := &PrompterGRPCServer{promptRunner: promptRunner}
	var s *grpc.Server
	var wg sync.WaitGroup
	wg.Add(1)
	serverFunc := func(opts []grpc.ServerOption) *grpc.Server {
		s = grpc.NewServer(opts...)
		proto.RegisterPrompterServer(s, prompterServer)
		defer wg.Done()
		return s
	}
	brokerID := broker.NextId()
	go broker.AcceptAndServe(brokerID, serverFunc)
	req := &ReqT{
		BrokerId:          brokerID,
		IsInteractiveMode: isInteractiveMode,
	}
	wg.Wait()
	_, err := callFn(context.Background(), req)
	s.Stop()
	return err
}

// PrompterGRPCServer implements the gRPC server that runs on the Core and is
// passed to the Plugin to allow prompt actions to the CLI user.
type PrompterGRPCServer struct {
	promptRunner ioutils.PromptRunner
}

// Run translates the gRPC call to perform a prompt Run on the Core.
func (p *PrompterGRPCServer) Run(
	ctx context.Context,
	req *proto.PromptRunReq,
) (*proto.PromptRunRes, error) {
	prompt := promptui.Prompt{
		Label:       req.GetLabel(),
		Default:     req.GetDefault(),
		AllowEdit:   req.GetAllowEdit(),
		Mask:        []rune(req.GetMask())[0],
		HideEntered: req.GetHideEntered(),
		IsConfirm:   req.GetIsConfirm(),
		IsVimMode:   req.GetIsVimMode(),
	}

	result, err := p.promptRunner.Run(prompt)
	res := &proto.PromptRunRes{Result: result}
	if err != nil {
		res.Error = &proto.PromptRunRes_Error{
			Happened: true,
			Message:  err.Error(),
		}
	}

	return res, nil
}

// PrompterGRPCClient implements the gRPC client that is used by the Plugin
// instance to communicate with the Core to request prompt actions from the
// user.
type PrompterGRPCClient struct {
	client proto.PrompterClient
}

// Run is called from the Plugin to request the Core to run the given
// promptui.Prompt.
func (p *PrompterGRPCClient) Run(prompt promptui.Prompt) (string, error) {
	label, isString := prompt.Label.(string)
	if !isString {
		return "", fmt.Errorf("label '%+v' must be a string", prompt.Label)
	}
	req := &proto.PromptRunReq{
		Label:       label,
		Default:     prompt.Default,
		AllowEdit:   prompt.AllowEdit,
		Mask:        string(prompt.Mask),
		HideEntered: prompt.HideEntered,
		IsConfirm:   prompt.IsConfirm,
		IsVimMode:   prompt.IsVimMode,
	}
	res, err := p.client.Run(context.Background(), req)
	if err != nil {
		return "", err
	}
	if res.Error != nil && res.Error.Happened {
		return "", fmt.Errorf(res.Error.Message)
	}
	return res.Result, nil
}
