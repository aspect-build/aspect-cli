package besproxy

import (
	"context"
	"fmt"
	"log"
	"strings"

	"aspect.build/cli/bazel/buildeventstream"
	"google.golang.org/grpc"
	"google.golang.org/protobuf/types/known/emptypb"

	buildv1 "google.golang.org/genproto/googleapis/devtools/build/v1"
)

const proxyChannelBufferSize = 10000

// The path here comes from the path of the Aspect Workflows bb_clientd unix socket to the
// buildbarn, remote cache:
// https://github.com/aspect-build/silo/blob/f4bd43ce3098345ac7b30a3fae8ddbb1860b814b/infrastructure/modules/workflows/runners/bootstrap/bb_clientd.sh#L53.
// The two must be kept in sync.
const workflowsBbclientdUnixSocketPrefix = "bytestream://///mnt/ephemeral/buildbarn/.cache/bb_clientd/grpc/"

// BESProxy implements a Build Event Protocol backend to be passed to the
// `bazel build` command so that the Aspect plugins can register as subscribers
// to the build events.
type BESProxy interface {
	CloseSend() error
	Connect() error
	Host() string
	PublishBuildToolEventStream(ctx context.Context, opts ...grpc.CallOption) error
	PublishLifecycleEvent(ctx context.Context, req *buildv1.PublishLifecycleEventRequest, opts ...grpc.CallOption) (*emptypb.Empty, error)
	Recv() (*buildv1.PublishBuildToolEventStreamResponse, error)
	Send(req *buildv1.PublishBuildToolEventStreamRequest) error
}

func NewBesProxy(host string, headers map[string]string, remoteCacheAddress string) *besProxy {
	return &besProxy{
		host:               host,
		headers:            headers,
		remoteCacheAddress: remoteCacheAddress,
	}
}

type besProxy struct {
	client             buildv1.PublishBuildEventClient
	stream             buildv1.PublishBuildEvent_PublishBuildToolEventStreamClient
	host               string
	remoteCacheAddress string
	headers            map[string]string
}

func (bp *besProxy) Connect() error {
	c, err := grpcDial(bp.host, bp.headers)
	if err != nil {
		return fmt.Errorf("failed to connect to build event stream backend %s: %w", bp.host, err)
	}
	bp.client = buildv1.NewPublishBuildEventClient(c)
	return nil
}

func (bp *besProxy) Host() string {
	return bp.host
}

func (bp *besProxy) PublishLifecycleEvent(ctx context.Context, req *buildv1.PublishLifecycleEventRequest, opts ...grpc.CallOption) (*emptypb.Empty, error) {
	if bp.client == nil {
		return &emptypb.Empty{}, fmt.Errorf("not connected to %v", bp.host)
	}
	return bp.client.PublishLifecycleEvent(ctx, req)
}

func (bp *besProxy) PublishBuildToolEventStream(ctx context.Context, opts ...grpc.CallOption) error {
	if bp.client == nil {
		return fmt.Errorf("not connected to %v", bp.host)
	}
	s, err := bp.client.PublishBuildToolEventStream(ctx, opts...)
	if err != nil {
		return fmt.Errorf("failed to create build event stream to %v: %w", bp.host, err)
	}
	bp.stream = s
	return nil
}

func (bp *besProxy) Send(req *buildv1.PublishBuildToolEventStreamRequest) error {
	if bp.stream == nil {
		return nil
	}

	MutateWorkflowsUris(req, bp.remoteCacheAddress)

	return bp.stream.Send(req)
}

func (bp *besProxy) Recv() (*buildv1.PublishBuildToolEventStreamResponse, error) {
	if bp.stream == nil {
		return nil, fmt.Errorf("stream to %v not configured", bp.host)
	}
	return bp.stream.Recv()
}

func (bp *besProxy) CloseSend() error {
	if bp.stream == nil {
		return nil
	}
	return bp.stream.CloseSend()
}

// Mutate file URIs in the BES on the way out of the CLI to point to the remote cache grpc address
// instead of the Aspect Workflows runner's remote cache unix socket. this allows external BES
// consumers that have network access to the remove cache to access test logs from the remote cache.
func MutateWorkflowsUris(req *buildv1.PublishBuildToolEventStreamRequest, remoteCacheAddress string) error {
	event := req.OrderedBuildEvent.Event
	if event != nil && remoteCacheAddress != "" {
		bazelEvent := event.GetBazelEvent()
		if bazelEvent != nil {
			var buildEvent buildeventstream.BuildEvent
			if err := bazelEvent.UnmarshalTo(&buildEvent); err != nil {
				return err
			}
			mutated := false
			switch buildEvent.Payload.(type) {
			// TestResult
			case *buildeventstream.BuildEvent_TestResult:
				for _, f := range buildEvent.GetTestResult().TestActionOutput {
					if MutateWorkflowsUri(f, remoteCacheAddress) {
						mutated = true
					}
				}

			// TestSummary
			case *buildeventstream.BuildEvent_TestSummary:
				summary := buildEvent.GetTestSummary()
				if summary.Passed != nil {
					for _, f := range summary.Passed {
						if MutateWorkflowsUri(f, remoteCacheAddress) {
							mutated = true
						}
					}
				}
				if summary.Failed != nil {
					for _, f := range summary.Failed {
						if MutateWorkflowsUri(f, remoteCacheAddress) {
							mutated = true
						}
					}
				}

			// NamedSetOfFiles
			case *buildeventstream.BuildEvent_NamedSetOfFiles:
				for _, f := range buildEvent.GetNamedSetOfFiles().Files {
					if MutateWorkflowsUri(f, remoteCacheAddress) {
						mutated = true
					}
				}

			// Action
			case *buildeventstream.BuildEvent_Action:
				if buildEvent.GetAction().Stdout != nil && MutateWorkflowsUri(buildEvent.GetAction().Stdout, remoteCacheAddress) {
					mutated = true
				}
				if buildEvent.GetAction().Stderr != nil && MutateWorkflowsUri(buildEvent.GetAction().Stderr, remoteCacheAddress) {
					mutated = true
				}
				if buildEvent.GetAction().PrimaryOutput != nil && MutateWorkflowsUri(buildEvent.GetAction().PrimaryOutput, remoteCacheAddress) {
					mutated = true
				}
				for _, f := range buildEvent.GetAction().ActionMetadataLogs {
					if MutateWorkflowsUri(f, remoteCacheAddress) {
						mutated = true
					}
				}

			// BuildToolLogs
			case *buildeventstream.BuildEvent_BuildToolLogs:
				for _, f := range buildEvent.GetBuildToolLogs().Log {
					if MutateWorkflowsUri(f, remoteCacheAddress) {
						mutated = true
					}
				}

			// Completed
			case *buildeventstream.BuildEvent_Completed:
				for _, f := range buildEvent.GetCompleted().ImportantOutput {
					if MutateWorkflowsUri(f, remoteCacheAddress) {
						mutated = true
					}
				}
				for _, f := range buildEvent.GetCompleted().DirectoryOutput {
					if MutateWorkflowsUri(f, remoteCacheAddress) {
						mutated = true
					}
				}

			case nil:
				log.Printf("illegal state: got a BuildEvent with no payload")

			default:
				// Ignore events we don't care about
			}
			if mutated {
				if err := bazelEvent.MarshalFrom(&buildEvent); err != nil {
					return err
				}
			}
		}
	}
	return nil
}

func MutateWorkflowsUri(f *buildeventstream.File, remoteCacheAddress string) bool {
	switch t := f.File.(type) {
	case *buildeventstream.File_Uri:
		if strings.HasPrefix(t.Uri, workflowsBbclientdUnixSocketPrefix) {
			// Mutate from a URI such as,
			// bytestream://///mnt/ephemeral/buildbarn/.cache/bb_clientd/grpc/blobs/f9c5cd4a9f458cf3d801640f6f69eb7523bc590b92d4e7b7b9fa5c7ffa4813cb/194
			// to,
			// bytestream://10.2.0.110:8980/blobs/f9c5cd4a9f458cf3d801640f6f69eb7523bc590b92d4e7b7b9fa5c7ffa4813cb/194
			t.Uri = "bytestream://" + remoteCacheAddress + "/" + t.Uri[len(workflowsBbclientdUnixSocketPrefix):]
			return true
		}

	default:
		// Ignore other types we don't care about
	}
	return false
}
