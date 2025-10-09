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

package bep

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path"
	"sync"
	"syscall"
	"time"

	buildeventstream "github.com/aspect-build/aspect-cli/bazel/buildeventstream"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"golang.org/x/sync/errgroup"
	"google.golang.org/protobuf/encoding/protodelim"
)

const besSocketInterceptorKey besBackendInterceptorKeyType = 0x00

type BESSocket interface {
	Setup() error
	ServeWait(ctx context.Context) error
	GracefulStop()
	Args() []string
	RegisterSubscriber(callback CallbackFn)
	Errors() []error
}

func BESSocketFromContext(ctx context.Context) BESSocket {
	return ctx.Value(besSocketInterceptorKey).(BESSocket)
}

func HasBESSocket(ctx context.Context) bool {
	return ctx.Value(besSocketInterceptorKey) != nil
}

func BESSocketErrors(ctx context.Context) []error {
	if !HasBESSocket(ctx) {
		return []error{}
	}
	return BESSocketFromContext(ctx).Errors()
}

// InjectBESSocket injects the given BESSocket into the context.
func InjectBESSocket(ctx context.Context, besSocket BESSocket) context.Context {
	return context.WithValue(ctx, besSocketInterceptorKey, besSocket)
}

func NewBESSocket(ctx context.Context) (BESSocket, error) {
	return &besSocket{
		ctx:         ctx,
		socketPath:  path.Join(os.TempDir(), fmt.Sprintf("aspect-cli-%v-bes.bin", os.Getpid())),
		errors:      &aspecterrors.ErrorList{},
		subscribers: &subscriberList{},
	}, nil
}

type besSocket struct {
	ctx         context.Context
	socketPath  string
	errors      *aspecterrors.ErrorList
	errorsMutex sync.RWMutex
	subscribers *subscriberList
}

var _ BESSocket = (*besSocket)(nil)

func (bb *besSocket) Setup() error {
	err := syscall.Mknod(bb.socketPath, syscall.S_IFIFO|0666, 0)
	if err != nil {
		return fmt.Errorf("failed to create BES socket %s: %w", bb.socketPath, err)
	}
	return nil
}

func (bb *besSocket) ServeWait(ctx context.Context) error {
	go func() {
		conn, err := os.OpenFile(bb.socketPath, os.O_RDONLY, os.ModeNamedPipe)
		if err != nil {
			bb.errorsMutex.Lock()
			defer bb.errorsMutex.Unlock()
			bb.errors.Insert(fmt.Errorf("failed to accept connection on BES socket %s: %w", bb.socketPath, err))
			return
		}

		defer conn.Close()

		if err := bb.streamBesEvents(ctx, conn); err != nil {
			bb.errorsMutex.Lock()
			defer bb.errorsMutex.Unlock()
			bb.errors.Insert(fmt.Errorf("failed to stream BES events: %w", err))
			return
		}
	}()
	return nil
}

func (bb *besSocket) streamBesEvents(ctx context.Context, r io.Reader) error {
	reader := bufio.NewReader(r)

	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		default:
		}

		event := buildeventstream.BuildEvent{}

		if err := protodelim.UnmarshalFrom(reader, &event); err != nil {
			if errors.Is(err, io.EOF) {
				select {
				case <-ctx.Done():
					return ctx.Err()
				case <-time.After(50):
					// throttle the reading of the BES file when no new data is available
					continue
				}
			}

			return fmt.Errorf("failed to parse BES event: %w", err)
		}

		if err := bb.publishBesEvent(&event); err != nil {
			return fmt.Errorf("failed to publish BES event: %w", err)
		}

		if event.LastMessage {
			break
		}
	}

	return nil
}

func (bb *besSocket) publishBesEvent(event *buildeventstream.BuildEvent) error {
	eg := errgroup.Group{}

	for s := bb.subscribers.head; s != nil; s = s.next {
		cb := s.callback
		eg.Go(
			func() error {
				return cb(event, -1)
			},
		)
	}

	return eg.Wait()
}

func (bb *besSocket) Args() []string {
	return []string{
		"--build_event_publish_all_actions",
		// TODO: when bazel6 dropped
		// "--build_event_binary_file_upload_mode=fully_async",
		"--build_event_binary_file",
		bb.socketPath,
	}
}

func (bb *besSocket) RegisterSubscriber(callback CallbackFn) {
	bb.subscribers.Insert(callback)
}

func (bb *besSocket) Errors() []error {
	bb.errorsMutex.RLock()
	defer bb.errorsMutex.RUnlock()
	return bb.errors.Errors()
}

func (bb *besSocket) GracefulStop() {
	os.Remove(bb.socketPath)
}
