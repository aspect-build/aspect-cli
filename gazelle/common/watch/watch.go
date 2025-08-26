package watch

import (
	"context"
	"encoding/json"
	"fmt"
	"iter"
	"os"
	"os/exec"
	"path"
	"sync/atomic"

	"github.com/aspect-build/aspect-cli/gazelle/common/bazel"
	"github.com/aspect-build/aspect-cli/gazelle/common/socket"
)

type ChangeSet struct {
	// Workspace Relative paths of changed files. eg: ["src/urchin/urchin.go", "src/urchin/urchin_test.go"]
	Paths []string
	// Root of the workspace. eg: /Users/thesayyn/Documents/urchin
	Root string
	// ClockSpec is a point in time that represents the state of the filesystem
	// you could use this to query changes since then but DO NOT rely on the specifics
	// of this string, treat it as an opaque token
	ClockSpec string

	// Fresh instance means that the watchman daemon has no prior knowledge of the state of the filesystem
	// and is starting from scratch.
	//
	// Normally watchman would report all the files in the workspace as changed in this case but we set the
	// `empty_on_fresh_instance` parameter to true in the query to avoid this because we want to traverse the
	// filesystem ourselves. In the future we mgiht to rely on watchman to get initial state of the filesystem
	// instead of traversing it ourselves.
	//
	// Here are some cases where `IsFreshInstance` might be true:
	//
	// 1. The watchman daemon was restarted since your last query
	// 2. The workspace watch was cancelled and restarted.
	// 3. Watchman is not able to track the changes
	// 4. The system was unable to keep up with the rate of change of watched files and the kernel flushed the queue to keep up.
	//    A recrawl of the filesystem by watchman to re-examine the watched tree to determine the current state.
	// 5. You're using timestamps rather than clocks and the timestamp is out of range of known events.
	// 6. You're using a `named cursor`` and that name has not been used before.
	// 7. You're using a blank clock string for the since generator in a query (this is not the same thing as a since term in a query expression!)
	//
	// IMPORTANT: IsFreshInstance ought to indicate a cache discard and a full traversal of the filesystem.
	IsFreshInstance bool
}

// Watcher is an interface that abstracts the underlying filesystem watching mechanism
type Watcher interface {
	Start() error
	Stop() error
	GetDiff(clockspec string) (*ChangeSet, error)
	Subscribe(ctx context.Context, dropWithinState string) iter.Seq2[*ChangeSet, error]
	Close() error
}

type watchmanSocket = socket.Socket[[]interface{}, map[string]interface{}]

type WatchmanWatcher struct {
	// Root of the bazel workspace being watched
	workspaceDir string

	// Watchman socket
	socket watchmanSocket
	// Path to watchman executable
	watchmanPath string
	// Last clockspec used in the query
	lastClockSpec string
	// Atomic counter for incrementing subscriber IDs for every call to Subscribe
	subscriberId atomic.Uint64

	// The root of the watchman project
	watchedRoot string

	// The relative path from the watchman project to the worksapce root
	watchedRelPath string
}

func (w *WatchmanWatcher) makeQueryParams(clockspec string) map[string]interface{} {
	bazelignoreDirs, err := bazel.LoadBazelIgnore(w.workspaceDir)
	if err != nil {
		fmt.Printf("failed to load bazelignore: %v", err)
	}

	bazelignoreDirnameExpressions := make([]interface{}, 0, len(bazelignoreDirs))
	for _, ignoredDir := range bazelignoreDirs {
		bazelignoreDirnameExpressions = append(bazelignoreDirnameExpressions, []interface{}{
			"dirname", ignoredDir,
		})
	}

	var queryParams = map[string]interface{}{
		"fields": []string{"name"},
		// Avoid an unnecessarily long response on the first query by omitting the list of potentially
		// changed (thus at that point, all) files.
		// See ChangeSet.IsFreshInstance for more information.
		// FR: maybe stop gazelle from traversing the filesystem on the first query and use this instead.
		"empty_on_fresh_instance": true,

		"relative_root": w.watchedRelPath,
		"expression": []interface{}{
			"not",
			append(
				[]interface{}{
					"anyof",
					// maybe not exclude directories? or just report directories to determine what directories have changed?
					[]interface{}{
						"type", "d",
					},
				},
				bazelignoreDirnameExpressions...,
			),
		},
		"ignore_dirs": bazelignoreDirs,
	}

	if clockspec != "" {
		queryParams["since"] = clockspec
	}

	return queryParams
}

func (w *WatchmanWatcher) findWatchman() error {
	if w.watchmanPath != "" {
		return nil
	}
	p, err := exec.LookPath("watchman")
	if err != nil {
		// FR: automatically install watchman if not found
		return fmt.Errorf("watchman not found in PATH: %w, did you install it?", err)
	}
	w.watchmanPath = p
	return nil
}

func (w *WatchmanWatcher) getWatchmanSocket() (string, error) {
	if err := w.findWatchman(); err != nil {
		return "", err
	}
	cmd := exec.Command(w.watchmanPath, "get-sockname")
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("failed to get watchman socket: %w", err)
	}

	var sockname map[string]string
	if err := json.Unmarshal(out, &sockname); err != nil {
		return "", fmt.Errorf("failed to parse get-socketname output: %w", err)
	}

	if sockname := sockname["sockname"]; sockname == "" {
		return "", fmt.Errorf("watchman socket not found")
	}
	return sockname["sockname"], nil
}

func (w *WatchmanWatcher) connect() (watchmanSocket, error) {
	sockname, err := w.getWatchmanSocket()
	if err != nil {
		return nil, fmt.Errorf("failed to get watchman socket: %w", err)
	}
	socket, err := socket.ConnectJsonSocket[[]interface{}, map[string]interface{}](sockname)
	if err != nil {
		return nil, fmt.Errorf("failed to connect to watchman socket: %w", err)
	}
	return socket, nil
}

func (w *WatchmanWatcher) recv() (map[string]interface{}, error) {
	if w.socket == nil {
		return nil, fmt.Errorf("watchman socket closed")
	}
	return w.socket.Recv()
}

func (w *WatchmanWatcher) send(args ...interface{}) error {
	if w.socket == nil {
		return fmt.Errorf("watchman socket closed")
	}
	return w.socket.Send(args)
}

// If clockspec is nil, it will return the changes since the last call to GetDiff
// If clockspec is not nil, it will return the changes since the provided clockspec
func (w *WatchmanWatcher) GetDiff(clockspec string) (*ChangeSet, error) {
	if w.socket == nil {
		return nil, fmt.Errorf("watchman socket closed")
	}
	if clockspec == "" {
		clockspec = w.lastClockSpec
	}
	if err := w.send("query", w.watchedRoot, w.makeQueryParams(clockspec)); err != nil {
		return nil, fmt.Errorf("failed to send query command: %w", err)
	}

	resp, err := w.recv()
	if err != nil {
		return nil, fmt.Errorf("failed to receive query response: %w", err)
	}

	files := make([]string, 0)

	if resp["files"] != nil {
		rf := resp["files"].([]interface{})
		files = make([]string, len(rf))
		for i, f := range rf {
			files[i] = f.(string)
		}
	}
	w.lastClockSpec = resp["clock"].(string)

	return &ChangeSet{
		Paths:     files,
		Root:      w.workspaceDir,
		ClockSpec: w.lastClockSpec,
	}, nil
}

// Connects to the watchman socket and starts watching the workspace
//
// Calling start multiple times will not start multiple watches
func (w *WatchmanWatcher) Start() error {
	if w.socket != nil {
		return nil
	}

	socket, err := w.connect()
	if err != nil {
		return err
	}
	w.socket = socket

	if err := w.send("watch-project", w.workspaceDir); err != nil {
		return fmt.Errorf("failed to send watch-project command: %w", err)
	}

	resp, err := w.recv()
	if err != nil {
		return fmt.Errorf("failed to receive watch-project response: %w", err)
	}

	if resp["error"] != nil {
		return fmt.Errorf("watch-project error response: %s", resp["error"])
	}

	w.watchedRoot = resp["watch"].(string)
	w.watchedRelPath = ""

	if resp["relative_path"] != nil {
		w.watchedRelPath = resp["relative_path"].(string)
	}

	if err := w.send("clock", w.watchedRoot); err != nil {
		return fmt.Errorf("failed to send clock command: %w", err)
	}

	resp, err = w.recv()
	if err != nil {
		return fmt.Errorf("failed to receive clock response: %w", err)
	}
	if resp["clock"] == nil {
		return fmt.Errorf("failed to get clock: %v", resp)
	}

	clock, ok := resp["clock"].(string)
	if !ok {
		return fmt.Errorf("invalid clock response: %v", clock)
	}

	w.lastClockSpec = clock

	return nil
}

// Stop watching the workspace if it was previously started, NO-OP if it was not started
//
// Do not call this function if you wish to resume watching the workspace at a later time.
//
// NOTE: This will not close any activate subscriptions, refer to the Subscribe function for that.
func (w *WatchmanWatcher) Stop() error {
	w.lastClockSpec = ""
	if err := w.send("watch-del", w.watchedRoot); err != nil {
		return fmt.Errorf("failed to send watch-del command: %w", err)
	}
	return nil
}

// This does not stop watching the workspace so next time it will resume where it left off.
//
// NOTE: This will not close any activate subscriptions, refer to the Subscribe function for that.
func (w *WatchmanWatcher) Close() error {
	if w.socket == nil {
		return nil
	}

	err := w.socket.Close()
	w.socket = nil
	return err
}

// This starts a new socket connection and starts watching the workspace
// Its important to note that ChangeSets received here will not move the
// lastClockSpec forward for the GetDiff function as this is a separate
// mechanism for receiving changes.
//
// Always the first ChangeSet will be a changeset with no changes to indicate
// the initial state of the workspace. In the future we might report current
// state of the filesystem instead of an empty changeset.
//
// When dropWithinState argument is non-empty, any change during state transition will be dropped.
// See: https://facebook.github.io/watchman/docs/cmd/subscribe#advanced-settling
func (w *WatchmanWatcher) Subscribe(ctx context.Context, dropWithinState string) iter.Seq2[*ChangeSet, error] {
	return func(yield func(*ChangeSet, error) bool) {
		if w.socket == nil {
			yield(nil, fmt.Errorf("watcher not started, call Start() first"))
			return
		}

		sock, err := w.connect()
		if err != nil {
			yield(nil, err)
			return
		}

		// Close the socket when the iterator is complete.
		defer sock.Close()

		// Close the socket when the context is done.
		if ctx != nil {
			go func() {
				<-ctx.Done()
				sock.Close()
			}()
		}

		subscriptionName := fmt.Sprintf("aspect-cli-%d.%d", os.Getpid(), w.subscriberId.Add(1))
		queryParams := w.makeQueryParams(w.lastClockSpec)
		if dropWithinState != "" {
			queryParams["drop"] = []string{dropWithinState}
		}

		err = sock.Send([]interface{}{"subscribe", w.watchedRoot, subscriptionName, queryParams})
		if err != nil {
			yield(nil, fmt.Errorf("failed to send subscribe command: %w", err))
			return
		}

		resp, err := sock.Recv()
		if err != nil {
			yield(nil, fmt.Errorf("failed to receive subscribe response: %w", err))
			return
		}

		if resp["error"] != nil {
			yield(nil, fmt.Errorf("failed to subscribe to project: %s", resp["error"]))
			return
		}

		if resp["subscribe"].(string) != subscriptionName {
			yield(nil, fmt.Errorf("wrong subscription name: %s != %s", resp["subscribe"], subscriptionName))
			return
		}

		// BEST EFFORT: if the subscriber panics, try to unsubscribe from watchman
		defer sock.Send([]interface{}{"unsubscribe", w.watchedRoot, subscriptionName})

		for {
			resp, err := sock.Recv()
			if err != nil {
				yield(nil, fmt.Errorf("failed to receive watchman response: %w", err))
				return
			}

			// This the unsubscribe PDU meaning we are done here.
			if ok := resp["unsubscribe"]; ok != nil {
				return
			}

			// This the canceled PDU meaning we are done here.
			if ok := resp["canceled"]; ok != nil {
				return
			}

			// Skip state-enter PDU
			if ok := resp["state-enter"]; ok != nil {
				continue
			}
			// Skip state-leave PDU
			if ok := resp["state-leave"]; ok != nil {
				continue
			}

			// There was an error, stop the iterator and cleanup.
			if resp["error"] != nil {
				yield(nil, fmt.Errorf("watchman error: %s", resp["error"]))
				return
			}

			files := make([]string, 0)

			if resp["files"] != nil {
				rf := resp["files"].([]interface{})
				files = make([]string, len(rf))
				for i, f := range rf {
					files[i] = f.(string)
				}
			}

			cs := ChangeSet{
				Paths:     files,
				Root:      path.Join(resp["root"].(string), w.watchedRelPath),
				ClockSpec: resp["clock"].(string),
			}
			if !yield(&cs, nil) {
				return
			}
		}
	}
}

func (w *WatchmanWatcher) StateEnter(name string) error {
	if err := w.send("state-enter", w.watchedRoot, name); err != nil {
		return fmt.Errorf("failed to send state-enter command: %w", err)
	}
	resp, err := w.recv()
	if err != nil {
		return fmt.Errorf("failed to receive state-enter command response: %w", err)
	}
	enterState, isEnterState := resp["state-enter"]
	if !isEnterState {
		return fmt.Errorf("unknown state-enter response: %v", resp)
	}
	if enterState.(string) != name {
		return fmt.Errorf("failed to state-enter: %s != %s in %v", name, enterState, resp)
	}
	return nil
}

func (w *WatchmanWatcher) StateLeave(name string) error {
	if err := w.send("state-leave", w.watchedRoot, name); err != nil {
		return fmt.Errorf("failed to send state-leave command: %w", err)
	}
	resp, err := w.recv()
	if err != nil {
		return fmt.Errorf("failed to receive state-leave command response: %w", err)
	}
	leaveState, isLeaveState := resp["state-leave"]
	if !isLeaveState {
		return fmt.Errorf("unknown state-leave response: %v", resp)
	}
	if leaveState.(string) != name {
		return fmt.Errorf("failed to state-leave: %s != %s in %v", name, leaveState, resp)
	}
	return nil
}

// NewWatchmanWatcher creates a new WatchmanWatcher
func NewWatchman(workspaceDir string) *WatchmanWatcher {
	return &WatchmanWatcher{workspaceDir: workspaceDir}
}

var _ Watcher = (*WatchmanWatcher)(nil)
