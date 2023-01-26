/*
 * Copyright 2023 Aspect Build Systems, Inc.
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

package bep_fanner

import (
	"context"
	"crypto/x509"
	"fmt"
	"io"

	"aspect.build/cli/pkg/ioutils"
	"github.com/golang/protobuf/ptypes/empty"
	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/credentials/oauth"
	"google.golang.org/grpc/status"
)

// BEPFanner is the interface for the Build Event Protocol fanner.
type BEPFanner interface {
	buildv1.PublishBuildEventServer
	Configure(configStreams []*ConfigStream) error
}

// bepFanner satisfies the PublishBuildEventServer interface and fans out the build events to the
// upstream servers.
type bepFanner struct {
	logStreams ioutils.Streams
	bepClients []bepClient
}

// NewBEPFanner creates a new BEPFanner.
func NewBEPFanner() BEPFanner {
	return &bepFanner{
		logStreams: ioutils.DefaultStreams,
	}
}

// Configure sets up the BEPFanner to fan out the build events to the provided upstream servers.
func (s *bepFanner) Configure(configStreams []*ConfigStream) error {
	bepClients := make([]bepClient, len(configStreams))
	for i, configStream := range configStreams {
		if err := configStream.setup(); err != nil {
			return fmt.Errorf("failed to configure upstream %s: %w", configStream.Host, err)
		}
		dialOpts, err := configStream.opts.DialOpts()
		if err != nil {
			return fmt.Errorf("failed to configure upstream %s: %w", configStream.Host, err)
		}
		clientConnection, err := grpc.Dial(configStream.Host, dialOpts...)
		if err != nil {
			return fmt.Errorf("failed to configure upstream %s: %w", configStream.Host, err)
		}
		bepClient := bepClient{
			host:                    configStream.Host,
			PublishBuildEventClient: buildv1.NewPublishBuildEventClient(clientConnection),
		}
		bepClients[i] = bepClient
	}
	s.bepClients = bepClients
	return nil
}

// PublishLifecycleEvent implements the gRPC PublishLifecycleEvent service. It forwards the events
// to the upstream servers and returns the responses to the client.
func (s *bepFanner) PublishLifecycleEvent(
	ctx context.Context,
	req *buildv1.PublishLifecycleEventRequest,
) (*empty.Empty, error) {
	// TODO(f0rmiga): make it configurable if the user wants this to never return an error
	// regardless of one of the upstreams returning an error.
	// It may be important to set in the configuration if it's a forwarding error to one of the
	// upstream servers, then it's an error. E.g. it should always be a build error if Buildcop is
	// configured on CI but the BEP Fanner fails to forward the event to it.
	var allErrs error
	for _, client := range s.bepClients {
		if _, err := client.PublishLifecycleEvent(ctx, req); err != nil {
			if allErrs == nil {
				allErrs = fmt.Errorf("failed to call upstream %s.PublishLifecycleEvent: %w", client.host, err)
			} else {
				allErrs = fmt.Errorf("%v: failed to call upstream %s.PublishLifecycleEvent: %w", allErrs, client.host, err)
			}
		}
	}
	if allErrs != nil {
		return nil, status.Error(codes.Internal, allErrs.Error())
	}
	return &empty.Empty{}, nil
}

// PublishBuildToolEventStream implements the gRPC PublishBuildToolEventStream service. It forwards
// the events to the upstream servers and returns the responses to the client.
func (s *bepFanner) PublishBuildToolEventStream(
	stream buildv1.PublishBuildEvent_PublishBuildToolEventStreamServer,
) error {
	ctx := stream.Context()
	errors := make(chan error, len(s.bepClients))
	upstreams := s.publishBuildToolEventStream_connectStreams(ctx, errors)

	go func() {
		defer close(errors)
		for {
			req, err := stream.Recv()
			if err != nil {
				if err != io.EOF {
					errors <- err
				}
				break
			}

			// TODO(f0rmiga): support upstream mode that is async if the upstream is out of our
			// control like a buildbuddy BEP consumer.
			for _, upstream := range upstreams {
				upstream.forward <- req
			}

			res := &buildv1.PublishBuildToolEventStreamResponse{
				StreamId:       req.OrderedBuildEvent.StreamId,
				SequenceNumber: req.OrderedBuildEvent.SequenceNumber,
			}
			if err := stream.Send(res); err != nil {
				errors <- err
				break
			}
		}
		for _, upstream := range upstreams {
			upstream.Wait()
		}
	}()

	hasErrors := false
	for err := range errors {
		fmt.Fprintf(s.logStreams.Stderr, "ERROR: failed to forward build events: %v", err)
		hasErrors = true
	}

	if hasErrors {
		return status.Error(codes.Unknown, "failed to forward one or more build events")
	}

	return nil
}

// publishBuildToolEventStream_connectStreams connects to all upstream servers and starts the stream
// forwarding.
func (s *bepFanner) publishBuildToolEventStream_connectStreams(ctx context.Context, errors chan<- error) []streamClient {
	streams := make([]streamClient, len(s.bepClients))
	for i, client := range s.bepClients {
		clientStream, err := client.PublishBuildToolEventStream(ctx)
		if err != nil {
			errors <- fmt.Errorf("failed to initialize PublishBuildToolEventStream for %s: %w", client.host, err)
			continue
		}
		stream := streamClient{
			host: client.host,
			PublishBuildEvent_PublishBuildToolEventStreamClient: clientStream,
		}
		stream.start(errors)
		streams[i] = stream
	}
	return streams
}

// ConfigStream represents the configuration for a stream.
type ConfigStream struct {
	Host string
	opts ConfigStreamOpts
}

// NewConfigStream creates a new stream configuration.
func NewConfigStream(host string, opts ConfigStreamOpts) *ConfigStream {
	return &ConfigStream{
		Host: host,
		opts: opts,
	}
}

// setup initializes the stream configuration.
func (stream *ConfigStream) setup() error {
	if stream.Host == "" {
		return fmt.Errorf("failed to setup stream: missing 'host'")
	}
	if stream.opts == nil {
		stream.opts = NewInsecureConfigStream()
	}
	return nil
}

// ConfigStreamOpts is an interface that represents the configuration options for a config stream.
type ConfigStreamOpts interface {
	// DialOpts returns the gRPC dial options for the configStreamOpts.
	DialOpts() ([]grpc.DialOption, error)
}

// TLSConfigStream is a wrapper around a string that satisfies the configStreamOpts interface. The
// value of the string is the path to the TLS certificate file.
type TLSConfigStream string

// Ensure that TLSConfigStream satisfies the ConfigStreamOpts interface.
var _ ConfigStreamOpts = (*TLSConfigStream)(nil)

// NewTLSConfigStream returns a new TLSConfigStream.
func NewTLSConfigStream(fileName string) TLSConfigStream {
	return TLSConfigStream(fileName)
}

// DialOpts returns the gRPC dial options for the TLSConfigStream.
func (s *TLSConfigStream) DialOpts() ([]grpc.DialOption, error) {
	// TODO(f0rmiga): allow serverNameOverride from configuration file.
	creds, err := credentials.NewClientTLSFromFile(string(*s), "")
	if err != nil {
		return nil, fmt.Errorf("failed to initialize TLS gRPC dial options: %w", err)
	}
	opts := []grpc.DialOption{
		grpc.WithTransportCredentials(creds),
	}
	return opts, nil
}

// GoogleConfigStream is a wrapper around a string that satisfies the configStreamOpts interface.
// The value of the string is the path to the Google service account token file.
type GoogleConfigStream string

// Ensure that GoogleConfigStream satisfies the ConfigStreamOpts interface.
var _ ConfigStreamOpts = (*GoogleConfigStream)(nil)

// NewGoogleConfigStream returns a new GoogleConfigStream.
func NewGoogleConfigStream(fileName string) GoogleConfigStream {
	return GoogleConfigStream(fileName)
}

// DialOpts returns the gRPC dial options for the GoogleConfigStream.
func (s *GoogleConfigStream) DialOpts() ([]grpc.DialOption, error) {
	// TODO(f0rmiga): allow for custom cert to be injected via config. Big enterprises usually have
	// their own CA certs and they often will want to consume separately from the system certs, aka
	// well-known CA certs (the most widely used list comes from Mozilla, and curl compiles it here:
	// https://curl.se/ca/cacert.pem).
	pool, err := x509.SystemCertPool()
	if err != nil {
		return nil, fmt.Errorf("failed to initialize GOOGLE gRPC dial options: %w", err)
	}
	// TODO(f0rmiga): allow serverNameOverride from configuration file.
	transportCreds := credentials.NewClientTLSFromCert(pool, "")
	perRPCCreds, err := oauth.NewServiceAccountFromFile(string(*s))
	if err != nil {
		return nil, fmt.Errorf("failed to initialize GOOGLE gRPC dial options: %w", err)
	}
	opts := []grpc.DialOption{
		grpc.WithTransportCredentials(transportCreds),
		grpc.WithPerRPCCredentials(perRPCCreds),
	}
	return opts, nil
}

// JWTConfigStream is a wrapper around a string that satisfies the configStreamOpts interface. The
// value of the string is the path to the JWT token file.
type JWTConfigStream string

// Ensure that JWTConfigStream satisfies the ConfigStreamOpts interface.
var _ ConfigStreamOpts = (*JWTConfigStream)(nil)

// NewJWTConfigStream returns a new JWTConfigStream with default values.
func NewJWTConfigStream(fileName string) JWTConfigStream {
	return JWTConfigStream(fileName)
}

// DialOpts returns the gRPC dial options for the JWTConfigStream.
func (s *JWTConfigStream) DialOpts() ([]grpc.DialOption, error) {
	// TODO(f0rmiga): implement this.
	return nil, fmt.Errorf("failed to initialize JWT gRPC dial options: NOT IMPLEMENTED")
}

// InsecureConfigStream satisfies the configStreamOpts interface and is used when no authentication
// method is provided.
type InsecureConfigStream struct{}

// Ensure that InsecureConfigStream satisfies the ConfigStreamOpts interface.
var _ ConfigStreamOpts = (*InsecureConfigStream)(nil)

// NewInsecureConfigStream returns a new InsecureConfigStream.
func NewInsecureConfigStream() *InsecureConfigStream {
	return &InsecureConfigStream{}
}

// DialOpts returns the gRPC dial options for the InsecureConfigStream.
func (s *InsecureConfigStream) DialOpts() ([]grpc.DialOption, error) {
	opts := []grpc.DialOption{
		grpc.WithTransportCredentials(insecure.NewCredentials()),
	}
	return opts, nil
}

// bepClient is a wrapper around the PublishBuildEventClient that satisfies the
// BuildEventServiceClient.
type bepClient struct {
	buildv1.PublishBuildEventClient

	host string
}

// streamClient is a wrapper around the PublishBuildToolEventStreamClient.
type streamClient struct {
	buildv1.PublishBuildEvent_PublishBuildToolEventStreamClient

	host        string
	forward     sendChannel
	doneRecv    doneChannel
	doneForward doneChannel
}

// sendChannel is a channel that sends PublishBuildToolEventStreamRequest. It is used to forward
// events from the client to the server.
type sendChannel chan<- *buildv1.PublishBuildToolEventStreamRequest

// doneChannel is a channel that sends a done signal. It is used to signal that the client has been
// closed.
type doneChannel <-chan struct{}

// start starts the streamClient. It starts two goroutines: one to receive events from the upstream
// server and one to forward events to the upstream server.
func (sc *streamClient) start(errors chan<- error) {
	forward := make(chan *buildv1.PublishBuildToolEventStreamRequest, 1024)
	sc.forward = forward
	doneRecv := make(chan struct{}, 1)
	sc.doneRecv = doneRecv
	doneForward := make(chan struct{}, 1)
	sc.doneForward = doneForward
	go func() {
		for {
			if _, err := sc.PublishBuildEvent_PublishBuildToolEventStreamClient.Recv(); err != nil {
				if err == io.EOF {
					break
				}
				errors <- fmt.Errorf("failed to forward event to %s: %w", sc.host, err)
			}
		}
		close(doneRecv)
	}()
	go func() {
		for req := range forward {
			if err := sc.PublishBuildEvent_PublishBuildToolEventStreamClient.Send(req); err != nil {
				errors <- fmt.Errorf("failed to forward event to %s: %w", sc.host, err)
			}
		}
		close(doneForward)
	}()
}

// Wait waits for the streamClient to be closed.
func (sc *streamClient) Wait() {
	close(sc.forward)
	<-sc.doneForward
	sc.PublishBuildEvent_PublishBuildToolEventStreamClient.CloseSend()
	<-sc.doneRecv
}
