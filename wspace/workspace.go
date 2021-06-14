package wspace

import (
	"bufio"
	"errors"
	"os"
	"path"
	"strings"
	"text/template"

	"aspect.build/cli/wspace/data"
)

type WorkspaceData struct {
	Name string
}

func CreateWorkspace(root string, name string) error {
	t := template.Must(template.New("emptyWorkspace").Parse(data.MustAssetString("workspace.new.tmpl")))
	f, err := os.Create(path.Join(root, "WORKSPACE"))
	if err != nil {
		return err
	}
	defer f.Close()

	w := bufio.NewWriter(f)
	t.Execute(w, WorkspaceData{name})
	w.Flush()
	return nil
}

func Validate(input string) error {
	if strings.Contains(input, "-") {
		return errors.New("Bazel workspace names cannot contain hyphen. Try replacing with an underscore.")
	}
	return nil
}
