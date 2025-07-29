package watch

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// Workaround: https://github.com/facebook/watchman/issues/662#issuecomment-1135757635
// Watchman does not like it when the root is a symlink
func getTempDir(t *testing.T) string {
	tmp, err := filepath.EvalSymlinks("/tmp")
	if err != nil {
		t.Fatal(err)
	}
	t.Log(tmp)
	tmp, err = os.MkdirTemp(tmp, "watchman-test-")
	if err != nil {
		t.Fatal(err)
	}
	return tmp
}

func createWatchman(root string) *WatchmanWatcher {
	w := WatchmanWatcher{root: root}
	if runtime.GOOS == "darwin" {
		w.watchmanPath = "/opt/homebrew/bin/watchman"
	}
	return &w
}

func TestWatchStart(t *testing.T) {
	tmp := getTempDir(t)
	defer os.RemoveAll(tmp)

	w := createWatchman(tmp)

	err := w.Start()
	if err != nil {
		t.Errorf("Expected to start watching: %s", err)
	}
	defer w.Stop()
	defer w.Close()

	os.WriteFile(tmp+"/test", []byte("test"), 0644)

	changeset, err := w.GetDiff("")
	if err != nil {
		t.Errorf("Expected to get diff: %s", err)
	}

	if len(changeset.Paths) != 1 {
		t.Errorf("Expected to get one change")
	}

	if changeset.Paths[0] != "test" {
		t.Errorf("Expected to get test file")
	}
}

func TestSubscribe(t *testing.T) {
	tmp := getTempDir(t)
	defer os.RemoveAll(tmp)

	w := createWatchman(tmp)

	err := w.Start()
	if err != nil {
		t.Errorf("Expected to start watching: %s", err)
	}
	defer w.Stop()

	changeset := make(chan ChangeSet)
	go func() {
		err = w.Subscribe(context.TODO(), func(cs ChangeSet) error {
			t.Log(cs)
			if len(cs.Paths) == 0 {
				return nil
			}
			changeset <- cs
			return fmt.Errorf("stop")
		})

		if err != nil {
			t.Errorf("Expected to subscribe: %s", err)
		}
	}()

	os.WriteFile(tmp+"/test", []byte("test"), 0644)

	cs := <-changeset
	if len(cs.Paths) != 1 {
		t.Errorf("Expected to get one change")
	}

	if cs.Paths[0] != "test" {
		t.Errorf("Expected to get test file")
	}

}
